use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("http error: {0}")]
    Http(String),
    #[error("invalid argument: {0}")]
    InvalidArg(String),
}

pub type AppResult<T> = Result<T, AppError>;
