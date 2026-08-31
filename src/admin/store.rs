use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::admin::types::RequestLogEntry;

const RING_BUFFER_CAP: usize = 10_000;
const BUCKET_COUNT: usize = 60;
const LATENCY_BUFFER_CAP: usize = 10_000;

#[derive(Debug, Clone)]
pub struct TimeBucket {
    pub timestamp_secs: u64,
    pub request_count: u64,
    pub total_latency_ms: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct RouteStats {
    pub total_requests: u64,
    pub error_count: u64,
    pub total_latency_ms: u64,
}

pub struct MetricsStore {
    request_log: Arc<Mutex<VecDeque<RequestLogEntry>>>,
    total_requests: AtomicU64,
    total_errors: AtomicU64,
    route_stats: Arc<RwLock<HashMap<String, RouteStats>>>,
    time_buckets: Arc<Mutex<VecDeque<TimeBucket>>>,
    recent_latencies: Arc<Mutex<VecDeque<u64>>>,
    status_code_counts: Arc<RwLock<HashMap<u16, u64>>>,
    event_tx: broadcast::Sender<RequestLogEntry>,
    started_at: std::time::Instant,
    next_id: AtomicU64,
}

impl MetricsStore {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(1024);
        Self {
            request_log: Arc::new(Mutex::new(VecDeque::with_capacity(RING_BUFFER_CAP))),
            total_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            route_stats: Arc::new(RwLock::new(HashMap::new())),
            time_buckets: Arc::new(Mutex::new(VecDeque::with_capacity(BUCKET_COUNT + 1))),
            recent_latencies: Arc::new(Mutex::new(VecDeque::with_capacity(LATENCY_BUFFER_CAP))),
            status_code_counts: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            started_at: std::time::Instant::now(),
            next_id: AtomicU64::new(1),
        }
    }

    pub async fn record_request(
        &self,
        method: &str,
        path: &str,
        matched_route: Option<String>,
        upstream_target: &str,
        status_code: u16,
        latency_ms: u64,
        error: Option<String>,
    ) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let entry = RequestLogEntry {
            id,
            timestamp_ms,
            method: method.to_string(),
            path: path.to_string(),
            matched_route: matched_route.clone(),
            status_code,
            latency_ms,
            upstream_target: upstream_target.to_string(),
            error: error.clone(),
        };

        // Update totals
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        // Both client (4xx) and server (5xx) responses count as errors, plus any
        // transport-level error captured out of band.
        let is_error = status_code >= 400 || error.is_some();
        if is_error {
            self.total_errors.fetch_add(1, Ordering::Relaxed);
        }

        // Update status code counts
        {
            let mut counts = self.status_code_counts.write().await;
            *counts.entry(status_code).or_insert(0) += 1;
        }

        // Update route stats
        if let Some(ref route) = matched_route {
            let mut stats = self.route_stats.write().await;
            let s = stats.entry(route.clone()).or_default();
            s.total_requests += 1;
            if is_error {
                s.error_count += 1;
            }
            s.total_latency_ms += latency_ms;
        }

        // Update time buckets
        {
            let mut buckets = self.time_buckets.lock().await;
            let now_secs = timestamp_ms / 1000;
            if let Some(last) = buckets.back_mut() {
                if last.timestamp_secs == now_secs {
                    last.request_count += 1;
                    last.total_latency_ms += latency_ms;
                    if is_error {
                        last.error_count += 1;
                    }
                } else {
                    buckets.push_back(TimeBucket {
                        timestamp_secs: now_secs,
                        request_count: 1,
                        total_latency_ms: latency_ms,
                        error_count: if is_error { 1 } else { 0 },
                    });
                    while buckets.len() > BUCKET_COUNT {
                        buckets.pop_front();
                    }
                }
            } else {
                buckets.push_back(TimeBucket {
                    timestamp_secs: now_secs,
                    request_count: 1,
                    total_latency_ms: latency_ms,
                    error_count: if is_error { 1 } else { 0 },
                });
            }
        }

        // Update recent latencies
        {
            let mut latencies = self.recent_latencies.lock().await;
            if latencies.len() >= LATENCY_BUFFER_CAP {
                latencies.pop_front();
            }
            latencies.push_back(latency_ms);
        }

        // Update ring buffer
        {
            let mut log = self.request_log.lock().await;
            if log.len() >= RING_BUFFER_CAP {
                log.pop_front();
            }
            log.push_back(entry.clone());
        }

