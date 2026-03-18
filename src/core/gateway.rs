use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
};
use std::net::SocketAddr;
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
use crate::gateway::graphql::GraphQLGateway;
use crate::gateway::grpc::GrpcGateway;
use crate::admin::store::MetricsStore;
use crate::config::{
    GrpcTransportConfig, GraphQLTransportConfig, ListenerConfig,
    TransportConfig, ZmqPattern, ZmqTransportConfig,
};
use crate::gateway::zmq::ZmqGateway;
use crate::core::middleware::{AuthMiddleware, MiddlewareChain, RateLimitMiddleware};
use crate::core::types::MiddlewareConfig;
use super::Gateway;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    pub router: Arc<crate::core::Router>,
    pub http_gateway: Arc<crate::gateway::http::HttpGateway>,
    pub graphql_gateway: Arc<GraphQLGateway>,
    pub grpc_gateway: Arc<GrpcGateway>,
    pub metrics: Arc<MetricsStore>,
    pub config_routes: Vec<crate::config::RouteConfig>,
    pub middleware: Arc<MiddlewareChain>,
}

// ---------------------------------------------------------------------------
// Request handler
// ---------------------------------------------------------------------------

async fn handle_request(
    State(state): State<AppState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response {
    let start = std::time::Instant::now();
    let method_str = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());

    // Security: reject path traversal attempts (raw and percent-encoded).
    if crate::protocols::http::path_contains_traversal(&path) {
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

    // WebSocket upgrades must be handled before the body is consumed.
    if let TransportConfig::WebSocket(cfg) = &route.transport {
        use axum::extract::ws::WebSocketUpgrade;
        use axum::extract::FromRequestParts;

        let (mut parts, _body) = req.into_parts();
        match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
            Ok(upgrade) => {
                let backend_url = cfg.url.clone();
                let timeout_secs = cfg.timeout_secs;
                let latency_ms = start.elapsed().as_millis() as u64;
                state.metrics.record_request(&method_str, &path, matched_route, &backend_url, 101, latency_ms, None).await;
                return crate::gateway::ws::handle_websocket_upgrade(upgrade, backend_url, timeout_secs);
            }
            Err(_) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                state.metrics.record_request(&method_str, &path, matched_route, &cfg.url, 426, latency_ms, None).await;
                return (StatusCode::UPGRADE_REQUIRED, "WebSocket upgrade required").into_response();
            }
        }
    }

    // Extract headers before consuming the body (injection guard: drop
    // headers whose value contains non-visible ASCII).
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|val| (k.as_str().to_string(), val.to_string()))
        })
        .collect();

    // Extract body (capped at 10 MB — belt-and-suspenders alongside the layer).
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b.to_vec(),
        Err(_) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let target = transport_target(&route.transport);
            state.metrics.record_request(&method_str, &path, matched_route, &target, 413, latency_ms, None).await;
            return (StatusCode::PAYLOAD_TOO_LARGE, "Body too large").into_response();
        }
    };

    // Run auth and rate-limit middleware before any dispatch.
    let timeout_secs = transport_timeout(&route.transport);
    let core_req = crate::core::types::Request {
        data: body_bytes,
        metadata: crate::core::types::RequestMetadata {
            protocol: "http".to_string(),
            method: Some(method_str.clone()),
            path: Some(path.clone()),
            headers,
            timeout: Some(Duration::from_secs(timeout_secs)),
            remote_addr: Some(remote_addr.ip().to_string()),
        },
    };

    let core_req = match state.middleware.handle_request(core_req).await {
        Ok(r) => r,
        Err(crate::error::Error::Unauthorized(msg)) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let target = transport_target(&route.transport);
            state.metrics.record_request(&method_str, &path, matched_route, &target, 401, latency_ms, None).await;
            return (StatusCode::UNAUTHORIZED, msg).into_response();
        }
        Err(crate::error::Error::RateLimited) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let target = transport_target(&route.transport);
            let window_secs = transport_timeout(&route.transport); // reuse for Retry-After approximation
            let retry_after = state.middleware
                .rate_limit_window_secs()
                .unwrap_or(window_secs)
                .to_string();
            state.metrics.record_request(&method_str, &path, matched_route, &target, 429, latency_ms, None).await;
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::RETRY_AFTER, retry_after.as_str())],
                "Too many requests",
            ).into_response();
        }
        Err(e) => {
            tracing::error!("Middleware error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            let target = transport_target(&route.transport);
            state.metrics.record_request(&method_str, &path, matched_route, &target, 500, latency_ms, Some(e.to_string())).await;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let headers = core_req.metadata.headers;
    let body_bytes = core_req.data;

    // Dispatch on transport type.
    match &route.transport {
        TransportConfig::Http(cfg) => {
            let reqwest_method = match reqwest::Method::from_bytes(method_str.as_bytes()) {
                Ok(m) => m,
                Err(_) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(&method_str, &path, matched_route, &cfg.url, 405, latency_ms, None).await;
                    return (StatusCode::METHOD_NOT_ALLOWED, "Invalid method").into_response();
                }
            };

            // Strip the matched route prefix from the path when configured.
            let forward_path = if cfg.strip_prefix {
                let stripped = path.strip_prefix(route.path.as_str()).unwrap_or(&path);
                if stripped.is_empty() {
                    "/".to_string()
                } else if stripped.starts_with('/') {
                    stripped.to_string()
                } else {
                    format!("/{}", stripped)
                }
            } else {
                path.clone()
            };

            match state
                .http_gateway
                .proxy(reqwest_method, &cfg.url, &forward_path, query.as_deref(), &headers, body_bytes, cfg.timeout_secs)
                .await
            {
                Ok((status, resp_headers, resp_body)) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(&method_str, &path, matched_route, &cfg.url, status, latency_ms, None).await;
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
                    state.metrics.record_request(&method_str, &path, matched_route, &cfg.url, 502, latency_ms, Some(e.to_string())).await;
                    (StatusCode::BAD_GATEWAY, "Bad gateway").into_response()
                }
            }
        }

        TransportConfig::Zmq(cfg) => {
            handle_zmq(&state, &method_str, &path, &route, cfg, body_bytes, start).await
        }

        TransportConfig::GraphQL(cfg) => {
            handle_graphql(&state, &method_str, &path, &route, cfg, headers, body_bytes, start).await
        }

        TransportConfig::Grpc(cfg) => {
            handle_grpc(&state, &method_str, &path, &route, cfg, headers, body_bytes, start).await
        }

        // WebSocket is handled above before body extraction.
        TransportConfig::WebSocket(_) => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Transport helpers
// ---------------------------------------------------------------------------

/// Returns a display label for the upstream target (used in metrics).
fn transport_target(transport: &TransportConfig) -> String {
    match transport {
        TransportConfig::Http(cfg) => cfg.url.clone(),
        TransportConfig::Zmq(cfg) => cfg.address.clone(),
        TransportConfig::GraphQL(cfg) => cfg.url.clone(),
        TransportConfig::Grpc(cfg) => cfg.url.clone(),
        TransportConfig::WebSocket(cfg) => cfg.url.clone(),
    }
}

/// Returns the timeout for a transport config.
fn transport_timeout(transport: &TransportConfig) -> u64 {
    match transport {
        TransportConfig::Http(cfg) => cfg.timeout_secs,
        TransportConfig::Zmq(cfg) => cfg.timeout_secs,
        TransportConfig::GraphQL(cfg) => cfg.timeout_secs,
        TransportConfig::Grpc(cfg) => cfg.timeout_secs,
        TransportConfig::WebSocket(cfg) => cfg.timeout_secs,
    }
}

// ---------------------------------------------------------------------------
// ZMQ dispatch helper
// ---------------------------------------------------------------------------

async fn handle_zmq(
    state: &AppState,
    method_str: &str,
    path: &str,
    route: &crate::config::RouteConfig,
    cfg: &ZmqTransportConfig,
    body: Vec<u8>,
    start: std::time::Instant,
) -> Response {
    let matched_route = Some(route.path.clone());
    let gw = ZmqGateway::new();

    match cfg.pattern {
        ZmqPattern::ReqRep => {
            match gw.forward_req_rep(&cfg.address, body, cfg.timeout_secs).await {
                Ok(resp_body) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 200, latency_ms, None).await;
                    axum::response::Response::builder()
                        .status(200)
                        .header("content-type", "application/octet-stream")
                        .body(axum::body::Body::from(resp_body))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
                }
                Err(e) => {
                    tracing::error!("ZMQ REQ/REP error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 502, latency_ms, Some(e.to_string())).await;
                    (StatusCode::BAD_GATEWAY, format!("ZMQ error: {}", e)).into_response()
                }
            }
        }

        ZmqPattern::Push => {
            match gw.forward_push(&cfg.address, body).await {
                Ok(()) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 202, latency_ms, None).await;
                    StatusCode::ACCEPTED.into_response()
                }
                Err(e) => {
                    tracing::error!("ZMQ PUSH error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 502, latency_ms, Some(e.to_string())).await;
                    (StatusCode::BAD_GATEWAY, format!("ZMQ error: {}", e)).into_response()
                }
            }
        }

        ZmqPattern::PubSub => {
            match gw.forward_pub(&cfg.address, body, cfg.topic.as_deref()).await {
                Ok(()) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 202, latency_ms, None).await;
                    StatusCode::ACCEPTED.into_response()
                }
                Err(e) => {
                    tracing::error!("ZMQ PUB error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 502, latency_ms, Some(e.to_string())).await;
                    (StatusCode::BAD_GATEWAY, format!("ZMQ error: {}", e)).into_response()
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Admin middleware — applies the same auth + rate-limit chain to all /admin/*
// routes so they cannot be accessed without a valid Bearer token when auth is
// enabled. Without this, the admin dashboard would be publicly readable even
// when API-key authentication is enforced on the proxy routes.
// ---------------------------------------------------------------------------

async fn admin_middleware(
    State(state): State<AppState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: axum::middleware::Next,
) -> Response {
    // Build a minimal core request with the incoming headers so the existing
    // middleware chain (auth + rate-limit) can evaluate it without consuming
    // the original request body.
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|val| (k.as_str().to_string(), val.to_string()))
        })
        .collect();

    let core_req = crate::core::types::Request {
        data: vec![],
        metadata: crate::core::types::RequestMetadata {
            protocol: "http".to_string(),
            method: None,
            path: None,
            headers,
            timeout: None,
            remote_addr: Some(remote_addr.ip().to_string()),
        },
    };

    match state.middleware.handle_request(core_req).await {
        Ok(_) => next.run(req).await,
        Err(crate::error::Error::Unauthorized(msg)) => {
            (StatusCode::UNAUTHORIZED, msg).into_response()
        }
        Err(crate::error::Error::RateLimited) => {
            (StatusCode::TOO_MANY_REQUESTS, "Too many requests").into_response()
        }
        Err(e) => {
            tracing::error!("Admin middleware error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// GraphQL dispatch helper
// ---------------------------------------------------------------------------

async fn handle_graphql(
    state: &AppState,
    method_str: &str,
    path: &str,
    route: &crate::config::RouteConfig,
    cfg: &GraphQLTransportConfig,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    start: std::time::Instant,
) -> Response {
    use crate::protocols::Protocol;

    let matched_route = Some(route.path.clone());

    // Validate the request body via the GraphQL protocol encoder.
    let graphql_proto = crate::protocols::graphql::GraphQLProtocol::new(serde_json::Value::Null)
        .expect("GraphQLProtocol::new is infallible");
    let body = match graphql_proto.encode(body).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("GraphQL validation error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.url, 400, latency_ms, Some(e.to_string())).await;
            return (StatusCode::BAD_REQUEST, format!("GraphQL error: {}", e)).into_response();
        }
    };

    match state.graphql_gateway.proxy(&cfg.url, &headers, body, cfg.timeout_secs).await {
        Ok((status, resp_headers, resp_body)) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.url, status, latency_ms, None).await;
            let mut builder = axum::response::Response::builder().status(status);
            for (name, value) in resp_headers {
                builder = builder.header(name, value);
            }
            builder
                .body(axum::body::Body::from(resp_body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            tracing::error!("GraphQL proxy error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.url, 502, latency_ms, Some(e.to_string())).await;
            (StatusCode::BAD_GATEWAY, "Bad gateway").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// gRPC dispatch helper
// ---------------------------------------------------------------------------

async fn handle_grpc(
    state: &AppState,
    method_str: &str,
    path: &str,
    route: &crate::config::RouteConfig,
    cfg: &GrpcTransportConfig,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    start: std::time::Instant,
) -> Response {
    use crate::protocols::Protocol;

    let matched_route = Some(route.path.clone());

    // Wrap the raw payload in a gRPC length-prefixed frame.
    let grpc_proto = crate::protocols::grpc::GrpcProtocol::new(serde_json::Value::Null)
        .expect("GrpcProtocol::new is infallible");
    let framed_body = match grpc_proto.encode(body).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("gRPC encode error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.url, 400, latency_ms, Some(e.to_string())).await;
            return (StatusCode::BAD_REQUEST, format!("gRPC error: {}", e)).into_response();
        }
    };

    match state.grpc_gateway.proxy(&cfg.url, path, &headers, framed_body, cfg.timeout_secs).await {
        Ok((status, resp_headers, resp_body)) => {
            // Strip gRPC framing from the response body before returning.
            // Strip gRPC framing on successful responses. For non-2xx
            // responses the body is typically an HTTP-level error message
            // (not a gRPC frame), so pass it through as-is.
            let decoded_body = if status >= 200 && status < 300 {
                grpc_proto.decode(resp_body.clone()).await.unwrap_or(resp_body)
            } else {
                resp_body
            };
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.url, status, latency_ms, None).await;
            let mut builder = axum::response::Response::builder().status(status);
            for (name, value) in resp_headers {
                builder = builder.header(name, value);
            }
            builder
                .body(axum::body::Body::from(decoded_body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            tracing::error!("gRPC proxy error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.url, 502, latency_ms, Some(e.to_string())).await;
            (StatusCode::BAD_GATEWAY, "Bad gateway").into_response()
        }
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
    graphql_gateway: Arc<GraphQLGateway>,
    grpc_gateway: Arc<GrpcGateway>,
    metrics: Arc<MetricsStore>,
    config_routes: Vec<crate::config::RouteConfig>,
    listeners: Vec<ListenerConfig>,
    middleware: Arc<MiddlewareChain>,
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
            graphql_gateway: Arc::clone(&self.graphql_gateway),
            grpc_gateway: Arc::clone(&self.grpc_gateway),
            metrics: Arc::clone(&self.metrics),
            config_routes: self.config_routes.clone(),
            middleware: Arc::clone(&self.middleware),
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

        // Inbound listener tasks (one per entry in `listeners`).
        for listener_cfg in &self.listeners {
            match listener_cfg {
                ListenerConfig::ZmqPull(cfg) => {
                    let cfg = cfg.clone();
                    tracing::info!("Spawning ZMQ PULL listener: {} → {}", cfg.bind, cfg.forward_to);
                    tokio::spawn(async move {
                        crate::gateway::zmq::run_pull_listener(cfg).await;
                    });
                }
            }
        }

        let admin_router = crate::admin::router::build_admin_router()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                admin_middleware,
            ));

        let app = axum::Router::new()
            .merge(admin_router)
            .route("/{*path}", any(handle_request))
            .route("/", any(handle_request))
            .with_state(state)
            .layer(TraceLayer::new_for_http())
            .layer(TimeoutLayer::with_status_code(axum::http::StatusCode::GATEWAY_TIMEOUT, Duration::from_secs(30)))
            .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024));

        let shutdown = Arc::clone(&self.shutdown);
        let shutdown_signal = async move { shutdown.notified().await };

        // Use `into_make_service_with_connect_info` so that handlers can extract
        // the verified remote `SocketAddr` via `ConnectInfo<SocketAddr>`. This
        // provides a tamper-proof client identity for rate limiting that cannot
        // be spoofed by manipulating HTTP headers.
        tokio::spawn(async move {
            if let Err(e) = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
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

/// Creates a `DefaultGateway` from a `GatewayConfig`.
pub fn create_gateway(config: crate::config::GatewayConfig) -> Result<DefaultGateway> {
    let protocols = build_protocols(&config.protocols)?;
    let router = Arc::new(crate::core::Router::new(config.routes.clone()));
    let http_protocol = Arc::new(HttpProtocol::new(serde_json::Value::Null)?);
    let http_gateway = Arc::new(crate::gateway::http::HttpGateway::new(http_protocol));
    let graphql_protocol = Arc::new(GraphQLProtocol::new(serde_json::Value::Null)?);
    let graphql_gateway = Arc::new(GraphQLGateway::new(graphql_protocol));
    let grpc_protocol = Arc::new(GrpcProtocol::new(serde_json::Value::Null)?);
    let grpc_gateway = Arc::new(GrpcGateway::new(grpc_protocol));
    let metrics = Arc::new(MetricsStore::new());
    let middleware = Arc::new(build_middleware_chain(&config.middleware));

    Ok(DefaultGateway {
        host: config.host,
        port: config.port,
        protocols,
        router,
        http_gateway,
        graphql_gateway,
        grpc_gateway,
        metrics,
        config_routes: config.routes,
        listeners: config.listeners,
        middleware,
        shutdown: Arc::new(Notify::new()),
    })
}

/// Instantiates enabled protocols from their config descriptors.
fn build_protocols(configs: &[crate::config::ProtocolConfig]) -> Result<Vec<Arc<dyn Protocol>>> {
    configs
        .iter()
        .filter(|c| c.enabled)
        .map(|c| -> Result<Arc<dyn Protocol>> {
            match c.name.as_str() {
                "http"      => Ok(Arc::new(HttpProtocol::new(c.settings.clone())?)),
                "grpc"      => Ok(Arc::new(GrpcProtocol::new(c.settings.clone())?)),
                "graphql"   => Ok(Arc::new(GraphQLProtocol::new(c.settings.clone())?)),
                "mqtt"      => Ok(Arc::new(MqttProtocol::new(c.settings.clone())?)),
                "websocket" => Ok(Arc::new(WebSocketProtocol::new(c.settings.clone())?)),
                "zmq"       => Ok(Arc::new(crate::protocols::zmq::ZmqProtocol::new(c.settings.clone())?)),
                other => Err(crate::error::Error::Protocol(format!(
                    "Unsupported protocol: {}", other
                ))),
            }
        })
        .collect()
}

/// Builds the middleware chain from the typed middleware config section.
fn build_middleware_chain(config: &crate::config::MiddlewareSectionConfig) -> MiddlewareChain {
    let mut chain = MiddlewareChain::new();

    chain.add(Arc::new(AuthMiddleware::new(MiddlewareConfig {
        enabled: config.auth.enabled,
        settings: serde_json::json!({ "api_keys": config.auth.api_keys }),
    })));

    chain.add(Arc::new(RateLimitMiddleware::new(MiddlewareConfig {
        enabled: config.rate_limit.enabled,
        settings: serde_json::json!({
            "requests_per_window": config.rate_limit.requests_per_window,
            "window_secs": config.rate_limit.window_secs,
        }),
    })));

    chain
}
