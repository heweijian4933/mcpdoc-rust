//! 错误类型定义

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DocError {
    #[error("local file not allowed: {0}. Allowed files: {1}")]
    FileNotAllowed(String, String),

    #[error("URL not allowed. Must start with one of the following domains: {0}")]
    UrlNotAllowed(String),

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("index error: {0}")]
    IndexError(String),
}
