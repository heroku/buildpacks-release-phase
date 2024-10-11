mod errors;

use errors::ReleaseArtifactsError;
use flate2::{read::GzDecoder, Compression, GzBuilder};
use regex::Regex;
use std::{collections::HashMap, fs::File, path::Path};
use tar::Archive;

use aws_config::meta::region::RegionProviderChain;
use aws_sdk_s3::{config::Credentials, config::Region, Client};
use url::Url;

use tokio as _;
use uuid as _;

pub async fn upload<S: ::std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
    dir: &Path,
) -> Result<(), ReleaseArtifactsError> {
    guard(env)?;

    let archive_name = format!("{}.tgz", env["RELEASE_ID"]);
    create_archive(dir, &archive_name)?;

    let (bucket_name, bucket_region, bucket_path) = parse_s3_url(&env["STATIC_ARTIFACTS_URL"])?;
    let bucket_key =
        bucket_path.map_or_else(|| archive_name.clone(), |p| format!("{p}/{archive_name}"));

    let credentials = Credentials::new(
        env["STATIC_ARTIFACTS_AWS_ACCESS_KEY_ID"].clone(),
        env["STATIC_ARTIFACTS_AWS_SECRET_ACCESS_KEY"].clone(),
        None,
        None,
        "Static Artifacts storage",
    );
    let region_provider = RegionProviderChain::first_try(bucket_region.map(Region::new))
        .or_else(Region::new("us-east-1"));
    let shared_config = aws_config::from_env()
        .region(region_provider)
        .credentials_provider(credentials)
        .load()
        .await;
    let s3 = Client::new(&shared_config);

    upload_with_client(s3, bucket_name, bucket_key, archive_name).await
}

pub async fn upload_with_client(
    s3: aws_sdk_s3::Client,
    bucket_name: String,
    bucket_key: String,
    archive_name: String,
) -> Result<(), ReleaseArtifactsError> {
    let archive_data =
        aws_sdk_s3::primitives::ByteStream::from_path(std::path::Path::new(&archive_name))
            .await
            .map_err(ReleaseArtifactsError::ArchiveStreamError)?;
    s3.put_object()
        .bucket(bucket_name)
        .key(bucket_key)
        .body(archive_data)
        .send()
        .await
        .map_err(ReleaseArtifactsError::from)?;
    Ok(())
}

pub fn guard<S: ::std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<(), ReleaseArtifactsError> {
    let mut messages: Vec<String> = vec![];
    if !env.contains_key("RELEASE_ID") {
        messages.push("RELEASE_ID is required".to_string());
    }
    if !env.contains_key("STATIC_ARTIFACTS_AWS_ACCESS_KEY_ID") {
        messages.push("STATIC_ARTIFACTS_AWS_ACCESS_KEY_ID is required".to_string());
    }
    if !env.contains_key("STATIC_ARTIFACTS_AWS_SECRET_ACCESS_KEY") {
        messages.push("STATIC_ARTIFACTS_AWS_SECRET_ACCESS_KEY is required".to_string());
    }
    if !env.contains_key("STATIC_ARTIFACTS_URL") {
        messages.push("STATIC_ARTIFACTS_URL is required".to_string());
    }
    if !messages.is_empty() {
        return Err(ReleaseArtifactsError::ConfigMissing(messages.join(". ")));
    }
    Ok(())
}

pub fn parse_s3_url(
    url: &str,
) -> Result<(String, Option<String>, Option<String>), ReleaseArtifactsError> {
    let bucket_name: String;
    let mut bucket_region: Option<String> = None;
    let s3_url = Url::parse(url).map_err(ReleaseArtifactsError::StorageURLInvalid)?;
    let s3_host_regex =
        Regex::new(r"([^\.]+).s3.([^\.]+).amazonaws.com$").expect("regex should compile");
    match s3_url.host() {
        Some(host) => match host {
            url::Host::Domain(name) => match s3_host_regex.captures(name) {
                Some(name_parts) => {
                    bucket_name = name_parts[1].to_string();
                    bucket_region = Some(name_parts[2].to_string());
                }
                None => bucket_name = name.to_string(),
            },
            url::Host::Ipv4(addr) => {
                bucket_name = addr.to_string();
            }
            url::Host::Ipv6(addr) => {
                bucket_name = addr.to_string();
            }
        },
        None => return Err(ReleaseArtifactsError::StorageURLHostMissing),
    }
    let bucket_path = if s3_url.path().is_empty() {
        None
    } else {
        Some(s3_url.path().to_string())
    };
    Ok((bucket_name, bucket_region, bucket_path))
}

