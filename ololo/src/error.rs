use thiserror::Error;

#[derive(Debug, Error)]
pub enum OlolError {
    #[error("storage error: {0}")]
    StorageError(String),
}
