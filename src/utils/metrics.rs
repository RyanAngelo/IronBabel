use std::time::Duration;
use super::MetricsCollector;

pub struct Metrics;

impl MetricsCollector for Metrics {
    fn record_request(&self, _protocol: &str, _duration: Duration) {
        // Legacy stub — real metrics go through MetricsStore in the admin module.
    }

    fn record_error(&self, _protocol: &str, _error_type: &str) {
        // Legacy stub — real metrics go through MetricsStore in the admin module.
    }
}
