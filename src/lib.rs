pub mod reader;
use thiserror::Error;

/// The error type for the `zarchive` crate.
#[derive(Debug, Error)]
pub enum ZArchiveError {
    #[error("Invalid file path: {0}")]
    InvalidFilePath(String),
    #[error("Archive entry is not a directory: {0}")]
    NotADirectory(String),
    #[error("Destination is not a directory: {0}")]
    InvalidDestination(String),
    #[error("File not in archive: {0}")]
    MissingFile(String),
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("{0}")]
    Other(#[from] cxx::Exception),
}
type Result<T> = std::result::Result<T, ZArchiveError>;
