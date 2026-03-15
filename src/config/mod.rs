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
    #[serde(default)]
    pub zmq_listeners: Vec<ZmqListenerConfig>,
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

/// Which ZMQ messaging pattern a route uses.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ZmqPattern {
    /// Gateway sends a REQ frame and waits for the upstream REP — synchronous, like HTTP.
    ReqRep,
    /// Gateway pushes a frame to the upstream PULL socket — fire-and-forget, returns 202.
    Push,
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
    ///
    /// Use `http://` or `https://` for HTTP upstreams; `zmq://host:port` for
    /// ZeroMQ upstreams (requires `zmq_pattern` to also be set).
    pub target: String,
    /// Allowed HTTP methods (e.g. ["GET", "POST"]). An empty list allows any method.
    pub methods: Vec<String>,
    /// Optional request timeout in seconds.
    pub timeout_secs: Option<u64>,
    /// If true, strip the matched path prefix before forwarding to the target.
    pub strip_prefix: Option<bool>,
    /// ZMQ messaging pattern. Required when `target` starts with `zmq://`.
    #[serde(default)]
    pub zmq_pattern: Option<ZmqPattern>,
}

/// A ZMQ→HTTP bridge: the gateway binds a PULL socket and forwards every
/// received frame as an HTTP POST to `target`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ZmqListenerConfig {
    /// ZMQ address to bind, e.g. `zmq://127.0.0.1:5557`
    pub listen: String,
    /// HTTP URL to POST each received frame to.
    pub target: String,
}

pub mod file;
pub mod env;
