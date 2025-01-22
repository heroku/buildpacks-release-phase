mod errors;

use aws_smithy_types::DateTime;
use errors::ReleaseArtifactsError;
use flate2::{read::GzDecoder, Compression, GzBuilder};
use regex::Regex;
use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    hash::BuildHasher,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use tar::Archive;

use aws_config::meta::region::RegionProviderChain;
use aws_sdk_s3::{
    config::{Credentials, Region},
    types::Object,
    Client,
};
use url::Url;

use tokio as _;
use uuid::{self as _, Uuid};

#[must_use]
pub fn capture_env(dyno_metadata_dir: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();
    for (key, value) in env::vars() {
        if key.starts_with("STATIC_ARTIFACTS_") || key == "RELEASE_ID" {
            env.insert(key, value);
        }
    }
    // Override RELEASE_ID with value from the dyno filesystem, when present.
    File::open(dyno_metadata_dir.join("release_id"))
        .map_or(None, |mut file| {
            let mut buffer = String::new();
            if file.read_to_string(&mut buffer).is_ok() {
                buffer = buffer.trim().to_string();
                return Some(buffer);
            }
            None
        })
        .map(|dyno_release_id| env.insert("RELEASE_ID".to_owned(), dyno_release_id));
    env
}

pub async fn save<S: BuildHasher>(
    env: &HashMap<String, String, S>,
    dir: &Path,
) -> Result<(), ReleaseArtifactsError> {
    match detect_storage_scheme(env) {
        Ok(scheme) if scheme == *"file" => {
            guard_file(env)?;
            let archive_name = generate_archive_name::<S>(env);
            eprintln!("save-release-artifacts writing archive: {archive_name}");
            let destination_path = generate_file_storage_location(env, &archive_name)?;
            create_archive(dir, &destination_path)?;
            Ok(())
        }
        Ok(scheme) if scheme == *"s3" => {
            guard_s3(env)?;
            let archive_name = generate_archive_name::<S>(env);
            eprintln!("save-release-artifacts uploading archive: {archive_name}");
            create_archive(dir, Path::new(archive_name.as_str()))?;
            let (bucket_name, bucket_region, bucket_key) =
                generate_s3_storage_location(env, &archive_name)?;
            let s3 = generate_s3_client(env, bucket_region).await;
            upload_with_client(&s3, &bucket_name, &bucket_key, &archive_name).await
        }
        Ok(scheme) => Err(ReleaseArtifactsError::StorageURLUnsupportedScheme(scheme)),
        Err(e) => Err(e),
    }
}

pub async fn load<S: BuildHasher>(
    env: &HashMap<String, String, S>,
    dir: &Path,
) -> Result<String, ReleaseArtifactsError> {
    if !env.contains_key("STATIC_ARTIFACTS_URL") {
        return Err(ReleaseArtifactsError::ConfigMissing(
            "STATIC_ARTIFACTS_URL is required".to_string(),
        ));
    }
    match detect_storage_scheme(env) {
        Ok(scheme) if scheme == *"file" => {
            let archive_name = generate_archive_name::<S>(env);
            eprintln!("load-release-artifacts reading archive: {archive_name}");
            // This file scheme does not currently find latest if the specific release ID is missing.
            let source_path = generate_file_storage_location(env, &archive_name)?;
            extract_archive(&source_path, dir)?;
            Ok(archive_name.to_string())
        }
        Ok(scheme) if scheme == *"s3" => {
            guard_s3(env)?;
            let archive_name = generate_archive_name::<S>(env);
            eprintln!("load-release-artifacts downloading archive: {archive_name}");
            let (bucket_name, bucket_region, bucket_key) =
                generate_s3_storage_location(env, &archive_name)?;
            let s3 = generate_s3_client(env, bucket_region).await;
            download_specific_or_latest_with_client(&s3, &bucket_name, &bucket_key, dir).await
        }
        Ok(scheme) => Err(ReleaseArtifactsError::StorageURLUnsupportedScheme(scheme)),
        Err(e) => Err(e),
    }
}

#[allow(clippy::unused_async)]
pub async fn gc<S: BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<(), ReleaseArtifactsError> {
    if !env.contains_key("STATIC_ARTIFACTS_URL") {
        return Err(ReleaseArtifactsError::ConfigMissing(
            "STATIC_ARTIFACTS_URL is required".to_string(),
        ));
    }
    match detect_storage_scheme(env) {
        Ok(scheme) if scheme == *"file" => gc_file(env),
        Ok(scheme) if scheme == *"s3" => gc_s3(env).await,
        Ok(scheme) => Err(ReleaseArtifactsError::StorageURLUnsupportedScheme(scheme)),
        Err(e) => Err(e),
    }
}

async fn gc_s3<S: BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<(), ReleaseArtifactsError> {
    guard_s3(env)?;
    let (bucket_name, bucket_region_from_url, bucket_path) =
        parse_s3_url(&env["STATIC_ARTIFACTS_URL"])?;
    eprintln!("gc-release-artifacts listing s3 archives : {bucket_name}");
    let bucket_region =
        bucket_region_from_url.or_else(|| env.get("STATIC_ARTIFACTS_REGION").cloned());
    let s3 = generate_s3_client(env, bucket_region).await;

    let mut objects = list_bucket_objects_with_client(&s3, &bucket_name).await?;
    // TODO handle date parsing error
    objects.sort_by_key(|s| s.last_modified.unwrap());

    let older_than_latest_two = objects[2..].to_vec();
    for object in older_than_latest_two {
        delete_object_with_client(&s3, &bucket_name, &object.key.unwrap()).await?;
    }

    // fn delete_s3_archive (archive)
    //
    // for archive in filtered {
    //   match delete_s3_archive() {
    //      Ok(_) => Ok()
    //      Err(err) => return GcS3Err(err)
    //   }
    // }
    //
    // Ok(())
    Ok(())
}

