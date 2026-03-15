use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use crate::{error::Result, protocols::Protocol};
use crate::protocols::{
    http::HttpProtocol, grpc::GrpcProtocol, graphql::GraphQLProtocol,
    mqtt::MqttProtocol, ws::WebSocketProtocol,
};
use crate::admin::store::MetricsStore;
use crate::config::ZmqPattern;
use crate::gateway::zmq::ZmqGateway;
use super::Gateway;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    pub router: Arc<crate::core::Router>,
    pub http_gateway: Arc<crate::gateway::http::HttpGateway>,
    pub metrics: Arc<MetricsStore>,
    pub config_routes: Vec<crate::config::RouteConfig>,
}

// ---------------------------------------------------------------------------
// Request handler
// ---------------------------------------------------------------------------

async fn handle_request(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let start = std::time::Instant::now();
    let method_str = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());

    // Security: reject path traversal attempts.
    if path.contains("..") {
        let latency_ms = start.elapsed().as_millis() as u64;
        state.metrics.record_request(&method_str, &path, None, "", 400, latency_ms, None).await;
        return (StatusCode::BAD_REQUEST, "Invalid path").into_response();
    }

    let route = match state.router.route(&method_str, &path) {
        None => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(&method_str, &path, None, "", 404, latency_ms, None).await;
            return (StatusCode::NOT_FOUND, "Not found").into_response();
        }
        Some(r) => r.clone(),
    };

    let matched_route = Some(route.path.clone());
    let target = route.target.clone();

    // Extract headers before consuming the body (injection guard: drop
    // headers whose value contains non-visible ASCII).
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|val| (k.as_str().to_string(), val.to_string()))
        })
        .collect();

    // Extract body (capped at 10 MB — belt-and-suspenders alongside the middleware layer).
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b.to_vec(),
        Err(_) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(&method_str, &path, matched_route, &target, 413, latency_ms, None).await;
            return (StatusCode::PAYLOAD_TOO_LARGE, "Body too large").into_response();
        }
    };

    // Dispatch on target scheme.
    if target.starts_with("zmq://") {
        return handle_zmq(&state, &method_str, &path, &target, &route, body_bytes, start).await;
    }

    // ── HTTP proxy ───────────────────────────────────────────────────────────

    let reqwest_method = match reqwest::Method::from_bytes(method_str.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(&method_str, &path, matched_route, &target, 405, latency_ms, None).await;
            return (StatusCode::METHOD_NOT_ALLOWED, "Invalid method").into_response();
        }
    };

    let timeout = route.timeout_secs.unwrap_or(30);

    match state
        .http_gateway
        .proxy(reqwest_method, &target, &path, query.as_deref(), &headers, body_bytes, timeout)
        .await
    {
        Ok((status, resp_headers, resp_body)) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(&method_str, &path, matched_route, &target, status, latency_ms, None).await;
            let mut builder = axum::response::Response::builder().status(status);
            for (name, value) in resp_headers {
                builder = builder.header(name, value);
            }
            builder
                .body(axum::body::Body::from(resp_body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            tracing::error!("Proxy error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(&method_str, &path, matched_route, &target, 502, latency_ms, Some(e.to_string())).await;
            (StatusCode::BAD_GATEWAY, "Bad gateway").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// ZMQ dispatch helper
// ---------------------------------------------------------------------------

async fn handle_zmq(
    state: &AppState,
    method_str: &str,
    path: &str,
    target: &str,
    route: &crate::config::RouteConfig,
    body: Vec<u8>,
    start: std::time::Instant,
) -> Response {
    let matched_route = Some(route.path.clone());
    let timeout = route.timeout_secs.unwrap_or(30);
    let gw = ZmqGateway::new();

    match route.zmq_pattern.as_ref() {
        Some(ZmqPattern::ReqRep) => {
            match gw.forward_req_rep(target, body, timeout).await {
                Ok(resp_body) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, target, 200, latency_ms, None).await;
                    axum::response::Response::builder()
                        .status(200)
                        .header("content-type", "application/octet-stream")
                        .body(axum::body::Body::from(resp_body))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
                }
                Err(e) => {
                    tracing::error!("ZMQ REQ/REP error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, target, 502, latency_ms, Some(e.to_string())).await;
                    (StatusCode::BAD_GATEWAY, format!("ZMQ error: {}", e)).into_response()
                }
            }
        }
        Some(ZmqPattern::Push) => {
            match gw.forward_push(target, body).await {
                Ok(()) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, target, 202, latency_ms, None).await;
                    StatusCode::ACCEPTED.into_response()
                }
                Err(e) => {
                    tracing::error!("ZMQ PUSH error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, target, 502, latency_ms, Some(e.to_string())).await;
                    (StatusCode::BAD_GATEWAY, format!("ZMQ error: {}", e)).into_response()
                }
            }
        }
        None => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, target, 500, latency_ms, Some("no zmq_pattern".to_string())).await;
            (StatusCode::INTERNAL_SERVER_ERROR, "Route targets zmq:// but has no zmq_pattern configured").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// GatewayConfig
// ---------------------------------------------------------------------------

pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
    pub protocols: Vec<Arc<dyn Protocol>>,
    pub routes: Vec<crate::config::RouteConfig>,
    pub zmq_listeners: Vec<crate::config::ZmqListenerConfig>,
}

impl GatewayConfig {
    pub fn from_config(config: crate::config::GatewayConfig) -> Result<Self> {
        let host = config.host.clone();
        let port = config.port;
        let routes = config.routes.clone();
        let zmq_listeners = config.zmq_listeners.clone();

        let mut protocols = Vec::new();

        for protocol_config in config.protocols {
            let protocol: Arc<dyn Protocol> = match protocol_config.name.as_str() {
                "http" => Arc::new(HttpProtocol::new(protocol_config.settings)?),
                "grpc" => Arc::new(GrpcProtocol::new(protocol_config.settings)?),
                "graphql" => Arc::new(GraphQLProtocol::new(protocol_config.settings)?),
                "mqtt" => Arc::new(MqttProtocol::new(protocol_config.settings)?),
                "websocket" => Arc::new(WebSocketProtocol::new(protocol_config.settings)?),
                _ => return Err(crate::error::Error::Protocol(format!(
                    "Unsupported protocol: {}",
                    protocol_config.name
                ))),
            };

            if protocol_config.enabled {
                protocols.push(protocol);
            }
        }

        Ok(Self { host, port, protocols, routes, zmq_listeners })
    }
}

// ---------------------------------------------------------------------------
// DefaultGateway
// ---------------------------------------------------------------------------

pub struct DefaultGateway {
    host: String,
    port: u16,
    protocols: Vec<Arc<dyn Protocol>>,
    router: Arc<crate::core::Router>,
    http_gateway: Arc<crate::gateway::http::HttpGateway>,
    metrics: Arc<MetricsStore>,
    config_routes: Vec<crate::config::RouteConfig>,
    zmq_listeners: Vec<crate::config::ZmqListenerConfig>,
    shutdown: Arc<Notify>,
}

#[async_trait]
impl Gateway for DefaultGateway {
    async fn start(&self) -> Result<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&addr).await.map_err(crate::error::Error::Io)?;

        tracing::info!("IronBabel gateway listening on {}", addr);
        tracing::info!("Admin dashboard available at http://{}/admin/", addr);

        let state = AppState {
            router: Arc::clone(&self.router),
            http_gateway: Arc::clone(&self.http_gateway),
            metrics: Arc::clone(&self.metrics),
            config_routes: self.config_routes.clone(),
        };

        // 1-second tick task for admin dashboard time buckets.
        let metrics_tick = Arc::clone(&self.metrics);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                metrics_tick.tick().await;
            }
        });

        // ZMQ → HTTP pull listener tasks (one per zmq_listeners entry).
        for listener_cfg in &self.zmq_listeners {
            let cfg = listener_cfg.clone();
            tracing::info!("Spawning ZMQ PULL listener: {} → {}", cfg.listen, cfg.target);
            tokio::spawn(async move {
                crate::gateway::zmq::run_pull_listener(cfg).await;
            });
        }

        let admin_router = crate::admin::router::build_admin_router();

        let app = axum::Router::new()
            .merge(admin_router)
            .route("/{*path}", any(handle_request))
            .route("/", any(handle_request))
            .with_state(state)
            .layer(TraceLayer::new_for_http())
            .layer(TimeoutLayer::new(Duration::from_secs(30)))
            .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024));

        let shutdown = Arc::clone(&self.shutdown);
        let shutdown_signal = async move { shutdown.notified().await };

        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal)
                .await
            {
                tracing::error!("Gateway server error: {}", e);
            }
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.shutdown.notify_one();
        Ok(())
    }

    fn protocols(&self) -> Vec<Arc<dyn Protocol>> {
        self.protocols.clone()
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub fn create_gateway(config: GatewayConfig) -> Result<DefaultGateway> {
    let router = Arc::new(crate::core::Router::new(config.routes.clone()));
    let http_protocol = Arc::new(HttpProtocol::new(serde_json::Value::Null)?);
    let http_gateway = Arc::new(crate::gateway::http::HttpGateway::new(http_protocol));
    let metrics = Arc::new(MetricsStore::new());
    Ok(DefaultGateway {
        host: config.host,
        port: config.port,
        protocols: config.protocols,
        router,
        http_gateway,
        metrics,
        config_routes: config.routes,
        zmq_listeners: config.zmq_listeners,
        shutdown: Arc::new(Notify::new()),
    })
}
