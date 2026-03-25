use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use crate::{error::Result, protocols::Protocol};
use crate::protocols::{
    http::HttpProtocol, grpc::GrpcProtocol, graphql::GraphQLProtocol, mqtt::MqttProtocol,
    ws::WebSocketProtocol,
};
use crate::gateway::graphql::GraphQLGateway;
use crate::gateway::grpc::GrpcGateway;
use crate::gateway::mqtt::MqttGateway;
use crate::gateway::amqp::AmqpGateway;
use crate::admin::config_store::AdminConfigStore;
use crate::admin::store::MetricsStore;
use crate::config::{
    AmqpTransportConfig, GrpcTransportConfig, GraphQLTransportConfig, ListenerConfig, MqttTransportConfig,
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
    pub mqtt_gateway: Arc<MqttGateway>,
    pub amqp_gateway: Arc<AmqpGateway>,
    pub metrics: Arc<MetricsStore>,
    pub config_routes: Vec<crate::config::RouteConfig>,
    pub middleware: Arc<MiddlewareChain>,
    pub config_store: Arc<AdminConfigStore>,
}

#[derive(Default)]
struct RuntimeHandles {
    running: bool,
    server_handle: Option<JoinHandle<()>>,
    metrics_handle: Option<JoinHandle<()>>,
    listener_handles: Vec<JoinHandle<()>>,
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
        return finalize_into_response(&state, (StatusCode::BAD_REQUEST, "Invalid path")).await;
    }

    let route = match state.router.route(&method_str, &path) {
        None => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(&method_str, &path, None, "", 404, latency_ms, None).await;
            return finalize_into_response(&state, (StatusCode::NOT_FOUND, "Not found")).await;
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
                return finalize_response(
                    &state,
                    crate::gateway::ws::handle_websocket_upgrade(upgrade, backend_url, timeout_secs),
                )
                .await;
            }
            Err(_) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                state.metrics.record_request(&method_str, &path, matched_route, &cfg.url, 426, latency_ms, None).await;
                return finalize_into_response(
                    &state,
                    (StatusCode::UPGRADE_REQUIRED, "WebSocket upgrade required"),
                )
                .await;
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
            return finalize_into_response(&state, (StatusCode::PAYLOAD_TOO_LARGE, "Body too large"))
                .await;
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
            return finalize_into_response(&state, (StatusCode::UNAUTHORIZED, msg)).await;
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
            return finalize_into_response(&state, (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::RETRY_AFTER, retry_after.as_str())],
                "Too many requests",
            ))
            .await;
        }
        Err(e) => {
            tracing::error!("Middleware error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            let target = transport_target(&route.transport);
            state.metrics.record_request(&method_str, &path, matched_route, &target, 500, latency_ms, Some(e.to_string())).await;
            return finalize_into_response(&state, StatusCode::INTERNAL_SERVER_ERROR).await;
        }
    };

    let remote_ip = core_req.metadata.remote_addr.as_deref().unwrap_or_default();
    let mut headers = core_req.metadata.headers;
    set_forwarded_for(&mut headers, remote_ip);
    let body_bytes = core_req.data;

    // Dispatch on transport type.
    match &route.transport {
        TransportConfig::Http(cfg) => {
            let reqwest_method = match reqwest::Method::from_bytes(method_str.as_bytes()) {
                Ok(m) => m,
                Err(_) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(&method_str, &path, matched_route, &cfg.url, 405, latency_ms, None).await;
                    return finalize_into_response(&state, (StatusCode::METHOD_NOT_ALLOWED, "Invalid method"))
                        .await;
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
                    finalize_response(
                        &state,
                        builder
                        .body(axum::body::Body::from(resp_body))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
                    )
                    .await
                }
                Err(e) => {
                    tracing::error!("Proxy error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(&method_str, &path, matched_route, &cfg.url, 502, latency_ms, Some(e.to_string())).await;
                    finalize_into_response(&state, (StatusCode::BAD_GATEWAY, "Bad gateway")).await
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

        TransportConfig::Mqtt(cfg) => {
            handle_mqtt(&state, &method_str, &path, &route, cfg, body_bytes, start).await
        }

        TransportConfig::Amqp(cfg) => {
            handle_amqp(&state, &method_str, &path, &route, cfg, body_bytes, start).await
        }

        // WebSocket is handled above before body extraction.
        TransportConfig::WebSocket(_) => unreachable!(),
    }
}

fn set_forwarded_for(headers: &mut Vec<(String, String)>, remote_addr: &str) {
    if remote_addr.is_empty() {
        return;
    }

    if let Some((_, value)) = headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-forwarded-for"))
    {
        *value = remote_addr.to_string();
    } else {
        headers.push(("x-forwarded-for".to_string(), remote_addr.to_string()));
    }
}

async fn finalize_into_response<T: IntoResponse>(state: &AppState, response: T) -> Response {
    finalize_response(state, response.into_response()).await
}

async fn finalize_response(state: &AppState, response: Response) -> Response {
    let (mut parts, body) = response.into_parts();
    let headers = parts
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect();

    let core_response = crate::core::types::Response {
        data: vec![],
        metadata: crate::core::types::ResponseMetadata {
            status_code: Some(parts.status.as_u16()),
            headers,
            duration: None,
        },
    };

    match state.middleware.handle_response(core_response).await {
        Ok(core_response) => {
            if let Some(status_code) = core_response.metadata.status_code {
                parts.status = StatusCode::from_u16(status_code)
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            }

            parts.headers.clear();
            for (name, value) in core_response.metadata.headers {
                let Ok(name) = HeaderName::try_from(name) else {
                    continue;
                };
                let Ok(value) = HeaderValue::from_str(&value) else {
                    continue;
                };
                parts.headers.append(name, value);
            }

            Response::from_parts(parts, body)
        }
        Err(e) => {
            tracing::error!("Response middleware error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
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
        TransportConfig::Mqtt(cfg) => cfg.broker_url.clone(),
        TransportConfig::Amqp(cfg) => cfg.broker_url.clone(),
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
        TransportConfig::Mqtt(cfg) => cfg.timeout_secs,
        TransportConfig::Amqp(cfg) => cfg.timeout_secs,
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
                    finalize_response(
                        state,
                        axum::response::Response::builder()
                        .status(200)
                        .header("content-type", "application/octet-stream")
                        .body(axum::body::Body::from(resp_body))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
                    )
                    .await
                }
                Err(e) => {
                    tracing::error!("ZMQ REQ/REP error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 502, latency_ms, Some(e.to_string())).await;
                    finalize_into_response(
                        state,
                        (StatusCode::BAD_GATEWAY, format!("ZMQ error: {}", e)),
                    )
                    .await
                }
            }
        }

        ZmqPattern::Push => {
            match gw.forward_push(&cfg.address, body).await {
                Ok(()) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 202, latency_ms, None).await;
                    finalize_into_response(state, StatusCode::ACCEPTED).await
                }
                Err(e) => {
                    tracing::error!("ZMQ PUSH error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 502, latency_ms, Some(e.to_string())).await;
                    finalize_into_response(
                        state,
                        (StatusCode::BAD_GATEWAY, format!("ZMQ error: {}", e)),
                    )
                    .await
                }
            }
        }

        ZmqPattern::PubSub => {
            match gw.forward_pub(&cfg.address, body, cfg.topic.as_deref()).await {
                Ok(()) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 202, latency_ms, None).await;
                    finalize_into_response(state, StatusCode::ACCEPTED).await
                }
                Err(e) => {
                    tracing::error!("ZMQ PUB error: {}", e);
                    let latency_ms = start.elapsed().as_millis() as u64;
                    state.metrics.record_request(method_str, path, matched_route, &cfg.address, 502, latency_ms, Some(e.to_string())).await;
                    finalize_into_response(
                        state,
                        (StatusCode::BAD_GATEWAY, format!("ZMQ error: {}", e)),
                    )
                    .await
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
        Ok(_) => finalize_response(&state, next.run(req).await).await,
        Err(crate::error::Error::Unauthorized(msg)) => {
            finalize_into_response(&state, (StatusCode::UNAUTHORIZED, msg)).await
        }
        Err(crate::error::Error::RateLimited) => {
            finalize_into_response(&state, (StatusCode::TOO_MANY_REQUESTS, "Too many requests"))
                .await
        }
        Err(e) => {
            tracing::error!("Admin middleware error: {}", e);
            finalize_into_response(&state, StatusCode::INTERNAL_SERVER_ERROR).await
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
            return finalize_into_response(
                state,
                (StatusCode::BAD_REQUEST, format!("GraphQL error: {}", e)),
            )
            .await;
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
            finalize_response(
                state,
                builder
                .body(axum::body::Body::from(resp_body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            )
            .await
        }
        Err(e) => {
            tracing::error!("GraphQL proxy error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.url, 502, latency_ms, Some(e.to_string())).await;
            finalize_into_response(state, (StatusCode::BAD_GATEWAY, "Bad gateway")).await
        }
    }
}

async fn handle_mqtt(
    state: &AppState,
    method_str: &str,
    path: &str,
    route: &crate::config::RouteConfig,
    cfg: &MqttTransportConfig,
    body: Vec<u8>,
    start: std::time::Instant,
) -> Response {
    let matched_route = Some(route.path.clone());

    match state.mqtt_gateway.publish(cfg, body).await {
        Ok(()) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.broker_url, 202, latency_ms, None).await;
            finalize_into_response(state, StatusCode::ACCEPTED).await
        }
        Err(e) => {
            tracing::error!("MQTT publish error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.broker_url, 502, latency_ms, Some(e.to_string())).await;
            finalize_into_response(
                state,
                (StatusCode::BAD_GATEWAY, format!("MQTT error: {}", e)),
            )
            .await
        }
    }
}

async fn handle_amqp(
    state: &AppState,
    method_str: &str,
    path: &str,
    route: &crate::config::RouteConfig,
    cfg: &AmqpTransportConfig,
    body: Vec<u8>,
    start: std::time::Instant,
) -> Response {
    let matched_route = Some(route.path.clone());

    match state.amqp_gateway.publish(cfg, body).await {
        Ok(()) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.broker_url, 202, latency_ms, None).await;
            finalize_into_response(state, StatusCode::ACCEPTED).await
        }
        Err(e) => {
            tracing::error!("AMQP publish error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.broker_url, 502, latency_ms, Some(e.to_string())).await;
            finalize_into_response(
                state,
                (StatusCode::BAD_GATEWAY, format!("AMQP error: {}", e)),
            )
            .await
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
            return finalize_into_response(
                state,
                (StatusCode::BAD_REQUEST, format!("gRPC error: {}", e)),
            )
            .await;
        }
    };

    match state.grpc_gateway.proxy(&cfg.url, path, &headers, framed_body, cfg.timeout_secs).await {
        Ok((status, resp_headers, resp_body)) => {
            // Strip gRPC framing from the response body before returning.
            // Strip gRPC framing on successful responses. For non-2xx
            // responses the body is typically an HTTP-level error message
            // (not a gRPC frame), so pass it through as-is.
            let decoded_body = if (200..300).contains(&status) {
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
            finalize_response(
                state,
                builder
                .body(axum::body::Body::from(decoded_body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            )
            .await
        }
        Err(e) => {
            tracing::error!("gRPC proxy error: {}", e);
            let latency_ms = start.elapsed().as_millis() as u64;
            state.metrics.record_request(method_str, path, matched_route, &cfg.url, 502, latency_ms, Some(e.to_string())).await;
            finalize_into_response(state, (StatusCode::BAD_GATEWAY, "Bad gateway")).await
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
    mqtt_gateway: Arc<MqttGateway>,
    amqp_gateway: Arc<AmqpGateway>,
    metrics: Arc<MetricsStore>,
    config_routes: Vec<crate::config::RouteConfig>,
    listeners: Vec<ListenerConfig>,
    middleware: Arc<MiddlewareChain>,
    config_store: Arc<AdminConfigStore>,
    shutdown: Arc<Notify>,
    runtime: Mutex<RuntimeHandles>,
}

#[async_trait]
impl Gateway for DefaultGateway {
    async fn start(&self) -> Result<()> {
        let mut runtime = self.runtime.lock().await;
        if runtime.running {
            return Err(crate::error::Error::Unknown(
                "Gateway is already running".to_string(),
            ));
        }

        let addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&addr).await.map_err(crate::error::Error::Io)?;

        tracing::info!("IronBabel gateway listening on {}", addr);
        tracing::info!("Admin dashboard available at http://{}/admin/", addr);

        let state = AppState {
            router: Arc::clone(&self.router),
            http_gateway: Arc::clone(&self.http_gateway),
            graphql_gateway: Arc::clone(&self.graphql_gateway),
            grpc_gateway: Arc::clone(&self.grpc_gateway),
            mqtt_gateway: Arc::clone(&self.mqtt_gateway),
            amqp_gateway: Arc::clone(&self.amqp_gateway),
            metrics: Arc::clone(&self.metrics),
            config_routes: self.config_routes.clone(),
            middleware: Arc::clone(&self.middleware),
            config_store: Arc::clone(&self.config_store),
        };

        // 1-second tick task for admin dashboard time buckets.
        let metrics_tick = Arc::clone(&self.metrics);
        let metrics_shutdown = Arc::clone(&self.shutdown);
        runtime.metrics_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = metrics_shutdown.notified() => break,
                    _ = interval.tick() => metrics_tick.tick().await,
                }
            }
        }));

        // Inbound listener tasks (one per entry in `listeners`).
        for listener_cfg in &self.listeners {
            match listener_cfg {
                ListenerConfig::ZmqPull(cfg) => {
                    let cfg = cfg.clone();
                    let listener_shutdown = Arc::clone(&self.shutdown);
                    tracing::info!("Spawning ZMQ PULL listener: {} → {}", cfg.bind, cfg.forward_to);
                    runtime.listener_handles.push(tokio::spawn(async move {
                        crate::gateway::zmq::run_pull_listener(cfg, listener_shutdown).await;
                    }));
                }
                ListenerConfig::MqttSub(cfg) => {
                    let cfg = cfg.clone();
                    let listener_shutdown = Arc::clone(&self.shutdown);
                    tracing::info!(
                        "Spawning MQTT subscriber listener: {} topics={:?} → {}",
                        cfg.broker_url,
                        cfg.topics,
                        cfg.forward_to
                    );
                    runtime.listener_handles.push(tokio::spawn(async move {
                        crate::gateway::mqtt::run_sub_listener(cfg, listener_shutdown).await;
                    }));
                }
                ListenerConfig::AmqpConsume(cfg) => {
                    let cfg = cfg.clone();
                    let listener_shutdown = Arc::clone(&self.shutdown);
                    tracing::info!(
                        "Spawning AMQP consumer listener: {} queue={} → {}",
                        cfg.broker_url,
                        cfg.queue,
                        cfg.forward_to
                    );
                    runtime.listener_handles.push(tokio::spawn(async move {
                        crate::gateway::amqp::run_consumer_listener(cfg, listener_shutdown).await;
                    }));
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
        runtime.server_handle = Some(tokio::spawn(async move {
            if let Err(e) = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_signal)
            .await
            {
                tracing::error!("Gateway server error: {}", e);
            }
        }));

        runtime.running = true;

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.shutdown.notify_waiters();

        let (server_handle, metrics_handle, listener_handles) = {
            let mut runtime = self.runtime.lock().await;
            if !runtime.running {
                return Ok(());
            }

            runtime.running = false;
            (
                runtime.server_handle.take(),
                runtime.metrics_handle.take(),
                std::mem::take(&mut runtime.listener_handles),
            )
        };

        if let Some(handle) = server_handle {
            let _ = handle.await;
        }
        if let Some(handle) = metrics_handle {
            let _ = handle.await;
        }
        for handle in listener_handles {
            let _ = handle.await;
        }

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
    let admin_config = Arc::new(AdminConfigStore::new(config.clone()));
    let protocols = build_protocols(&config.protocols)?;
    let router = Arc::new(crate::core::Router::new(config.routes.clone()));
    let http_protocol = Arc::new(HttpProtocol::new(serde_json::Value::Null)?);
    let http_gateway = Arc::new(crate::gateway::http::HttpGateway::new(http_protocol));
    let graphql_protocol = Arc::new(GraphQLProtocol::new(serde_json::Value::Null)?);
    let graphql_gateway = Arc::new(GraphQLGateway::new(graphql_protocol));
    let grpc_protocol = Arc::new(GrpcProtocol::new(serde_json::Value::Null)?);
    let grpc_gateway = Arc::new(GrpcGateway::new(grpc_protocol));
    let mqtt_gateway = Arc::new(MqttGateway::new());
    let amqp_gateway = Arc::new(AmqpGateway::new());
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
        mqtt_gateway,
        amqp_gateway,
        metrics,
        config_routes: config.routes,
        listeners: config.listeners,
        middleware,
        config_store: admin_config,
        shutdown: Arc::new(Notify::new()),
        runtime: Mutex::new(RuntimeHandles::default()),
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
                "amqp"      => Ok(Arc::new(crate::protocols::amqp::AmqpProtocol::new(c.settings.clone())?)),
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

    chain.add(Arc::new(crate::core::middleware::LoggingMiddleware::new(
        MiddlewareConfig {
            enabled: config.logging.enabled,
            settings: serde_json::json!({}),
        },
    )));

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