fn gc_file<S: BuildHasher>(env: &HashMap<String, String, S>) -> Result<(), ReleaseArtifactsError> {
    // We do not run `guard_file` here because we do not care about RELEASE_ID
    let parsed_url = Url::parse(&env["STATIC_ARTIFACTS_URL"])
        .map_err(ReleaseArtifactsError::StorageURLInvalid)?;

    let entries = sorted_dir_entries(parsed_url.path())?;
    if entries.len() >= 2 {
        for filename in entries[2..].iter() {
            let filepath = Path::new(parsed_url.path()).join(filename);
            fs::remove_file(filepath).map_err(|e| {
                ReleaseArtifactsError::ArchiveError(
                    e,
                    format!("Could not remove file {filename} during artifact garbage collection."),
                )
            })?
        }
    }
    Ok(())
}

fn sorted_dir_entries(path: &str) -> Result<Vec<String>, ReleaseArtifactsError> {
    let entries = fs::read_dir(path).map_err(|e| {
        ReleaseArtifactsError::ArchiveError(
            e,
            format!("Could not read directory {path} when reading directory entries."),
        )
    })?;

    let mut entries_with_mod_time: Vec<(String, SystemTime)> = vec![];
    for entry in entries.flatten() {
        // TODO cleanup
        if let Ok(metadata) = entry.metadata() {
            if let Ok(filename) = entry.file_name().into_string() {
                let ext = Path::new(filename.as_str()).extension();
                let has_correct_ext = ext.is_some_and(|e| e == "tgz");
                if metadata.is_file() && has_correct_ext {
                    if let Ok(modified) = metadata.modified() {
                        entries_with_mod_time.append(vec![(filename.clone(), modified)].as_mut());
                    };
                }
            }
        }
    }

    entries_with_mod_time.sort_by(|a, b| b.1.cmp(&a.1));

    let result = entries_with_mod_time
        .iter()
        .map(|tup| tup.0.clone())
        .collect();
    Ok(result)
}

