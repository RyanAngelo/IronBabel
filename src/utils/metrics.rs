use std::time::Duration;
use super::MetricsCollector;

pub struct Metrics;

impl MetricsCollector for Metrics {
    fn record_request(&self, _protocol: &str, _duration: Duration) {
        todo!("Implement metrics recording")
    }

    fn record_error(&self, _protocol: &str, _error_type: &str) {
        todo!("Implement error recording")
    }
} 