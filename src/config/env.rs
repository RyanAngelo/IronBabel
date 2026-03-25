use super::GatewayConfig;

/// Returns the config file path to load.
/// Checks `IRON_BABEL_CONFIG` first; falls back to `"config/gateway.yaml"`.
pub fn config_file_path() -> String {
    std::env::var("IRON_BABEL_CONFIG")
        .unwrap_or_else(|_| "config/gateway.yaml".to_string())
}

/// Applies environment variable overrides on top of a loaded `GatewayConfig`.
///
/// | Variable            | Overrides         |
/// |---------------------|-------------------|
/// | `IRON_BABEL_PORT`   | `config.port`     |
/// | `IRON_BABEL_HOST`   | `config.host`     |
pub fn apply_env_overrides(config: &mut GatewayConfig) {
    if let Ok(val) = std::env::var("IRON_BABEL_PORT") {
        match val.parse::<u16>() {
            Ok(port) => config.port = port,
            Err(_) => tracing::warn!(
                "IRON_BABEL_PORT=\"{}\" is not a valid port number; ignoring",
                val
            ),
        }
    }

    if let Ok(val) = std::env::var("IRON_BABEL_HOST") {
        config.host = val;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProtocolConfig;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn base_config() -> GatewayConfig {
        GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
            protocols: vec![ProtocolConfig {
                name: "http".to_string(),
                enabled: true,
                settings: json!({}),
            }],
            routes: vec![],
            listeners: vec![],
            middleware: Default::default(),
        }
    }

    #[test]
    fn env_overrides_are_still_subject_to_validation() {
        let _guard = env_lock().lock().unwrap();
        let original_port = std::env::var("IRON_BABEL_PORT").ok();

        std::env::set_var("IRON_BABEL_PORT", "0");

        let mut config = base_config();
        apply_env_overrides(&mut config);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("port must be between 1 and 65535"));

        if let Some(port) = original_port {
            std::env::set_var("IRON_BABEL_PORT", port);
        } else {
            std::env::remove_var("IRON_BABEL_PORT");
        }
    }
}
