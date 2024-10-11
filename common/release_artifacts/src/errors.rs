#[derive(Debug)]
pub enum ReleaseArtifactsError {
    ArchiveError(std::io::Error),
    ArchiveStreamError(aws_sdk_s3::primitives::ByteStreamError),
    ConfigMissing(String),
    StorageError(String),
    StorageURLInvalid(url::ParseError),
    StorageURLHostMissing(String),
}

impl<T: aws_sdk_s3::error::ProvideErrorMetadata> From<T> for ReleaseArtifactsError {
    fn from(value: T) -> Self {
        ReleaseArtifactsError::StorageError(format!(
            "{}: {}",
            value.code().map_or("unknown code".into(), String::from),
            value
                .message()
                .map_or("missing reason".into(), String::from),
        ))
    }
}
