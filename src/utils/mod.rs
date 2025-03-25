pub mod metrics;
pub mod logging;

pub use metrics::Metrics;
pub use logging::Logger;

pub trait MetricsCollector: Send + Sync {
    fn record_request(&self, protocol: &str, duration: std::time::Duration);
    fn record_error(&self, protocol: &str, error_type: &str);
}

pub trait Logging: Send + Sync {
    fn info(&self, message: &str);
    fn error(&self, message: &str);
    fn debug(&self, message: &str);
} 