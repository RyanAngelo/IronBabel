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
        config.validate()?;
        Ok(config)
    }

    /// Validates the loaded configuration, returning a descriptive error if
    /// any field has an obviously invalid value.
    ///
    /// This catches misconfiguration early — at startup — rather than
    /// surfacing confusing errors on the first request.
    pub fn validate(&self) -> Result<()> {
        use crate::error::Error;

        if self.host.trim().is_empty() {
            return Err(Error::Config(
                "host must not be empty".to_string(),
            ));
        }

        if self.port == 0 {
            return Err(Error::Config(
                "port must be between 1 and 65535".to_string(),
            ));
        }

        for route in &self.routes {
            if route.path.is_empty() {
                return Err(Error::Config(
                    "route path must not be empty".to_string(),
                ));
            }
            if !route.path.starts_with('/') {
                return Err(Error::Config(format!(
                    "route path '{}' must start with '/'",
                    route.path
                )));
            }
            match &route.transport {
                TransportConfig::Http(cfg) => {
                    validate_http_url(&cfg.url, &route.path)?;
                }
                TransportConfig::GraphQL(cfg) => {
                    validate_http_url(&cfg.url, &route.path)?;
                }
                TransportConfig::Grpc(cfg) => {
                    validate_http_url(&cfg.url, &route.path)?;
                }
                TransportConfig::WebSocket(cfg) => {
                    validate_ws_url(&cfg.url, &route.path)?;
                }
                TransportConfig::Mqtt(cfg) => {
                    validate_mqtt_broker_url(&cfg.broker_url, &route.path)?;
                    validate_mqtt_topic(&cfg.topic, &route.path)?;
                    validate_mqtt_qos(cfg.qos, &route.path)?;
                }
                TransportConfig::Zmq(cfg) => {
                    if cfg.address.trim().is_empty() {
                        return Err(Error::Config(format!(
                            "ZMQ route '{}': address must not be empty",
                            route.path
                        )));
                    }
                }
            }
        }

        for listener in &self.listeners {
            match listener {
                ListenerConfig::ZmqPull(cfg) => {
                    if cfg.bind.trim().is_empty() {
                        return Err(Error::Config(
                            "ZMQ pull listener: bind address must not be empty".to_string(),
                        ));
                    }
                    validate_http_url(&cfg.forward_to, "zmq_pull listener forward_to")?;
                }
                ListenerConfig::MqttSub(cfg) => {
                    validate_mqtt_broker_url(&cfg.broker_url, "mqtt_sub listener broker_url")?;
                    if cfg.topics.is_empty() {
                        return Err(Error::Config(
                            "mqtt_sub listener: topics must not be empty".to_string(),
                        ));
                    }
                    for topic in &cfg.topics {
                        validate_mqtt_topic(topic, "mqtt_sub listener topic")?;
                    }
                    validate_mqtt_qos(cfg.qos, "mqtt_sub listener qos")?;
                    validate_http_url(&cfg.forward_to, "mqtt_sub listener forward_to")?;
                }
            }
        }

        Ok(())
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
    GraphQL(GraphQLTransportConfig),
    Grpc(GrpcTransportConfig),
    WebSocket(WebSocketTransportConfig),
    Mqtt(MqttTransportConfig),
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

/// Configuration for a GraphQL upstream route.
///
/// Requests are always forwarded as HTTP POST with `Content-Type: application/json`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphQLTransportConfig {
    /// Target GraphQL endpoint URL, e.g. `"http://api:4000/graphql"`.
    ///
    /// SECURITY: must be a trusted internal address — request data can never
    /// alter the destination URL (SSRF mitigation).
    pub url: String,
    /// Request timeout in seconds. Defaults to 30.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

/// Configuration for a gRPC upstream route.
///
/// Requests are forwarded over HTTP/2 with `Content-Type: application/grpc`.
/// The request body must be a gRPC length-prefixed frame (5-byte header +
/// protobuf payload); use `GrpcProtocol::encode` to add the framing.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GrpcTransportConfig {
    /// Base URL of the upstream gRPC server, e.g. `"http://service:50051"`.
    ///
    /// SECURITY: must be a trusted internal address — request data can never
    /// alter the destination URL (SSRF mitigation).
    pub url: String,
    /// Request timeout in seconds. Defaults to 30.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

/// Configuration for a WebSocket upstream route.
///
/// Incoming WebSocket upgrade requests are proxied bidirectionally to the
/// backend WebSocket server. `ws://`, `wss://`, `http://`, and `https://`
/// URL schemes are all accepted.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebSocketTransportConfig {
    /// Backend WebSocket URL, e.g. `"ws://realtime:8080"` or `"wss://host/ws"`.
    ///
    /// SECURITY: must be a trusted internal address — request data can never
    /// alter the destination URL (SSRF mitigation).
    pub url: String,
    /// Connection timeout in seconds. Defaults to 30.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

/// Configuration for an MQTT publish route.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MqttTransportConfig {
    /// MQTT broker URL. Accepted schemes: `mqtt://`, `mqtts://`, `tcp://`, `ssl://`.
    pub broker_url: String,
    /// Topic to publish each request body to.
    pub topic: String,
    /// MQTT QoS level (0, 1, or 2). Defaults to 0.
    #[serde(default)]
    pub qos: u8,
    /// Whether the message should be retained by the broker. Defaults to false.
    #[serde(default)]
    pub retain: bool,
    /// Optional client ID for the publisher connection.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Publish timeout in seconds. Defaults to 30.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
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
    MqttSub(MqttSubListenerConfig),
}

