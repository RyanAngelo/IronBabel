//! Tests for DefaultGateway lifecycle and create_gateway factory logic.

mod common;

use iron_babel::config::{
    GatewayConfig, HttpTransportConfig, ProtocolConfig, RouteConfig, TransportConfig,
};
use iron_babel::core::gateway::create_gateway;
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

fn http_route(path: &str, target: &str, methods: &[&str]) -> RouteConfig {
    RouteConfig {
        path: path.to_string(),
        methods: methods.iter().map(|s| s.to_string()).collect(),
        transport: TransportConfig::Http(HttpTransportConfig {
            url: target.to_string(),
            timeout_secs: 5,
            strip_prefix: false,
        }),
    }
}

fn config_with_protocols(protocols: Vec<ProtocolConfig>) -> GatewayConfig {
    GatewayConfig {
        port: 8080,
        host: "127.0.0.1".to_string(),
        protocols,
        routes: vec![],
        listeners: vec![],
        middleware: Default::default(),
    }
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

// ---------------------------------------------------------------------------
// Protocol building tests
// ---------------------------------------------------------------------------

#[test]
fn gateway_creates_http_protocol() {
    let config = config_with_protocols(vec![http_protocol(true)]);
    let gw = create_gateway(config).unwrap();
    assert_eq!(gw.protocols().len(), 1);
    assert_eq!(gw.protocols()[0].name(), "http");
}

#[test]
fn gateway_rejects_unknown_protocol() {
    let config = config_with_protocols(vec![ProtocolConfig {
        name: "unknown_proto".to_string(),
        enabled: true,
        settings: json!({}),
    }]);
    let err = create_gateway(config).err().unwrap();
    assert!(
        err.to_string().contains("Unsupported protocol"),
        "unexpected error message: {err}"
    );
}

#[test]
fn gateway_skips_disabled_protocols() {
    let config = config_with_protocols(vec![http_protocol(false), http_protocol(true)]);
    let gw = create_gateway(config).unwrap();
    assert_eq!(gw.protocols().len(), 1);
}

#[test]
fn gateway_empty_protocols_is_ok() {
    let config = config_with_protocols(vec![]);
    let gw = create_gateway(config).unwrap();
    assert!(gw.protocols().is_empty());
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
    let config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![http_route("/api", &mock_server.uri(), &[])],
        listeners: vec![],
        middleware: Default::default(),
    };

    let gateway = create_gateway(config).unwrap();
    gateway.start().await.unwrap();
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
    let config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![http_route("/api", "http://127.0.0.1:19999", &[])],
        listeners: vec![],
        middleware: Default::default(),
    };

    let gateway = create_gateway(config).unwrap();
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
    let config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![http_route("/items", &mock_server.uri(), &["POST"])],
        listeners: vec![],
        middleware: Default::default(),
    };

    let gateway = create_gateway(config).unwrap();
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
async fn gateway_returns_404_for_wrong_method() {
    let port = free_port();
    let config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![http_route("/api", "http://127.0.0.1:19999", &["GET"])],
        listeners: vec![],
        middleware: Default::default(),
    };

    let gateway = create_gateway(config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("http://127.0.0.1:{}/api/resource", port))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 404);
    gateway.stop().await.unwrap();
}

#[tokio::test]
async fn gateway_stop_shuts_down_server() {
    let port = free_port();
    let config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![],
        routes: vec![],
        listeners: vec![],
        middleware: Default::default(),
    };

    let gateway = create_gateway(config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/anything", port))
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);

    gateway.stop().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let result = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}/anything", port))
        .timeout(std::time::Duration::from_millis(200))
        .send()
        .await;

    assert!(result.is_err(), "server should no longer be reachable after stop()");
}
