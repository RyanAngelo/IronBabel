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

/// Initialize the logging system with default settings.
///
/// This is idempotent: if a global subscriber has already been installed (for
/// example by another `init()` call or a test harness), it is treated as a
/// no-op rather than a fatal error.
pub fn init() -> Result<()> {
    let mut filter = EnvFilter::from_default_env().add_directive(Level::INFO.into());
    // Built-in default directive; parse defensively so a future edit that makes
    // it invalid degrades gracefully instead of panicking.
    match "iron_babel=debug".parse() {
        Ok(directive) => filter = filter.add_directive(directive),
        Err(e) => eprintln!("WARN: ignoring invalid log directive: {}", e),
    }

    let result = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .pretty()
        .try_init();

    // `Err` from `try_init` means a subscriber is already installed — that is a
    // benign no-op, not a startup failure.
    if let Err(e) = result {
        tracing::debug!("logging already initialized: {}", e);
    }
    Ok(())
}