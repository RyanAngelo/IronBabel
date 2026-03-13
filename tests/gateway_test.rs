//! Tests for DefaultGateway lifecycle and GatewayConfig factory logic.

mod common;

use iron_babel::config::{GatewayConfig as AppConfig, ProtocolConfig, RouteConfig};
use iron_babel::core::gateway::{create_gateway, GatewayConfig};
use iron_babel::core::Gateway;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn http_protocol(enabled: bool) -> ProtocolConfig {
    ProtocolConfig {
        name: "http".to_string(),
        enabled,
        settings: json!({}),
    }
}

fn app_config_with_protocols(protocols: Vec<ProtocolConfig>) -> AppConfig {
    AppConfig {
        port: 8080,
        host: "127.0.0.1".to_string(),
        protocols,
        routes: vec![],
    }
}

/// Bind to port 0, record the assigned port, then release the socket so the
/// gateway can bind it a moment later. The OS will not immediately reuse the
/// ephemeral port, making this safe for test isolation.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

// ---------------------------------------------------------------------------
// GatewayConfig::from_config tests
// ---------------------------------------------------------------------------

#[test]
fn gateway_config_creates_http_protocol() {
    let config = app_config_with_protocols(vec![http_protocol(true)]);
    let gw = GatewayConfig::from_config(config).unwrap();
    assert_eq!(gw.protocols.len(), 1);
    assert_eq!(gw.protocols[0].name(), "http");
}

#[test]
fn gateway_config_rejects_unknown_protocol() {
    let config = app_config_with_protocols(vec![ProtocolConfig {
        name: "unknown_proto".to_string(),
        enabled: true,
        settings: json!({}),
    }]);
    let err = GatewayConfig::from_config(config).err().unwrap();
    assert!(
        err.to_string().contains("Unsupported protocol"),
        "unexpected error message: {err}"
    );
}

#[test]
fn gateway_config_skips_disabled_protocols() {
    let config = app_config_with_protocols(vec![
        http_protocol(false), // disabled — should be excluded
        http_protocol(true),  // enabled — should be included
    ]);
    let gw = GatewayConfig::from_config(config).unwrap();
    assert_eq!(gw.protocols.len(), 1);
}

#[test]
fn gateway_config_empty_protocols_is_ok() {
    let config = app_config_with_protocols(vec![]);
    let gw = GatewayConfig::from_config(config).unwrap();
    assert!(gw.protocols.is_empty());
}

#[test]
fn gateway_config_propagates_routes() {
    let mut config = app_config_with_protocols(vec![]);
    config.routes = vec![RouteConfig {
        path: "/api".to_string(),
        target: "http://127.0.0.1:9000".to_string(),
        methods: vec![],
        timeout_secs: Some(10),
        strip_prefix: None,
    }];
    let gw = GatewayConfig::from_config(config).unwrap();
    assert_eq!(gw.routes.len(), 1);
    assert_eq!(gw.routes[0].path, "/api");
}

// ---------------------------------------------------------------------------
// End-to-end gateway tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gateway_proxies_get_request_end_to_end() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/users"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"[{"id":1}]"#)
                .insert_header("content-type", "application/json"),
        )
        .mount(&mock_server)
        .await;

    let port = free_port();
    let gw_config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![RouteConfig {
            path: "/api".to_string(),
            target: mock_server.uri(),
            methods: vec![],
            timeout_secs: Some(5),
            strip_prefix: None,
        }],
    };

    let gateway = create_gateway(gw_config).unwrap();
    gateway.start().await.unwrap();
    // Give the spawned serve task a moment to start accepting connections.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/api/users", port))
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().await.unwrap(), r#"[{"id":1}]"#);

    gateway.stop().await.unwrap();
}

#[tokio::test]
async fn gateway_returns_404_for_unmatched_route() {
    let port = free_port();
    let gw_config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![RouteConfig {
            path: "/api".to_string(),
            target: "http://127.0.0.1:19999".to_string(),
            methods: vec![],
            timeout_secs: Some(5),
            strip_prefix: None,
        }],
    };

    let gateway = create_gateway(gw_config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/health", port))
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 404);
    gateway.stop().await.unwrap();
}

#[tokio::test]
async fn gateway_proxies_post_with_body() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/items"))
        .respond_with(ResponseTemplate::new(201).set_body_string("created"))
        .mount(&mock_server)
        .await;

    let port = free_port();
    let gw_config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![RouteConfig {
            path: "/items".to_string(),
            target: mock_server.uri(),
            methods: vec!["POST".to_string()],
            timeout_secs: Some(5),
            strip_prefix: None,
        }],
    };

    let gateway = create_gateway(gw_config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{}/items", port))
        .header("content-type", "application/json")
        .body(r#"{"name":"widget"}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 201);
    assert_eq!(resp.text().await.unwrap(), "created");

    gateway.stop().await.unwrap();
}

#[tokio::test]
async fn gateway_returns_405_for_wrong_method() {
    let port = free_port();
    let gw_config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![RouteConfig {
            path: "/api".to_string(),
            target: "http://127.0.0.1:19999".to_string(),
            methods: vec!["GET".to_string()],
            timeout_secs: Some(5),
            strip_prefix: None,
        }],
    };

    let gateway = create_gateway(gw_config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("http://127.0.0.1:{}/api/resource", port))
        .send()
        .await
        .unwrap();

    // DELETE is not in the allowed methods list — router returns None → 404.
    assert_eq!(resp.status().as_u16(), 404);

    gateway.stop().await.unwrap();
}

#[tokio::test]
async fn gateway_stop_shuts_down_server() {
    let port = free_port();
    let gw_config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![],
    };

    let gateway = create_gateway(gw_config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Confirm it's up.
    let resp = reqwest::get(format!("http://127.0.0.1:{}/anything", port))
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404); // no routes, but server responded

    // Stop and confirm it's no longer accepting connections.
    gateway.stop().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let result = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}/anything", port))
        .timeout(std::time::Duration::from_millis(200))
        .send()
        .await;

    assert!(result.is_err(), "server should no longer be reachable after stop()");
}
