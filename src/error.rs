use std::result;
use thiserror::Error;

pub type Result<T> = result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("MQTT error: {0}")]
    Mqtt(#[from] paho_mqtt::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] ws::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),

    #[error("gRPC transport error: {0}")]
    GrpcTransport(#[from] tonic::transport::Error),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Schema error: {0}")]
    Schema(String),

    #[error("Transformation error: {0}")]
    Transform(String),

    #[error("GraphQL error: {0}")]
    GraphQL(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
} 