#[derive(Debug)]
pub enum ReleaseArtifactsError {
    ArchiveError(std::io::Error, String),
    ArchiveStreamError(aws_sdk_s3::primitives::ByteStreamError),
    ConfigMissing(String),
    StorageError(String),
    StorageKeyNotFound(String),
    StorageURLInvalid(url::ParseError),
    StorageURLHostMissing(String),
}

impl<T: std::error::Error + aws_sdk_s3::error::ProvideErrorMetadata> From<T>
    for ReleaseArtifactsError
{
    fn from(value: T) -> Self {
        match value.code() {
            Some(code) => match code {
                "NoSuchKey" => ReleaseArtifactsError::StorageKeyNotFound("Not Found".to_string()),
                _ => ReleaseArtifactsError::StorageError(format!(
                    "{code}: {}",
                    value.message().map_or("(no message)".into(), String::from)
                )),
            },
            _ => ReleaseArtifactsError::StorageError(format!(
                "{}",
                aws_smithy_types::error::display::DisplayErrorContext(value)
            )),
        }
    }
}
