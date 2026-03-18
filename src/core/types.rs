use std::time::Duration;
use serde::{Deserialize, Serialize};

/// Represents a request to be processed by the gateway
#[derive(Debug, Clone)]
pub struct Request {
    pub data: Vec<u8>,
    pub metadata: RequestMetadata,
}

/// Metadata associated with a request
#[derive(Debug, Clone, Default)]
pub struct RequestMetadata {
    pub protocol: String,
    pub method: Option<String>,
    pub path: Option<String>,
    pub headers: Vec<(String, String)>,
    pub timeout: Option<Duration>,
    /// Verified remote IP address of the connecting client (from the TCP socket,
    /// not from headers). Used for rate limiting so clients cannot spoof their
    /// identity by manipulating `X-Forwarded-For`.
    pub remote_addr: Option<String>,
}

/// Represents a response from the gateway
#[derive(Debug, Clone)]
pub struct Response {
    pub data: Vec<u8>,
    pub metadata: ResponseMetadata,
}

/// Metadata associated with a response
#[derive(Debug, Clone, Default)]
pub struct ResponseMetadata {
    pub status_code: Option<u16>,
    pub headers: Vec<(String, String)>,
    pub duration: Option<Duration>,
}

/// Configuration for middleware
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiddlewareConfig {
    pub enabled: bool,
    pub settings: serde_json::Value,
} 