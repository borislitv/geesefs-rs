use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("filesystem error: {0}")]
    Fs(i32),
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("allocation of {requested} bytes exceeds budget {limit}")]
    Budget { requested: usize, limit: usize },
    #[error("operation cancelled")]
    Cancelled,
    #[error("{0} timed out")]
    Timeout(&'static str),
    #[error("invalid backend response: {0}")]
    InvalidResponse(String),
    #[error("S3 {operation} failed ({code}): {message}")]
    Backend {
        operation: &'static str,
        code: String,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