/// Tars & compresses contents of the given directory to a .tar.gz file.
pub fn create_archive(
    source_dir: &Path,
    destination: impl AsRef<Path>,
) -> Result<(), ReleaseArtifactsError> {
    let output_file: File =
        File::create(destination).map_err(ReleaseArtifactsError::ArchiveError)?;
    let gz = GzBuilder::new().write(output_file, Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.follow_symlinks(false);
    // add to root of archive
    tar.append_dir_all("", source_dir)
        .map_err(ReleaseArtifactsError::ArchiveError)?;
    Ok(())
}

/// Decompresses and untars a given .tar.gz file to the given directory.
pub fn extract_archive(
    source_file: &Path,
    destination: impl AsRef<Path>,
) -> Result<(), ReleaseArtifactsError> {
    let source = File::open(source_file).map_err(ReleaseArtifactsError::ArchiveError)?;
    let mut archive = Archive::new(GzDecoder::new(source));
    archive
        .unpack(destination)
        .map_err(ReleaseArtifactsError::ArchiveError)
}

#[allow(dead_code)]
fn make_s3_test_credentials() -> aws_sdk_s3::config::Credentials {
    aws_sdk_s3::config::Credentials::new(
        "ATESTCLIENT",
        "astestsecretkey",
        Some("atestsessiontoken".to_string()),
        None,
        "",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs::{self, File},
        path::Path,
    };

    use aws_config::BehaviorVersion;
    use flate2::read::GzDecoder;
    use tar::Archive;
    use uuid::Uuid;

    use aws_smithy_runtime::client::http::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;

    use crate::{
        create_archive, errors::ReleaseArtifactsError, extract_archive, guard,
        make_s3_test_credentials, parse_s3_url, upload_with_client,
    };

    #[tokio::test]
    async fn upload_with_client_succeeds() {
        let page_1 = ReplayEvent::new(
            http::Request::builder()
                .method("PUT")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/sub/path/static-artifacts.tgz?x-id=PutObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::empty())
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![page_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = upload_with_client(
            s3,
            "test-bucket".to_string(),
            "sub/path/static-artifacts.tgz".to_string(),
            "test/fixtures/static-artifacts.tgz".to_string(),
        )
        .await;

        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
    }

    #[test]
    fn guard_should_pass_with_required_env() {
        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_AWS_ACCESS_KEY_ID".to_string(),
            "test-key".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_AWS_SECRET_ACCESS_KEY".to_string(),
            "test-secret".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://test-bucket.s3.us-west-2.amazonaws.com".to_string(),
        );

        let result = guard(&test_env);
        assert!(result.is_ok());
    }

    #[test]
    fn guard_should_fail_missing_requirements() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_AWS_ACCESS_KEY_ID".to_string(),
            "test-key".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_AWS_SECRET_ACCESS_KEY".to_string(),
            "test-secret".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://test-bucket.s3.us-west-2.amazonaws.com".to_string(),
        );

        let result = guard(&test_env);
        assert!(result.is_err());

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_AWS_SECRET_ACCESS_KEY".to_string(),
            "test-secret".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://test-bucket.s3.us-west-2.amazonaws.com".to_string(),
        );

        let result = guard(&test_env);
        assert!(result.is_err());

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_AWS_ACCESS_KEY_ID".to_string(),
            "test-key".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://test-bucket.s3.us-west-2.amazonaws.com".to_string(),
        );

        let result = guard(&test_env);
        assert!(result.is_err());

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_AWS_ACCESS_KEY_ID".to_string(),
            "test-key".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_AWS_SECRET_ACCESS_KEY".to_string(),
            "test-secret".to_string(),
        );

        let result = guard(&test_env);
        assert!(result.is_err());
    }

    #[test]
    fn parse_s3_url_returns_parts() {
        let (bucket_name, bucket_region, bucket_path) =
            parse_s3_url("s3://test-bucket.s3.us-west-2.amazonaws.com/sub/path")
                .expect("should parse the URL");
        assert_eq!(bucket_name, "test-bucket".to_string());
        assert_eq!(bucket_region, Some("us-west-2".to_string()));
        assert_eq!(bucket_path, Some("/sub/path".to_string()));

        let (bucket_name, bucket_region, bucket_path) =
            parse_s3_url("s3://test-bare-name/sub/path").expect("should parse the URL");
        assert_eq!(bucket_name, "test-bare-name".to_string());
        assert_eq!(bucket_region, None);
        assert_eq!(bucket_path, Some("/sub/path".to_string()));

        let (bucket_name, bucket_region, bucket_path) =
            parse_s3_url("s3://test-bucket.s3.us-west-2.amazonaws.com")
                .expect("should parse the URL");
        assert_eq!(bucket_name, "test-bucket".to_string());
        assert_eq!(bucket_region, Some("us-west-2".to_string()));
        assert_eq!(bucket_path, None);
    }

    #[test]
    fn parse_s3_url_fail_when_invalid() {
        let error = parse_s3_url("test-bucket.s3.us-west-2.amazonaws.com/sub/path")
            .expect_err("should not parse the URL");
        assert!(matches!(error, ReleaseArtifactsError::StorageURLInvalid(_)));

        let error = parse_s3_url("s3:///sub/path").expect_err("should not parse the URL");
        assert!(matches!(
            error,
            ReleaseArtifactsError::StorageURLHostMissing
        ));
    }

    #[test]
    fn create_archive_should_output_tar_gz_file() {
        let unique = Uuid::new_v4();
        let output_file = format!("artifact-from-test-succeeds-{unique}.tgz");
        let output_dir = format!("artifact-from-test-{unique}");
        let output_path = Path::new(&output_dir);
        fs::remove_file(&output_file).unwrap_or_default();
        fs::remove_dir_all(output_path).unwrap_or_default();

        create_archive(Path::new("test/fixtures/static-artifacts"), &output_file).unwrap();
        let result_metadata = fs::metadata(&output_file).unwrap();
        assert!(result_metadata.is_file());
        let output = File::open(&output_file).unwrap();
        let mut archive = Archive::new(GzDecoder::new(&output));
        archive.unpack(output_path).unwrap();
        let result_metadata = fs::metadata(output_path.join("index.html")).unwrap();
        assert!(result_metadata.is_file());
        let result_metadata =
            fs::metadata(output_path.join("images/desktop-heroku-pride.jpg")).unwrap();
        assert!(result_metadata.is_file());
        fs::remove_file(&output_file).unwrap_or_default();
        fs::remove_dir_all(output_path).unwrap_or_default();
    }

    #[test]
    fn create_archive_should_fail_for_missing_source_dir() {
        let unique = Uuid::new_v4();
        let output_dir = format!("artifact-from-test-{unique}");
        let output_path = Path::new(&output_dir);
        fs::remove_file(output_path).unwrap_or_default();

        create_archive(Path::new("non-existent-path"), output_path)
            .expect_err("should fail for missing source dir");
        fs::remove_file(output_path).unwrap_or_default();
    }

    #[test]
    fn extract_archive_should_output_a_directory() {
        let unique = Uuid::new_v4();
        let output_dir = format!("artifact-from-test-{unique}");
        let output_path = Path::new(&output_dir);
        fs::remove_dir_all(output_path).unwrap_or_default();

        extract_archive(Path::new("test/fixtures/static-artifacts.tgz"), output_path).unwrap();
        let result_metadata = fs::metadata(output_path).unwrap();
        assert!(result_metadata.is_dir());
        let result_metadata = fs::metadata(output_path.join("index.html")).unwrap();
        assert!(result_metadata.is_file());
        let result_metadata =
            fs::metadata(output_path.join("images/desktop-heroku-pride.jpg")).unwrap();
        assert!(result_metadata.is_file());
        fs::remove_dir_all(output_path).unwrap_or_default();
    }

    #[test]
    fn extract_archive_should_fail_for_missing_source_file() {
        let unique = Uuid::new_v4();
        let output_dir = format!("artifact-from-test-{unique}");
        let output_path = Path::new(&output_dir);
        fs::remove_dir_all(output_path).unwrap_or_default();

        extract_archive(Path::new("non-existent-path"), output_path)
            .expect_err("should fail for missing source file");
        fs::remove_dir_all(output_path).unwrap_or_default();
    }
}
