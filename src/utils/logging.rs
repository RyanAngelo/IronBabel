use crate::error::Result;
use super::Logging;
use tracing::Level;
use tracing_subscriber::{fmt, EnvFilter};

pub struct Logger;

impl Logging for Logger {
    fn info(&self, message: &str) {
        println!("INFO: {}", message);
    }

    fn error(&self, message: &str) {
        eprintln!("ERROR: {}", message);
    }

    fn debug(&self, message: &str) {
        println!("DEBUG: {}", message);
    }
}

/// Initialize the logging system with default settings
pub fn init() -> Result<()> {
    // Set up the subscriber with a default format and filter
    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(Level::INFO.into())
                .add_directive("iron_babel=debug".parse().unwrap()),
        )
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .pretty()
        .try_init();

    match subscriber {
        Ok(_) => Ok(()),
        Err(e) => Err(crate::error::Error::Unknown(e.to_string())),
    }
} 