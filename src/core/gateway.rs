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
use crate::protocols::{http::HttpProtocol, grpc::GrpcProtocol, graphql::GraphQLProtocol, mqtt::MqttProtocol, ws::WebSocketProtocol};
use super::Gateway;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AppState {
    router: Arc<crate::core::Router>,
    http_gateway: Arc<crate::gateway::http::HttpGateway>,
}

// ---------------------------------------------------------------------------
// Request handler
// ---------------------------------------------------------------------------

async fn handle_request(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let method_str = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());

    // Security: reject path traversal attempts.
    if path.contains("..") {
        return (StatusCode::BAD_REQUEST, "Invalid path").into_response();
    }

    let route = match state.router.route(&method_str, &path) {
        None => return (StatusCode::NOT_FOUND, "Not found").into_response(),
        Some(r) => r.clone(),
    };

    // Extract headers, dropping any with non-visible ASCII values (injection guard).
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|val| (k.as_str().to_string(), val.to_string()))
        })
        .collect();

    // Extract body (capped at 10MB — belt-and-suspenders alongside the middleware layer).
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b.to_vec(),
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "Body too large").into_response(),
    };

    // Parse method for reqwest.
    let reqwest_method = match reqwest::Method::from_bytes(method_str.as_bytes()) {
        Ok(m) => m,
        Err(_) => return (StatusCode::METHOD_NOT_ALLOWED, "Invalid method").into_response(),
    };

    let timeout = route.timeout_secs.unwrap_or(30);

    match state
        .http_gateway
        .proxy(
            reqwest_method,
            &route.target,
            &path,
            query.as_deref(),
            &headers,
            body_bytes,
            timeout,
        )
        .await
    {
        Ok((status, resp_headers, resp_body)) => {
            let mut builder = axum::response::Response::builder()
                .status(status);
            for (name, value) in resp_headers {
                builder = builder.header(name, value);
            }
            builder
                .body(axum::body::Body::from(resp_body))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            tracing::error!("Proxy error: {}", e);
            (StatusCode::BAD_GATEWAY, "Bad gateway").into_response()
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
}

impl GatewayConfig {
    pub fn from_config(config: crate::config::GatewayConfig) -> Result<Self> {
        let host = config.host.clone();
        let port = config.port;
        let routes = config.routes.clone();

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

        Ok(Self { host, port, protocols, routes })
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
    shutdown: Arc<Notify>,
}

#[async_trait]
impl Gateway for DefaultGateway {
    async fn start(&self) -> Result<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&addr).await.map_err(crate::error::Error::Io)?;

        tracing::info!("IronBabel gateway listening on {}", addr);

        let state = AppState {
            router: Arc::clone(&self.router),
            http_gateway: Arc::clone(&self.http_gateway),
        };

        let app = axum::Router::new()
            .route("/{*path}", any(handle_request))
            .route("/", any(handle_request))
            .with_state(state)
            .layer(TraceLayer::new_for_http())
            .layer(TimeoutLayer::with_status_code(axum::http::StatusCode::GATEWAY_TIMEOUT, Duration::from_secs(30)))
            .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024));

        let shutdown = Arc::clone(&self.shutdown);
        let shutdown_signal = async move {
            shutdown.notified().await;
        };

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
    let router = Arc::new(crate::core::Router::new(config.routes));
    let http_protocol = Arc::new(HttpProtocol::new(serde_json::Value::Null)?);
    let http_gateway = Arc::new(crate::gateway::http::HttpGateway::new(http_protocol));
    Ok(DefaultGateway {
        host: config.host,
        port: config.port,
        protocols: config.protocols,
        router,
        http_gateway,
        shutdown: Arc::new(Notify::new()),
    })
}