/// Binds a ZMQ PULL socket and forwards each received frame as an HTTP POST.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ZmqPullListenerConfig {
    /// ZMQ address to bind in `host:port` form, e.g. `"127.0.0.1:5557"`.
    pub bind: String,
    /// HTTP URL to POST each received frame to.
    pub forward_to: String,
}

/// Subscribes to MQTT topics and forwards each publish as an HTTP POST.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MqttSubListenerConfig {
    /// MQTT broker URL. Accepted schemes: `mqtt://`, `mqtts://`, `tcp://`, `ssl://`.
    pub broker_url: String,
    /// Topics to subscribe to.
    pub topics: Vec<String>,
    /// MQTT QoS level for subscriptions (0, 1, or 2). Defaults to 0.
    #[serde(default)]
    pub qos: u8,
    /// Optional client ID for the subscriber connection.
    #[serde(default)]
    pub client_id: Option<String>,
    /// HTTP URL to POST each received payload to.
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
    #[serde(default)]
    pub logging: LoggingConfig,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Set to true to log requests and responses through the middleware chain.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

// ---------------------------------------------------------------------------
// Serde defaults
// ---------------------------------------------------------------------------

fn default_timeout_secs() -> u64 { 30 }
fn default_requests_per_window() -> u32 { 100 }
fn default_window_secs() -> u64 { 60 }

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_http_url(url: &str, context: &str) -> crate::error::Result<()> {
    if url.trim().is_empty() {
        return Err(crate::error::Error::Config(format!(
            "{}: URL must not be empty",
            context
        )));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(crate::error::Error::Config(format!(
            "{}: URL '{}' must start with http:// or https://",
            context, url
        )));
    }
    Ok(())
}

fn validate_ws_url(url: &str, context: &str) -> crate::error::Result<()> {
    if url.trim().is_empty() {
        return Err(crate::error::Error::Config(format!(
            "{}: URL must not be empty",
            context
        )));
    }
    let ok = url.starts_with("ws://")
        || url.starts_with("wss://")
        || url.starts_with("http://")
        || url.starts_with("https://");
    if !ok {
        return Err(crate::error::Error::Config(format!(
            "{}: WebSocket URL '{}' must start with ws://, wss://, http://, or https://",
            context, url
        )));
    }
    Ok(())
}

fn validate_mqtt_broker_url(url: &str, context: &str) -> crate::error::Result<()> {
    if url.trim().is_empty() {
        return Err(crate::error::Error::Config(format!(
            "{}: broker URL must not be empty",
            context
        )));
    }
    let ok = url.starts_with("mqtt://")
        || url.starts_with("mqtts://")
        || url.starts_with("tcp://")
        || url.starts_with("ssl://");
    if !ok {
        return Err(crate::error::Error::Config(format!(
            "{}: MQTT broker URL '{}' must start with mqtt://, mqtts://, tcp://, or ssl://",
            context, url
        )));
    }
    Ok(())
}

fn validate_mqtt_topic(topic: &str, context: &str) -> crate::error::Result<()> {
    if topic.trim().is_empty() {
        return Err(crate::error::Error::Config(format!(
            "{}: MQTT topic must not be empty",
            context
        )));
    }
    Ok(())
}

fn validate_mqtt_qos(qos: u8, context: &str) -> crate::error::Result<()> {
    if qos > 2 {
        return Err(crate::error::Error::Config(format!(
            "{}: MQTT QoS must be 0, 1, or 2",
            context
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sub-modules
// ---------------------------------------------------------------------------

pub mod file;
pub mod env;
