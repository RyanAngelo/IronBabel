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