        // Fan-out to SSE subscribers (ignore error if no subscribers)
        let _ = self.event_tx.send(entry);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RequestLogEntry> {
        self.event_tx.subscribe()
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::Relaxed)
    }

    pub fn total_errors(&self) -> u64 {
        self.total_errors.load(Ordering::Relaxed)
    }

    pub async fn recent_requests(&self, n: usize) -> Vec<RequestLogEntry> {
        let log = self.request_log.lock().await;
        log.iter().rev().take(n).cloned().collect()
    }

    pub async fn get_route_stats(&self) -> HashMap<String, RouteStats> {
        self.route_stats.read().await.clone()
    }

    pub async fn get_status_code_counts(&self) -> HashMap<u16, u64> {
        self.status_code_counts.read().await.clone()
    }

    pub async fn get_percentiles(&self) -> (f64, f64, f64) {
        let latencies = self.recent_latencies.lock().await;
        if latencies.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut sorted: Vec<u64> = latencies.iter().cloned().collect();
        sorted.sort_unstable();
        (
            percentile(&sorted, 50.0),
            percentile(&sorted, 95.0),
            percentile(&sorted, 99.0),
        )
    }

    pub async fn get_time_buckets(&self) -> Vec<TimeBucket> {
        self.time_buckets.lock().await.iter().cloned().collect()
    }

    /// Compute RPS from the most recent 5-second window of buckets.
    pub async fn compute_rps(&self) -> f64 {
        let buckets = self.time_buckets.lock().await;
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let window = 5u64;
        // Strictly greater than `now - window`: buckets are one second wide, so
        // `>=` would span `window + 1` of them (now-5 ..= now) and inflate the
        // rate by ~20% once the divisor below settles at the full window.
        let count: u64 = buckets
            .iter()
            .filter(|b| b.timestamp_secs > now_secs.saturating_sub(window))
            .map(|b| b.request_count)
            .sum();
        // Divide by the actual observed span, not always the full window: right
        // after startup fewer than `window` seconds of data exist, so dividing
        // by the full window would systematically under-report RPS.
        let divisor = window.min(self.uptime_secs().max(1)).max(1);
        count as f64 / divisor as f64
    }

    /// Called every second by the background tick task to keep the bucket timeline current.
    pub async fn tick(&self) {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut buckets = self.time_buckets.lock().await;
        if buckets.back().map(|b| b.timestamp_secs != now_secs).unwrap_or(true) {
            buckets.push_back(TimeBucket {
                timestamp_secs: now_secs,
                request_count: 0,
                total_latency_ms: 0,
                error_count: 0,
            });
            while buckets.len() > BUCKET_COUNT {
                buckets.pop_front();
            }
        }
    }
}

impl Default for MetricsStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Nearest-rank percentile over an ascending-sorted slice.
///
/// `p` is in the range `[0, 100]`. The rank is `ceil(p/100 * n)` (1-based),
/// clamped to the slice bounds. For example the median of `[10, 1000]` is `10`
/// (the lower of the two), not `1000`.
fn percentile(sorted_asc: &[u64], p: f64) -> f64 {
    if sorted_asc.is_empty() {
        return 0.0;
    }
    let n = sorted_asc.len();
    let rank = ((p / 100.0) * n as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted_asc[idx] as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `counts` into the bucket timeline as one-second buckets ending at
    /// the current second, then reads back the computed rate.
    async fn rps_over(counts: &[u64]) -> f64 {
        let store = MetricsStore::new();
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        {
            let mut buckets = store.time_buckets.lock().await;
            let oldest = counts.len() as u64 - 1;
            for (i, count) in counts.iter().enumerate() {
                buckets.push_back(TimeBucket {
                    timestamp_secs: now_secs - oldest + i as u64,
                    request_count: *count,
                    total_latency_ms: 0,
                    error_count: 0,
                });
            }
        }

        store.compute_rps().await
    }

    #[tokio::test]
    async fn rps_sums_exactly_window_buckets() {
        // Regression: the filter used `>= now - window`, which spans window + 1
        // one-second buckets while the divisor is the window itself, inflating a
        // steady rate by ~20%.
        //
        // The store is brand new here, so `uptime_secs()` is 0 and the divisor
        // collapses to 1 — which is what makes the raw bucket count observable.
        // One request in each of six consecutive seconds must count as 5, not 6.
        let rps = rps_over(&[1, 1, 1, 1, 1, 1]).await;
        assert_eq!(rps, 5.0, "window must cover exactly 5 one-second buckets");
    }

    #[tokio::test]
    async fn rps_excludes_the_bucket_at_the_window_edge() {
        // The 999 sits at `now - 5`, one second beyond the window.
        let rps = rps_over(&[999, 0, 0, 0, 0, 0]).await;
        assert_eq!(rps, 0.0);
    }

    #[test]
    fn percentile_empty_is_zero() {
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn percentile_median_picks_lower_of_two() {
        // Regression: previously p50 of [10, 1000] returned 1000 (the max).
        assert_eq!(percentile(&[10, 1000], 50.0), 10.0);
    }

    #[test]
    fn percentile_single_element() {
        assert_eq!(percentile(&[42], 50.0), 42.0);
        assert_eq!(percentile(&[42], 95.0), 42.0);
        assert_eq!(percentile(&[42], 99.0), 42.0);
    }

    #[test]
    fn percentile_known_distribution() {
        let data: Vec<u64> = (1..=100).collect(); // 1..=100 ascending
        assert_eq!(percentile(&data, 50.0), 50.0);
        assert_eq!(percentile(&data, 95.0), 95.0);
        assert_eq!(percentile(&data, 99.0), 99.0);
        assert_eq!(percentile(&data, 100.0), 100.0);
    }

    #[test]
    fn percentile_never_out_of_bounds() {
        // p just over a boundary must still clamp to the last element.
        assert_eq!(percentile(&[1, 2, 3], 100.0), 3.0);
    }

    #[tokio::test]
    async fn counts_4xx_as_errors() {
        let store = MetricsStore::new();
        store
            .record_request("GET", "/x", Some("/x".to_string()), "u", 404, 1, None)
            .await;
        store
            .record_request("GET", "/y", Some("/y".to_string()), "u", 200, 1, None)
            .await;
        assert_eq!(store.total_requests(), 2);
        assert_eq!(store.total_errors(), 1);
    }

    #[tokio::test]
    async fn get_percentiles_uses_nearest_rank() {
        let store = MetricsStore::new();
        store
            .record_request("GET", "/x", None, "u", 200, 10, None)
            .await;
        store
            .record_request("GET", "/y", None, "u", 200, 1000, None)
            .await;
        let (p50, _p95, _p99) = store.get_percentiles().await;
        assert_eq!(p50, 10.0);
    }
}
