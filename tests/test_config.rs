use iron_babel::config::{GatewayConfig, ProtocolConfig};
use serde_json::json;

pub fn create_test_config() -> GatewayConfig {
    GatewayConfig {
        port: 8080,
        host: "127.0.0.1".to_string(),
        protocols: vec![
            ProtocolConfig {
                name: "http".to_string(),
                enabled: true,
                settings: json!({
                    "timeout": 5000,
                    "max_connections": 100
                }),
            },
            ProtocolConfig {
                name: "grpc".to_string(),
                enabled: true,
                settings: json!({
                    "timeout": 5000,
                    "max_message_size": 10485760
                }),
            },
        ],
    }
}

pub fn create_minimal_test_config() -> GatewayConfig {
    GatewayConfig {
        port: 8080,
        host: "127.0.0.1".to_string(),
        protocols: vec![
            ProtocolConfig {
                name: "http".to_string(),
                enabled: true,
                settings: json!({}),
            },
        ],
    }
} 