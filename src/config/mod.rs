use serde::{Deserialize, Serialize};
use crate::error::Result;
use crate::config::file::load_from_file;

#[derive(Debug, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub port: u16,
    pub host: String,
    pub protocols: Vec<ProtocolConfig>,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

impl GatewayConfig {
    /// Loads configuration from the default path
    pub async fn load() -> Result<Self> {
        load_from_file("config/gateway.yaml".to_string()).await
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProtocolConfig {
    pub name: String,
    pub enabled: bool,
    pub settings: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RouteConfig {
    /// Path prefix to match against incoming requests, e.g. "/api/v1"
    pub path: String,
    /// Backend URL to forward matched requests to.
    ///
    /// SECURITY: This must be a trusted internal address. IronBabel mitigates
    /// SSRF by only routing to targets explicitly defined in this config file —
    /// request data can never inject or alter the destination URL.
    pub target: String,
    /// Allowed HTTP methods (e.g. ["GET", "POST"]). An empty list allows any method.
    pub methods: Vec<String>,
    /// Optional request timeout in seconds.
    pub timeout_secs: Option<u64>,
    /// If true, strip the matched path prefix before forwarding to the target.
    pub strip_prefix: Option<bool>,
}

pub mod file;
pub mod env;
