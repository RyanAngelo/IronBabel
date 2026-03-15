pub mod logging;

pub use logging::Logger;

pub trait Logging: Send + Sync {
    fn info(&self, message: &str);
    fn error(&self, message: &str);
    fn debug(&self, message: &str);
}
