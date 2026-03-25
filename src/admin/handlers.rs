use std::collections::HashMap;
use std::convert::Infallible;

use axum::{
    extract::{Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html,
    },
    Json,
};
use futures::Stream;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

use crate::admin::types::{
    BucketPoint, HealthResponse, MetricsSummary, RequestLogEntry, RouteInfo,
};
use crate::config::TransportConfig;
use crate::core::gateway::AppState;

pub async fn admin_index() -> Html<&'static str> {
    Html(crate::admin::assets::DASHBOARD_HTML)
}

pub async fn admin_health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        uptime_secs: state.metrics.uptime_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        active_routes: state.config_routes.len(),
    })
}

pub async fn admin_metrics(State(state): State<AppState>) -> Json<MetricsSummary> {
    let total_requests = state.metrics.total_requests();
    let total_errors = state.metrics.total_errors();
    let rps = state.metrics.compute_rps().await;
    let error_rate = if total_requests > 0 {
        total_errors as f64 / total_requests as f64
    } else {
        0.0
    };
    let (p50, p95, p99) = state.metrics.get_percentiles().await;

    let raw_status_counts = state.metrics.get_status_code_counts().await;
    let status_code_counts: HashMap<String, u64> = raw_status_counts
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    let route_stats = state.metrics.get_route_stats().await;
    let requests_by_route: HashMap<String, u64> = route_stats
        .iter()
        .map(|(k, v)| (k.clone(), v.total_requests))
        .collect();

    let buckets = state.metrics.get_time_buckets().await;
    let rps_series: Vec<BucketPoint> = buckets
        .iter()
        .map(|b| BucketPoint {
            timestamp_secs: b.timestamp_secs,
            value: b.request_count as f64,
        })
        .collect();
    let latency_series: Vec<BucketPoint> = buckets
        .iter()
        .map(|b| BucketPoint {
            timestamp_secs: b.timestamp_secs,
            value: if b.request_count > 0 {
                b.total_latency_ms as f64 / b.request_count as f64
            } else {
                0.0
            },
        })
        .collect();

    Json(MetricsSummary {
        total_requests,
        rps,
        error_rate,
        p50_latency_ms: p50,
        p95_latency_ms: p95,
        p99_latency_ms: p99,
        status_code_counts,
        requests_by_route,
        rps_series,
        latency_series,
    })
}

pub async fn admin_routes(State(state): State<AppState>) -> Json<Vec<RouteInfo>> {
    let route_stats = state.metrics.get_route_stats().await;
    let routes: Vec<RouteInfo> = state
        .config_routes
        .iter()
        .map(|r| {
            let stats = route_stats.get(&r.path).cloned().unwrap_or_default();
            let avg_latency_ms = if stats.total_requests > 0 {
                stats.total_latency_ms as f64 / stats.total_requests as f64
            } else {
                0.0
            };
            let (transport_type, target, timeout_secs) = match &r.transport {
                TransportConfig::Http(cfg) => {
                    ("http".to_string(), cfg.url.clone(), cfg.timeout_secs)
                }
                TransportConfig::Zmq(cfg) => {
                    ("zmq".to_string(), cfg.address.clone(), cfg.timeout_secs)
                }
                TransportConfig::GraphQL(cfg) => {
                    ("graphql".to_string(), cfg.url.clone(), cfg.timeout_secs)
                }
                TransportConfig::Grpc(cfg) => {
                    ("grpc".to_string(), cfg.url.clone(), cfg.timeout_secs)
                }
                TransportConfig::WebSocket(cfg) => {
                    ("websocket".to_string(), cfg.url.clone(), cfg.timeout_secs)
                }
                TransportConfig::Mqtt(cfg) => {
                    ("mqtt".to_string(), cfg.broker_url.clone(), cfg.timeout_secs)
                }
            };
            RouteInfo {
                path: r.path.clone(),
                transport_type,
                target,
                methods: r.methods.clone(),
                timeout_secs,
                total_requests: stats.total_requests,
                error_count: stats.error_count,
                avg_latency_ms,
            }
        })
        .collect();
    Json(routes)
}

#[derive(serde::Deserialize)]
pub struct RecentQuery {
    pub n: Option<usize>,
}

pub async fn admin_recent_requests(
    State(state): State<AppState>,
    Query(params): Query<RecentQuery>,
) -> Json<Vec<RequestLogEntry>> {
    let n = params.n.unwrap_or(50).min(500);
    Json(state.metrics.recent_requests(n).await)
}

pub async fn admin_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.metrics.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().and_then(|entry| {
            serde_json::to_string(&entry)
                .ok()
                .map(|json| Ok(Event::default().data(json)))
        })
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
