use std::result;
use thiserror::Error;

pub type Result<T> = result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("GraphQL error: {0}")]
    GraphQL(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("Unknown error: {0}")]
    Unknown(String),
}
