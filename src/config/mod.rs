use serde::{Deserialize, Serialize};
use crate::error::Result;
use crate::config::file::load_from_file;
use crate::config::env::{apply_env_overrides, config_file_path};

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub port: u16,
    pub host: String,
    pub protocols: Vec<ProtocolConfig>,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    /// Inbound listeners — e.g. ZMQ PULL sockets that forward to HTTP targets.
    #[serde(default)]
    pub listeners: Vec<ListenerConfig>,
    #[serde(default)]
    pub middleware: MiddlewareSectionConfig,
}

impl GatewayConfig {
    /// Loads configuration from the path given by `IRON_BABEL_CONFIG` (or
    /// `config/gateway.yaml` by default), then applies env var overrides.
    pub async fn load() -> Result<Self> {
        let path = config_file_path();
        let mut config = load_from_file(path).await?;
        apply_env_overrides(&mut config);
        Ok(config)
    }
}

// ---------------------------------------------------------------------------
// Protocol config (for protocol-level global settings)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct ProtocolConfig {
    pub name: String,
    pub enabled: bool,
    pub settings: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Route config
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RouteConfig {
    /// Path prefix to match, e.g. `"/api/v1"`.
    pub path: String,
    /// Allowed HTTP methods. An empty list allows any method.
    #[serde(default)]
    pub methods: Vec<String>,
    /// Transport-specific configuration — determines where and how to forward
    /// the request. Each protocol owns its own typed config block.
    pub transport: TransportConfig,
}

// ---------------------------------------------------------------------------
// Transport config (discriminated by `type:` in YAML)
// ---------------------------------------------------------------------------

/// Per-route transport configuration. The `type` field selects the variant;
/// each variant carries only the fields relevant to that transport.
///
/// Adding a new protocol means adding one new variant + one new struct here —
/// nothing in `RouteConfig` changes.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportConfig {
    Http(HttpTransportConfig),
    Zmq(ZmqTransportConfig),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HttpTransportConfig {
    /// Target URL, e.g. `"http://127.0.0.1:9000"`.
    ///
    /// SECURITY: must be a trusted internal address — request data can never
    /// alter the destination URL (SSRF mitigation).
    pub url: String,
    /// Request timeout in seconds. Defaults to 30.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Strip the matched path prefix before forwarding. Defaults to false.
    #[serde(default)]
    pub strip_prefix: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ZmqTransportConfig {
    /// ZMQ endpoint address in `host:port` form, e.g. `"127.0.0.1:5555"`.
    pub address: String,
    /// ZMQ messaging pattern — determines socket type and semantics.
    pub pattern: ZmqPattern,
    /// Request timeout in seconds. Defaults to 30.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Topic prefix for `pub_sub` pattern. Ignored by other patterns.
    #[serde(default)]
    pub topic: Option<String>,
}

/// ZMQ messaging pattern for a route.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ZmqPattern {
    /// Synchronous request/reply — gateway sends REQ and waits for REP.
    ReqRep,
    /// Fire-and-forget — gateway PUSHes a frame; returns 202 immediately.
    Push,
    /// Broadcast — gateway PUBlishes a frame with an optional topic prefix.
    PubSub,
}

// ---------------------------------------------------------------------------
// Listener config (inbound transports, discriminated by `type:` in YAML)
// ---------------------------------------------------------------------------

/// Inbound listener configuration. Listeners run as background tasks that
/// receive data on a non-HTTP transport and forward it to an HTTP target.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ListenerConfig {
    ZmqPull(ZmqPullListenerConfig),
}

/// Binds a ZMQ PULL socket and forwards each received frame as an HTTP POST.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ZmqPullListenerConfig {
    /// ZMQ address to bind in `host:port` form, e.g. `"127.0.0.1:5557"`.
    pub bind: String,
    /// HTTP URL to POST each received frame to.
    pub forward_to: String,
}

// ---------------------------------------------------------------------------
// Middleware config
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct MiddlewareSectionConfig {
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    /// Set to true to require a valid Bearer token on every request.
    #[serde(default)]
    pub enabled: bool,
    /// Allowed Bearer tokens. An empty list allows all tokens when enabled.
    #[serde(default)]
    pub api_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Set to true to enforce per-client request rate limits.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum requests allowed within `window_secs`. Defaults to 100.
    #[serde(default = "default_requests_per_window")]
    pub requests_per_window: u32,
    /// Sliding window size in seconds. Defaults to 60.
    #[serde(default = "default_window_secs")]
    pub window_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_window: default_requests_per_window(),
            window_secs: default_window_secs(),
        }
    }
}

// ---------------------------------------------------------------------------
// Serde defaults
// ---------------------------------------------------------------------------

fn default_timeout_secs() -> u64 { 30 }
fn default_requests_per_window() -> u32 { 100 }
fn default_window_secs() -> u64 { 60 }

// ---------------------------------------------------------------------------
// Sub-modules
// ---------------------------------------------------------------------------

pub mod file;
pub mod env;
