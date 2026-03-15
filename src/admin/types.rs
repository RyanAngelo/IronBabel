use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogEntry {
    pub id: u64,
    pub timestamp_ms: u64,
    pub method: String,
    pub path: String,
    pub matched_route: Option<String>,
    pub status_code: u16,
    pub latency_ms: u64,
    pub upstream_target: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BucketPoint {
    pub timestamp_secs: u64,
    pub value: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetricsSummary {
    pub total_requests: u64,
    pub rps: f64,
    pub error_rate: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub status_code_counts: HashMap<String, u64>,
    pub requests_by_route: HashMap<String, u64>,
    pub rps_series: Vec<BucketPoint>,
    pub latency_series: Vec<BucketPoint>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub uptime_secs: u64,
    pub version: String,
    pub active_routes: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteInfo {
    pub path: String,
    pub transport_type: String,
    pub target: String,
    pub methods: Vec<String>,
    pub timeout_secs: u64,
    pub total_requests: u64,
    pub error_count: u64,
    pub avg_latency_ms: f64,
}