pub async fn upload_with_client(
    s3: &aws_sdk_s3::Client,
    bucket_name: &String,
    bucket_key: &String,
    archive_name: &String,
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

pub async fn download_specific_or_latest_with_client(
    s3: &aws_sdk_s3::Client,
    bucket_name: &String,
    bucket_key: &String,
    destination_dir: &Path,
) -> Result<String, ReleaseArtifactsError> {
    match download_with_client(s3, bucket_name, bucket_key, destination_dir).await {
        Ok(()) => Ok(bucket_key.clone()),
        Err(e) => match e {
            ReleaseArtifactsError::StorageKeyNotFound(_) => {
                eprintln!("load-release-artifacts specific artifact not found '{bucket_key}', instead getting latest artifact");
                let key_parts = bucket_key.split('/');
                let key_prefix_size = key_parts.clone().count() - 1;
                let key_prefix_parts: Vec<&str> = key_parts.clone().take(key_prefix_size).collect();
                let key_prefix = if key_prefix_parts.is_empty() {
                    String::new()
                } else {
                    key_prefix_parts.join("/") + "/"
                };
                let latest_result = find_latest_with_client(s3, bucket_name, &key_prefix)
                    .await
                    .map_err(ReleaseArtifactsError::from)?;
                match latest_result {
                    Some(latest_bucket_key) => {
                        eprintln!(
                            "load-release-artifacts getting latest artifact '{latest_bucket_key}'"
                        );
                        download_with_client(s3, bucket_name, &latest_bucket_key, destination_dir)
                            .await?;
                        Ok(latest_bucket_key.clone())
                    }
                    None => Err(ReleaseArtifactsError::StorageKeyNotFound(format!(
                        "Nothing found in bucket '{bucket_name}' prefix '{key_prefix}'"
                    ))),
                }
            }
            _ => Err(e),
        },
    }
}

pub async fn list_bucket_objects_with_client(
    s3: &aws_sdk_s3::Client,
    bucket_name: &String,
) -> Result<Vec<Object>, ReleaseArtifactsError> {
    let response = s3
        .list_objects_v2()
        .bucket(bucket_name)
        .send()
        .await
        .map_err(ReleaseArtifactsError::from)?;
    // TODO handle error
    Ok(response.contents.unwrap())
}

pub async fn delete_object_with_client(
    s3: &aws_sdk_s3::Client,
    bucket_name: &String,
    key: &String,
) -> Result<bool, ReleaseArtifactsError> {
    let response = s3
        .delete_object()
        .bucket(bucket_name)
        .key(key)
        .send()
        .await
        .map_err(ReleaseArtifactsError::from)?;
    // TODO handle response.delete_marker being false
    //   maybe not worth bubbling up
    // if response.delete_marker.is_some_and(|s| !s) {
    //     todo()
    // }
    Ok(response.delete_marker.unwrap())
}

pub async fn download_with_client(
    s3: &aws_sdk_s3::Client,
    bucket_name: &String,
    bucket_key: &String,
    destination_dir: &Path,
) -> Result<(), ReleaseArtifactsError> {
    let mut output = s3
        .get_object()
        .bucket(bucket_name)
        .key(bucket_key)
        .send()
        .await
        .map_err(ReleaseArtifactsError::from)?;

    let unique = Uuid::new_v4();
    let temp_archive_name = format!(
        "static-artifacts-temp--{}--{}",
        bucket_key.replace('/', "-"),
        unique
    );
    let temp_archive_path = Path::new(&temp_archive_name);

    let mut archive = File::create(temp_archive_path).map_err(|e| {
        ReleaseArtifactsError::ArchiveError(
            e,
            format!("during download_with_client File::create({temp_archive_path:?})"),
        )
    })?;

    let mut byte_count = 0_usize;
    while let Some(bytes) = output
        .body
        .try_next()
        .await
        .map_err(ReleaseArtifactsError::ArchiveStreamError)?
    {
        let bytes_len = bytes.len();
        archive.write_all(&bytes).map_err(|e| {
            ReleaseArtifactsError::ArchiveError(
                e,
                "during download_with_client archive.write_all".to_string(),
            )
        })?;
        byte_count += bytes_len;
    }
    eprintln!("load-release-artifacts received {byte_count}-bytes");

    extract_archive(temp_archive_path, destination_dir)?;
    fs::remove_file(temp_archive_path).map_err(|e| {
        ReleaseArtifactsError::ArchiveError(
            e,
            format!("during download_with_client fs::remove_file({temp_archive_path:?})"),
        )
    })?;

    Ok(())
}

pub async fn find_latest_with_client(
    s3: &aws_sdk_s3::Client,
    bucket_name: &String,
    bucket_key_prefix: &String,
) -> Result<Option<String>, ReleaseArtifactsError> {
    let output = s3
        .list_objects_v2()
        .bucket(bucket_name)
        .prefix(bucket_key_prefix)
        .send()
        .await
        .map_err(ReleaseArtifactsError::from)?;
    let latest_key = output.contents.and_then(|mut c| {
        if c.is_empty() {
            return None;
        }
        c.sort_by_key(|k| {
            k.last_modified()
                .map_or_else(|| DateTime::from_secs(0), std::borrow::ToOwned::to_owned)
        });
        c.last()
            .expect("should have at least one sorted object")
            .key()
            .map(std::string::ToString::to_string)
    });
    Ok(latest_key)
}

fn detect_storage_scheme<S: BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<String, ReleaseArtifactsError> {
    match env.get("STATIC_ARTIFACTS_URL") {
        Some(url) => {
            let result = Url::parse(url).map_err(ReleaseArtifactsError::StorageURLInvalid)?;
            Ok(result.scheme().to_string())
        }
        None => Err(ReleaseArtifactsError::StorageURLMissing),
    }
}

fn guard_s3<S: ::std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<(), ReleaseArtifactsError> {
    let mut messages: Vec<String> = vec![];
    if !env.contains_key("RELEASE_ID") {
        messages.push("RELEASE_ID is required".to_string());
    }
    if !env.contains_key("STATIC_ARTIFACTS_ACCESS_KEY_ID") {
        messages.push("STATIC_ARTIFACTS_ACCESS_KEY_ID is required".to_string());
    }
    if !env.contains_key("STATIC_ARTIFACTS_SECRET_ACCESS_KEY") {
        messages.push("STATIC_ARTIFACTS_SECRET_ACCESS_KEY is required".to_string());
    }
    if !env.contains_key("STATIC_ARTIFACTS_URL") {
        messages.push("STATIC_ARTIFACTS_URL is required".to_string());
    }
    if !messages.is_empty() {
        return Err(ReleaseArtifactsError::ConfigMissing(messages.join(". ")));
    }
    Ok(())
}

fn guard_file<S: ::std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<(), ReleaseArtifactsError> {
    let mut messages: Vec<String> = vec![];
    if !env.contains_key("RELEASE_ID") {
        messages.push("RELEASE_ID is required".to_string());
    }
    if !env.contains_key("STATIC_ARTIFACTS_URL") {
        messages.push("STATIC_ARTIFACTS_URL is required".to_string());
    }
    if !messages.is_empty() {
        return Err(ReleaseArtifactsError::ConfigMissing(messages.join(". ")));
    }
    Ok(())
}

fn generate_archive_name<S: BuildHasher>(env: &HashMap<String, String, S>) -> String {
    let release_id = env
        .get("RELEASE_ID")
        .map_or(String::default(), std::borrow::ToOwned::to_owned);
    if release_id.is_empty() {
        let unique = Uuid::new_v4();
        format!("artifact-{unique}.tgz")
    } else {
        format!("release-{release_id}.tgz")
    }
}

fn generate_s3_storage_location<S: BuildHasher>(
    env: &HashMap<String, String, S>,
    archive_name: &String,
) -> Result<(String, Option<String>, String), ReleaseArtifactsError> {
    let (bucket_name, bucket_region_from_url, bucket_path) =
        parse_s3_url(&env["STATIC_ARTIFACTS_URL"])?;
    let bucket_region =
        bucket_region_from_url.or_else(|| env.get("STATIC_ARTIFACTS_REGION").cloned());
    let bucket_key =
        bucket_path.map_or_else(|| archive_name.clone(), |p| format!("{p}/{archive_name}"));
    Ok((bucket_name, bucket_region, bucket_key))
}

fn generate_file_storage_location<S: BuildHasher>(
    env: &HashMap<String, String, S>,
    archive_name: &String,
) -> Result<PathBuf, ReleaseArtifactsError> {
    let url = Url::parse(&env["STATIC_ARTIFACTS_URL"])
        .map_err(ReleaseArtifactsError::StorageURLInvalid)?;
    let dest_path = url.path();
    fs::create_dir_all(dest_path).map_err(|e| {
        ReleaseArtifactsError::ArchiveError(
            e,
            format!("creating filesystem destination directory '{dest_path}'"),
        )
    })?;
    let result = Path::new(dest_path).join(archive_name);
    Ok(result.clone())
}

async fn generate_s3_client<S: BuildHasher>(
    env: &HashMap<String, String, S>,
    bucket_region: Option<String>,
) -> Client {
    let credentials = Credentials::new(
        env["STATIC_ARTIFACTS_ACCESS_KEY_ID"].clone(),
        env["STATIC_ARTIFACTS_SECRET_ACCESS_KEY"].clone(),
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
    Client::new(&shared_config)
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
        None => {
            return Err(ReleaseArtifactsError::StorageURLHostMissing(
                "S3 URL is missing host".to_string(),
            ))
        }
    }
    let bucket_path = if s3_url.path().is_empty() {
        None
    } else {
        Some(
            s3_url
                .path()
                .trim_start_matches('/')
                .trim_end_matches('/')
                .to_string(),
        )
    };
    Ok((bucket_name, bucket_region, bucket_path))
}

/// Tars & compresses contents of the given directory to a .tar.gz file.
pub fn create_archive(source_dir: &Path, destination: &Path) -> Result<(), ReleaseArtifactsError> {
    let output_file: File = File::create(destination).map_err(|e| {
        ReleaseArtifactsError::ArchiveError(
            e,
            format!("during create_archive File::create({destination:?})"),
        )
    })?;
    let gz = GzBuilder::new().write(output_file, Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.follow_symlinks(false);
    // add to root of archive
    tar.append_dir_all("", source_dir).map_err(|e| {
        ReleaseArtifactsError::ArchiveError(
            e,
            format!("during create_archive tar.append_dir_all({source_dir:?})"),
        )
    })?;
    tar.finish().map_err(|e| {
        ReleaseArtifactsError::ArchiveError(e, "during create_archive tar.finish()".to_string())
    })
}

/// Decompresses and untars a given .tar.gz file to the given directory.
pub fn extract_archive(
    source_file: &Path,
    destination: &Path,
) -> Result<(), ReleaseArtifactsError> {
    let source = File::open(source_file).map_err(|e| {
        ReleaseArtifactsError::ArchiveError(
            e,
            format!("during extract_archive File::open({source_file:?})"),
        )
    })?;
    let mut archive = Archive::new(GzDecoder::new(source));
    archive.unpack(destination).map_err(|e| {
        ReleaseArtifactsError::ArchiveError(
            e,
            format!("during extract_archive archive.unpack({destination:?})"),
        )
    })
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
        env,
        fs::{self, File},
        io::{Read, Write},
        path::Path,
        time::{Duration, SystemTime},
    };

    use aws_config::BehaviorVersion;
    use flate2::read::GzDecoder;
    use tar::Archive;
    use uuid::Uuid;

    use aws_smithy_runtime::client::http::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;

    use crate::{
        capture_env, create_archive, detect_storage_scheme,
        download_specific_or_latest_with_client, download_with_client,
        errors::ReleaseArtifactsError, extract_archive, find_latest_with_client, gc,
        generate_archive_name, generate_file_storage_location, generate_s3_client,
        generate_s3_storage_location, guard_file, guard_s3, load, make_s3_test_credentials,
        parse_s3_url, save, sorted_dir_entries, upload_with_client,
    };

    #[test]
    fn capture_env_succeeds() {
        env::set_var("RELEASE_ID", "test-release-id");
        let result = capture_env(Path::new("does-not-exist"));
        env::remove_var("RELEASE_ID");
        assert_eq!(
            result.get("RELEASE_ID"),
            Some(&"test-release-id".to_string())
        );
    }

    #[test]
    fn capture_env_with_metadata_file_succeeds() {
        let unique = Uuid::new_v4();
        let dyno_metadata_dir = format!("dyno-metadata-for-test-{unique}");
        let dyno_metadata_path = Path::new(&dyno_metadata_dir);
        fs::create_dir_all(dyno_metadata_path).expect("dyno metadata dir should be created");
        let release_id_path = dyno_metadata_path.join("release_id");
        File::create(&release_id_path)
            .and_then(|mut file| file.write_all(b"test-release-id-from-file"))
            .expect("dyno metadata file shoud be written");

        env::set_var("RELEASE_ID", "test-release-id-from-env");
        let result = capture_env(dyno_metadata_path);
        env::remove_var("RELEASE_ID");
        assert_eq!(
            result.get("RELEASE_ID"),
            Some(&"test-release-id-from-file".to_string())
        );
        fs::remove_dir_all(dyno_metadata_path).unwrap_or_default();
    }

    #[tokio::test]
    async fn save_file_url_succeeds() {
        let unique = Uuid::new_v4();
        let output_archive_dir = format!("test-saved-static-artifacts-{unique}");
        let abs_root = env::current_dir().expect("should have a current working directory");
        let output_archive_dir_path = Path::new(&abs_root).join(output_archive_dir.as_str());
        fs::remove_dir_all(&output_archive_dir_path).unwrap_or_default();

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), unique.to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            format!("file://{}", output_archive_dir_path.to_string_lossy()),
        );

        let result = save(&test_env, Path::new("test/fixtures/static-artifacts")).await;

        eprintln!("{result:?}");
        assert!(result.is_ok());
        eprintln!("{:#?}", fs::metadata(&output_archive_dir_path));
        assert!(fs::metadata(&output_archive_dir_path).is_ok());
        assert!(
            fs::metadata(output_archive_dir_path.join(format!("release-{unique}.tgz"))).is_ok()
        );
        fs::remove_dir_all(output_archive_dir_path).expect("temporary directory should be deleted");
    }

    #[tokio::test]
    async fn upload_with_client_succeeds() {
        let put_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("PUT")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/sub/path/static-artifacts.tgz?x-id=PutObject")
                .body(SdkBody::empty()) // body must be empty here, because it uses a streamer impl
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::empty())
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![put_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = upload_with_client(
            &s3,
            &"test-bucket".to_string(),
            &"sub/path/static-artifacts.tgz".to_string(),
            &"test/fixtures/static-artifacts.tgz".to_string(),
        )
        .await;

        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
    }

    #[tokio::test]
    async fn load_file_url_succeeds() {
        let unique = Uuid::new_v4();
        let abs_root = env::current_dir().expect("should have a current working directory");
        let source_archive_dir_path = Path::new(&abs_root).join("test/fixtures");
        let destination_dir_path =
            Path::new(&abs_root).join(format!("static-artifacts-test-{unique}"));

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "xxxxx".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            format!("file://{}", source_archive_dir_path.to_string_lossy()).to_string(),
        );

        let result = load(&test_env, &destination_dir_path).await;

        eprintln!("{result:?}");
        assert!(result.is_ok());
        assert!(fs::metadata(&destination_dir_path).is_ok());
        assert!(fs::metadata(destination_dir_path.join("index.html")).is_ok());
        assert!(fs::metadata(destination_dir_path.join("images")).is_ok());
        assert!(fs::metadata(destination_dir_path.join("images/desktop-heroku-pride.jpg")).is_ok());
        fs::remove_dir_all(destination_dir_path).expect("temporary directory should be deleted");
    }

    #[tokio::test]
    async fn download_specific_or_latest_with_client_specific_succeeds() {
        let unique = Uuid::new_v4();
        let output_dir_name = format!("test-output-static-artifacts-{unique}");
        let output_dir = Path::new(output_dir_name.as_str());
        fs::remove_dir_all(output_dir).unwrap_or_default();

        let get_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/sub/path/static-artifacts.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(read_fixture_archive_data()))
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![get_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = download_specific_or_latest_with_client(
            &s3,
            &"test-bucket".to_string(),
            &"sub/path/static-artifacts.tgz".to_string(),
            output_dir,
        )
        .await;

        println!("{result:#?}");
        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
        assert!(fs::metadata(output_dir).is_ok());
        assert!(fs::metadata(output_dir.join("index.html")).is_ok());
        assert!(fs::metadata(output_dir.join("images")).is_ok());
        assert!(fs::metadata(output_dir.join("images/desktop-heroku-pride.jpg")).is_ok());
        fs::remove_dir_all(output_dir).expect("temporary directory should be deleted");
    }

    #[tokio::test]
    async fn download_specific_or_latest_with_client_specific_no_prefix_succeeds() {
        let unique = Uuid::new_v4();
        let output_dir_name = format!("test-output-static-artifacts-{unique}");
        let output_dir = Path::new(output_dir_name.as_str());
        fs::remove_dir_all(output_dir).unwrap_or_default();

        let get_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/static-artifacts.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(read_fixture_archive_data()))
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![get_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = download_specific_or_latest_with_client(
            &s3,
            &"test-bucket".to_string(),
            &"static-artifacts.tgz".to_string(),
            output_dir,
        )
        .await;

        println!("{result:#?}");
        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
        assert!(fs::metadata(output_dir).is_ok());
        assert!(fs::metadata(output_dir.join("index.html")).is_ok());
        assert!(fs::metadata(output_dir.join("images")).is_ok());
        assert!(fs::metadata(output_dir.join("images/desktop-heroku-pride.jpg")).is_ok());
        fs::remove_dir_all(output_dir).expect("temporary directory should be deleted");
    }

    #[tokio::test]
    async fn download_specific_or_latest_with_client_latest_succeeds() {
        let unique = Uuid::new_v4();
        let output_dir_name = format!("test-output-static-artifacts-{unique}");
        let output_dir = Path::new(output_dir_name.as_str());
        fs::remove_dir_all(output_dir).unwrap_or_default();

        let get_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/sub/path/static-artifacts.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(404)
                .body(SdkBody::from(r"
                    <Error>
                        <Code>NoSuchKey</Code>
                    </Error>",
                ))
                .unwrap(),
        );
        let list_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/?list-type=2&prefix=sub%2Fpath%2F")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(r"
                    <ListBucketResult>
                        <IsTruncated>false</IsTruncated>
                        <Contents>
                            <Key>sub/path/v100.tgz</Key>
                            <LastModified>2024-07-01T12:20:47.000Z</LastModified>
                        </Contents>
                        <Contents>
                            <Key>sub/path/v102.tgz</Key>
                            <LastModified>2024-07-04T04:51:50.000Z</LastModified>
                        </Contents>
                        <Contents>
                            <Key>sub/path/v101.tgz</Key>
                            <LastModified>2024-07-01T19:40:05.000Z</LastModified>
                        </Contents>
                    </ListBucketResult>",
                ))
                .unwrap(),
        );
        let get_object_2 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/sub/path/v102.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(read_fixture_archive_data()))
                .unwrap(),
        );
        let replay_client =
            StaticReplayClient::new(vec![get_object_1, list_object_1, get_object_2]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = download_specific_or_latest_with_client(
            &s3,
            &"test-bucket".to_string(),
            &"sub/path/static-artifacts.tgz".to_string(),
            output_dir,
        )
        .await;

        println!("{result:#?}");
        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
        assert!(fs::metadata(output_dir).is_ok());
        assert!(fs::metadata(output_dir.join("index.html")).is_ok());
        assert!(fs::metadata(output_dir.join("images")).is_ok());
        assert!(fs::metadata(output_dir.join("images/desktop-heroku-pride.jpg")).is_ok());
        fs::remove_dir_all(output_dir).expect("temporary directory should be deleted");
    }

    #[tokio::test]
    async fn download_specific_or_latest_with_client_latest_no_prefix_succeeds() {
        let unique = Uuid::new_v4();
        let output_dir_name = format!("test-output-static-artifacts-{unique}");
        let output_dir = Path::new(output_dir_name.as_str());
        fs::remove_dir_all(output_dir).unwrap_or_default();

        let get_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/static-artifacts.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(404)
                .body(SdkBody::from(r"
                    <Error>
                        <Code>NoSuchKey</Code>
                    </Error>",
                ))
                .unwrap(),
        );
        let list_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/?list-type=2&prefix=")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(
                    r"
                    <ListBucketResult>
                        <IsTruncated>false</IsTruncated>
                        <Contents>
                            <Key>v100.tgz</Key>
                            <LastModified>2024-07-01T12:20:47.000Z</LastModified>
                        </Contents>
                        <Contents>
                            <Key>v102.tgz</Key>
                            <LastModified>2024-07-04T04:51:50.000Z</LastModified>
                        </Contents>
                        <Contents>
                            <Key>v101.tgz</Key>
                            <LastModified>2024-07-01T19:40:05.000Z</LastModified>
                        </Contents>
                    </ListBucketResult>",
                ))
                .unwrap(),
        );
        let get_object_2 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/v102.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(read_fixture_archive_data()))
                .unwrap(),
        );
        let replay_client =
            StaticReplayClient::new(vec![get_object_1, list_object_1, get_object_2]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = download_specific_or_latest_with_client(
            &s3,
            &"test-bucket".to_string(),
            &"static-artifacts.tgz".to_string(),
            output_dir,
        )
        .await;

        println!("{result:#?}");
        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
        assert!(fs::metadata(output_dir).is_ok());
        assert!(fs::metadata(output_dir.join("index.html")).is_ok());
        assert!(fs::metadata(output_dir.join("images")).is_ok());
        assert!(fs::metadata(output_dir.join("images/desktop-heroku-pride.jpg")).is_ok());
        fs::remove_dir_all(output_dir).expect("temporary directory should be deleted");
    }

    #[tokio::test]
    async fn download_specific_or_latest_with_client_latest_empty() {
        let unique = Uuid::new_v4();
        let output_dir_name = format!("test-output-static-artifacts-{unique}");
        let output_dir = Path::new(output_dir_name.as_str());
        fs::remove_dir_all(output_dir).unwrap_or_default();

        let get_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/sub/path/static-artifacts.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(404)
                .body(SdkBody::from(r"
                    <Error>
                        <Code>NoSuchKey</Code>
                    </Error>",
                ))
                .unwrap(),
        );
        let list_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/?list-type=2&prefix=sub%2Fpath%2F")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(r"
                    <ListBucketResult>
                        <IsTruncated>false</IsTruncated>
                    </ListBucketResult>",
                ))
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![get_object_1, list_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = download_specific_or_latest_with_client(
            &s3,
            &"test-bucket".to_string(),
            &"sub/path/static-artifacts.tgz".to_string(),
            output_dir,
        )
        .await;

        println!("{result:#?}");
        assert!(result.is_err());
        replay_client.assert_requests_match(&[]);
        assert!(fs::metadata(output_dir).is_err());
    }

    #[tokio::test]
    async fn download_specific_or_latest_with_client_latest_no_prefix_empty() {
        let unique = Uuid::new_v4();
        let output_dir_name = format!("test-output-static-artifacts-{unique}");
        let output_dir = Path::new(output_dir_name.as_str());
        fs::remove_dir_all(output_dir).unwrap_or_default();

        let get_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/static-artifacts.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(404)
                .body(SdkBody::from(r"
                    <Error>
                        <Code>NoSuchKey</Code>
                    </Error>",
                ))
                .unwrap(),
        );
        let list_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/?list-type=2&prefix=")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(
                    r"
                    <ListBucketResult>
                        <IsTruncated>false</IsTruncated>
                    </ListBucketResult>",
                ))
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![get_object_1, list_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = download_specific_or_latest_with_client(
            &s3,
            &"test-bucket".to_string(),
            &"static-artifacts.tgz".to_string(),
            output_dir,
        )
        .await;

        println!("{result:#?}");
        assert!(result.is_err());
        replay_client.assert_requests_match(&[]);
        assert!(fs::metadata(output_dir).is_err());
    }

    #[tokio::test]
    async fn download_with_client_succeeds() {
        let unique = Uuid::new_v4();
        let output_dir_name = format!("test-output-static-artifacts-{unique}");
        let output_dir = Path::new(output_dir_name.as_str());
        fs::remove_dir_all(output_dir).unwrap_or_default();

        let get_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/sub/path/static-artifacts.tgz?x-id=GetObject")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(read_fixture_archive_data()))
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![get_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result = download_with_client(
            &s3,
            &"test-bucket".to_string(),
            &"sub/path/static-artifacts.tgz".to_string(),
            output_dir,
        )
        .await;

        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
        assert!(fs::metadata(output_dir).is_ok());
        assert!(fs::metadata(output_dir.join("index.html")).is_ok());
        assert!(fs::metadata(output_dir.join("images")).is_ok());
        assert!(fs::metadata(output_dir.join("images/desktop-heroku-pride.jpg")).is_ok());
        fs::remove_dir_all(output_dir).expect("temporary directory should be deleted");
    }

    #[test]
    fn sorted_dir_entries_succeeds() {
        let result = sorted_dir_entries("test/fixtures/archives-in-storage");
        eprintln!("{result:?}");
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result[0], String::from("release-angel.tgz"));
        assert_eq!(result[1], String::from("release-funzzies.tgz"));
        assert_eq!(result[2], String::from("release-bork.tgz"));
    }

    #[tokio::test]
    async fn find_latest_with_client_succeeds() {
        let list_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/?list-type=2&prefix=sub%2Fpath%2F")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(r"
                    <ListBucketResult>
                        <IsTruncated>false</IsTruncated>
                        <Contents>
                            <Key>v100.tgz</Key>
                            <LastModified>2024-07-01T12:20:47.000Z</LastModified>
                        </Contents>
                        <Contents>
                            <Key>v102.tgz</Key>
                            <LastModified>2024-07-04T04:51:50.000Z</LastModified>
                        </Contents>
                        <Contents>
                            <Key>v101.tgz</Key>
                            <LastModified>2024-07-01T19:40:05.000Z</LastModified>
                        </Contents>
                    </ListBucketResult>",
                ))
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![list_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result =
            find_latest_with_client(&s3, &"test-bucket".to_string(), &"sub/path/".to_string())
                .await;

        println!("find_latest_with_client_succeeds result {result:#?}");
        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
        assert!(result
            .expect("should be ok")
            .is_some_and(|f| f == "v102.tgz"));
    }

    #[tokio::test]
    async fn find_latest_with_client_empty() {
        let list_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/?list-type=2&prefix=sub%2Fpath%2F")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(r"
                    <ListBucketResult>
                        <IsTruncated>false</IsTruncated>
                    </ListBucketResult>",
                ))
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![list_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );

        let result =
            find_latest_with_client(&s3, &"test-bucket".to_string(), &"sub/path/".to_string())
                .await;

        println!("find_latest_with_client_succeeds result {result:#?}");
        assert!(result.is_ok());
        replay_client.assert_requests_match(&[]);
        assert!(result.expect("should be ok").is_none());
    }

    fn read_fixture_archive_data() -> std::vec::Vec<u8> {
        let mut archive_file = File::open(Path::new("test/fixtures/static-artifacts.tgz"))
            .expect("test fixture file should exist");
        let mut archive_data = Vec::new();
        archive_file
            .read_to_end(&mut archive_data)
            .expect("test fixture file should contain bytes");
        archive_data
    }

    #[test]
    fn guard_s3_should_pass_with_required_env() {
        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_ACCESS_KEY_ID".to_string(),
            "test-key".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_SECRET_ACCESS_KEY".to_string(),
            "test-secret".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://test-bucket.s3.us-west-2.amazonaws.com".to_string(),
        );

        let result = guard_s3(&test_env);
        assert!(result.is_ok());
    }

    #[test]
    fn guard_s3_should_fail_missing_requirements() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_ACCESS_KEY_ID".to_string(),
            "test-key".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_SECRET_ACCESS_KEY".to_string(),
            "test-secret".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://test-bucket.s3.us-west-2.amazonaws.com".to_string(),
        );

        let result = guard_s3(&test_env);
        assert!(result.is_err());

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_SECRET_ACCESS_KEY".to_string(),
            "test-secret".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://test-bucket.s3.us-west-2.amazonaws.com".to_string(),
        );

        let result = guard_s3(&test_env);
        assert!(result.is_err());

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_ACCESS_KEY_ID".to_string(),
            "test-key".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://test-bucket.s3.us-west-2.amazonaws.com".to_string(),
        );

        let result = guard_s3(&test_env);
        assert!(result.is_err());

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_ACCESS_KEY_ID".to_string(),
            "test-key".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_SECRET_ACCESS_KEY".to_string(),
            "test-secret".to_string(),
        );

        let result = guard_s3(&test_env);
        assert!(result.is_err());
    }

    #[test]
    fn guard_file_should_pass_with_required_env() {
        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "file:///volumes/static-artifacts".to_string(),
        );

        let result = guard_file(&test_env);
        assert!(result.is_ok());
    }

    #[test]
    fn guard_file_should_fail_missing_requirements() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "file:///volumes/static-artifacts".to_string(),
        );

        let result = guard_file(&test_env);
        assert!(result.is_err());

        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "test-release-id".to_string());

        let result = guard_file(&test_env);
        assert!(result.is_err());
    }

    #[test]
    fn generate_archive_name_with_release_id() {
        let mut test_env = HashMap::new();
        test_env.insert("RELEASE_ID".to_string(), "xxxxx".to_string());
        let result = generate_archive_name(&test_env);
        assert_eq!(result, "release-xxxxx.tgz".to_string());
    }

    #[test]
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    fn generate_archive_name_without_release_id() {
        let test_env = HashMap::new();

        let result = generate_archive_name(&test_env);
        assert!(result.starts_with("artifact-"));
        assert!(result.ends_with(".tgz"));
    }

    #[test]
    fn generate_s3_storage_location_without_path_in_url() {
        let mut test_env = HashMap::new();
        test_env.insert("STATIC_ARTIFACTS_URL".to_string(), "s3://xxxxx".to_string());
        let test_name = String::from("test-name.tgz");

        let result = generate_s3_storage_location(&test_env, &test_name);
        println!("{result:#?}");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("result is ok"),
            ("xxxxx".to_string(), None, "test-name.tgz".to_string())
        );
    }

    #[test]
    fn generate_s3_storage_location_without_region() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://xxxxx/yyyyy".to_string(),
        );
        let test_name = String::from("test-name.tgz");

        let result = generate_s3_storage_location(&test_env, &test_name);
        println!("{result:#?}");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("result is ok"),
            ("xxxxx".to_string(), None, "yyyyy/test-name.tgz".to_string())
        );
    }

    #[test]
    fn generate_s3_storage_location_with_region_in_url() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://xxxxx.s3.us-west-2.amazonaws.com/yyyyy".to_string(),
        );
        let test_name = String::from("test-name.tgz");

        let result = generate_s3_storage_location(&test_env, &test_name);
        println!("{result:#?}");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("result is ok"),
            (
                "xxxxx".to_string(),
                Some("us-west-2".to_string()),
                "yyyyy/test-name.tgz".to_string()
            )
        );
    }

    #[test]
    fn generate_s3_storage_location_with_region_in_env() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://xxxxx/yyyyy".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_REGION".to_string(),
            "us-east-2".to_string(),
        );
        let test_name = String::from("test-name.tgz");

        let result = generate_s3_storage_location(&test_env, &test_name);
        println!("{result:#?}");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("result is ok"),
            (
                "xxxxx".to_string(),
                Some("us-east-2".to_string()),
                "yyyyy/test-name.tgz".to_string()
            )
        );
    }

    #[test]
    fn generate_s3_storage_location_with_region_in_both() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://xxxxx.s3.us-west-2.amazonaws.com/yyyyy".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_REGION".to_string(),
            "us-east-2".to_string(),
        );
        let test_name = String::from("test-name.tgz");

        let result = generate_s3_storage_location(&test_env, &test_name);
        println!("{result:#?}");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("result is ok"),
            (
                "xxxxx".to_string(),
                Some("us-west-2".to_string()),
                "yyyyy/test-name.tgz".to_string()
            )
        );
    }

    #[test]
    fn generate_file_storage_location_succeeds() {
        let unique = Uuid::new_v4();
        let output_archive_dir = format!("test-file-storage-location-{unique}");
        let abs_root = env::current_dir().expect("should have a current working directory");
        let output_archive_dir_path = Path::new(&abs_root).join(output_archive_dir.as_str());
        fs::remove_dir_all(&output_archive_dir_path).unwrap_or_default();

        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            format!("file://{}", output_archive_dir_path.to_string_lossy()),
        );
        let test_name = String::from("test-name.tgz");

        let result = generate_file_storage_location(&test_env, &test_name);
        println!("{result:#?}");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("result is ok"),
            output_archive_dir_path.join(test_name)
        );

        fs::remove_dir_all(output_archive_dir_path).expect("temporary directory should be deleted");
    }

    #[tokio::test]
    async fn generate_s3_client_with_region() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_ACCESS_KEY_ID".to_string(),
            "test-key-id".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_SECRET_ACCESS_KEY".to_string(),
            "test-key-secret".to_string(),
        );
        let test_bucket_region = String::from("us-west-1");

        let result = generate_s3_client(&test_env, Some(test_bucket_region)).await;
        assert!(result
            .config()
            .region()
            .is_some_and(|r| r.to_string() == "us-west-1"));
    }

    #[tokio::test]
    async fn generate_s3_client_without_region() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_ACCESS_KEY_ID".to_string(),
            "test-key-id".to_string(),
        );
        test_env.insert(
            "STATIC_ARTIFACTS_SECRET_ACCESS_KEY".to_string(),
            "test-key-secret".to_string(),
        );

        let result = generate_s3_client(&test_env, None).await;
        assert!(result
            .config()
            .region()
            .is_some_and(|r| r.to_string() == "us-east-1"));
    }

    #[test]
    fn detect_storage_scheme_return_scheme() {
        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "file:///volumes/static-artifacts".to_string(),
        );

        let result = detect_storage_scheme(&test_env).expect("should parse the URL");
        assert_eq!(result, "file".to_string());

        let mut test_env = HashMap::new();
        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            "s3://bucket-of-static-artifacts/path/to/them".to_string(),
        );

        let result = detect_storage_scheme(&test_env).expect("should parse the URL");
        assert_eq!(result, "s3".to_string());
    }

    #[test]
    fn parse_s3_url_returns_parts() {
        let (bucket_name, bucket_region, bucket_path) =
            parse_s3_url("s3://test-bucket.s3.us-west-2.amazonaws.com/sub/path")
                .expect("should parse the URL");
        assert_eq!(bucket_name, "test-bucket".to_string());
        assert_eq!(bucket_region, Some("us-west-2".to_string()));
        assert_eq!(bucket_path, Some("sub/path".to_string()));

        let (bucket_name, bucket_region, bucket_path) =
            parse_s3_url("s3://test-bare-name/sub/path").expect("should parse the URL");
        assert_eq!(bucket_name, "test-bare-name".to_string());
        assert_eq!(bucket_region, None);
        assert_eq!(bucket_path, Some("sub/path".to_string()));

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
            ReleaseArtifactsError::StorageURLHostMissing(_)
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

        create_archive(
            Path::new("test/fixtures/static-artifacts"),
            Path::new(output_file.as_str()),
        )
        .unwrap();
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

    #[tokio::test]
    async fn garbage_collect_should_succeed_with_empty_dir() {
        let mut test_env = HashMap::new();

        // TODO: file test_env helper
        let unique = Uuid::new_v4();
        let output_archive_dir = format!("test-file-storage-location-{unique}");
        let abs_root = env::current_dir().expect("should have a current working directory");
        let output_archive_dir_path = Path::new(&abs_root).join(output_archive_dir.as_str());
        fs::remove_dir_all(&output_archive_dir_path).unwrap_or_default();
        fs::create_dir_all(&output_archive_dir_path).unwrap_or_default();

        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            format!("file://{}", output_archive_dir_path.to_string_lossy()),
        );

        let result = gc(&test_env).await;

        eprintln!("result is: {result:?}");
        assert!(result.is_ok())
    }

    #[tokio::test]
    async fn garbage_collect_should_remove_files_older_than_the_first_two() {
        let mut test_env = HashMap::new();

        // TODO: file test_env helper
        let unique = Uuid::new_v4();
        let output_archive_dir = format!("test-file-storage-location-{unique}");
        let abs_root = env::current_dir().expect("should have a current working directory");
        let output_archive_dir_path = Path::new(&abs_root).join(output_archive_dir.as_str());
        fs::remove_dir_all(&output_archive_dir_path).unwrap_or_default();
        fs::create_dir_all(&output_archive_dir_path).unwrap_or_default();

        let test_path_1 = output_archive_dir_path.join("test1.tgz");
        let test_file_1 = File::create_new(test_path_1.clone()).unwrap();
        test_file_1
            .set_modified(SystemTime::now() - Duration::new(120, 0))
            .unwrap();

        let test_path_2 = output_archive_dir_path.join("test2.tgz");
        let test_file_2 = File::create_new(test_path_2.clone()).unwrap();
        test_file_2
            .set_modified(SystemTime::now() - Duration::new(60, 0))
            .unwrap();

        let test_path_3 = output_archive_dir_path.join("test3.tgz");
        let test_file_3 = File::create_new(test_path_3.clone()).unwrap();
        test_file_3.set_modified(SystemTime::now()).unwrap();

        let entries = fs::read_dir(output_archive_dir_path.clone()).unwrap();
        assert!(entries.count() == 3);

        test_env.insert(
            "STATIC_ARTIFACTS_URL".to_string(),
            format!("file://{}", output_archive_dir_path.to_string_lossy()),
        );

        let result = gc(&test_env).await;
        eprintln!("{result:?}");
        assert!(result.is_ok());

        assert!(!test_path_1.exists());
        assert!(test_path_2.exists());
        assert!(test_path_3.exists());

        fs::remove_dir_all(&output_archive_dir_path).unwrap_or_default();
    }

    #[tokio::test]
    async fn garbage_collect_should_remove_s3_archives_older_than_the_first_two() {
        let list_object_1 = ReplayEvent::new(
            http::Request::builder()
                .method("GET")
                .uri("https://test-bucket.s3.us-east-1.amazonaws.com/?list-type=2&prefix=sub%2Fpath%2F")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(SdkBody::from(r"
                    <ListBucketResult>
                        <IsTruncated>false</IsTruncated>
                    </ListBucketResult>",
                ))
                .unwrap(),
        );
        let replay_client = StaticReplayClient::new(vec![list_object_1]);
        let s3 = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(make_s3_test_credentials())
                .region(aws_sdk_s3::config::Region::new("us-east-1"))
                .http_client(replay_client.clone())
                .build(),
        );
    }
}
