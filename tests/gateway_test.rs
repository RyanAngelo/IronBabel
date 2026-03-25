//! Tests for DefaultGateway lifecycle and create_gateway factory logic.

mod common;

use iron_babel::config::{
    AmqpTransportConfig, GatewayConfig, HttpTransportConfig, MqttTransportConfig, ProtocolConfig,
    RouteConfig, TransportConfig,
};
use iron_babel::core::gateway::create_gateway;
use iron_babel::core::Gateway;
use serde_json::json;
use wiremock::matchers::{header, method, path};
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

fn mqtt_route(path: &str, broker_url: &str, topic: &str, methods: &[&str]) -> RouteConfig {
    RouteConfig {
        path: path.to_string(),
        methods: methods.iter().map(|s| s.to_string()).collect(),
        transport: TransportConfig::Mqtt(MqttTransportConfig {
            broker_url: broker_url.to_string(),
            topic: topic.to_string(),
            qos: 1,
            retain: false,
            client_id: Some("iron-babel-test-pub".to_string()),
            timeout_secs: 1,
        }),
    }
}

fn amqp_route(path: &str, broker_url: &str, routing_key: &str, methods: &[&str]) -> RouteConfig {
    RouteConfig {
        path: path.to_string(),
        methods: methods.iter().map(|s| s.to_string()).collect(),
        transport: TransportConfig::Amqp(AmqpTransportConfig {
            broker_url: broker_url.to_string(),
            exchange: "".to_string(),
            routing_key: routing_key.to_string(),
            mandatory: false,
            persistent: true,
            content_type: Some("application/octet-stream".to_string()),
            timeout_secs: 1,
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
fn gateway_creates_mqtt_protocol() {
    let config = config_with_protocols(vec![ProtocolConfig {
        name: "mqtt".to_string(),
        enabled: true,
        settings: json!({}),
    }]);
    let gw = create_gateway(config).unwrap();
    assert_eq!(gw.protocols().len(), 1);
    assert_eq!(gw.protocols()[0].name(), "mqtt");
}

#[test]
fn gateway_creates_amqp_protocol() {
    let config = config_with_protocols(vec![ProtocolConfig {
        name: "amqp".to_string(),
        enabled: true,
        settings: json!({}),
    }]);
    let gw = create_gateway(config).unwrap();
    assert_eq!(gw.protocols().len(), 1);
    assert_eq!(gw.protocols()[0].name(), "amqp");
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
async fn gateway_forwards_verified_client_ip() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/users"))
        .and(header("x-forwarded-for", "127.0.0.1"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
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

    let result = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}/anything", port))
        .timeout(std::time::Duration::from_millis(200))
        .send()
        .await;

    assert!(result.is_err(), "server should no longer be reachable after stop()");
}

#[tokio::test]
async fn gateway_can_restart_after_stop() {
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
    gateway.stop().await.unwrap();

    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{}/anything", port))
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);

    gateway.stop().await.unwrap();
}

#[tokio::test]
async fn admin_endpoints_report_mqtt_routes_and_requests() {
    let port = free_port();
    let config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![ProtocolConfig {
            name: "mqtt".to_string(),
            enabled: true,
            settings: json!({}),
        }],
        routes: vec![mqtt_route(
            "/mqtt/events",
            "mqtt://127.0.0.1:18830",
            "events.http",
            &["POST"],
        )],
        listeners: vec![],
        middleware: Default::default(),
    };

    let gateway = create_gateway(config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let mqtt_resp = client
        .post(format!("http://127.0.0.1:{}/mqtt/events", port))
        .body("hello mqtt")
        .send()
        .await
        .unwrap();
    assert_eq!(mqtt_resp.status().as_u16(), 502);

    let routes: serde_json::Value = reqwest::get(format!(
        "http://127.0.0.1:{}/admin/api/routes",
        port
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();

    let mqtt_route = routes
        .as_array()
        .unwrap()
        .iter()
        .find(|route| route.get("path") == Some(&serde_json::Value::String("/mqtt/events".to_string())))
        .expect("mqtt route should appear in admin routes");
    assert_eq!(mqtt_route.get("transport_type").unwrap(), "mqtt");
    assert_eq!(mqtt_route.get("target").unwrap(), "mqtt://127.0.0.1:18830");
    assert_eq!(mqtt_route.get("total_requests").unwrap(), 1);
    assert_eq!(mqtt_route.get("error_count").unwrap(), 1);

    let metrics: serde_json::Value = reqwest::get(format!(
        "http://127.0.0.1:{}/admin/api/metrics",
        port
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(metrics.get("total_requests").unwrap(), 1);
    assert!(metrics.get("total_errors").is_none());
    assert_eq!(
        metrics
            .get("requests_by_route")
            .unwrap()
            .get("/mqtt/events")
            .unwrap(),
        1
    );
    assert_eq!(
        metrics
            .get("status_code_counts")
            .unwrap()
            .get("502")
            .unwrap(),
        1
    );

    let recent: serde_json::Value = reqwest::get(format!(
        "http://127.0.0.1:{}/admin/api/requests/recent?n=5",
        port
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let recent_entry = recent.as_array().unwrap().first().unwrap();
    assert_eq!(recent_entry.get("matched_route").unwrap(), "/mqtt/events");
    assert_eq!(recent_entry.get("upstream_target").unwrap(), "mqtt://127.0.0.1:18830");
    assert_eq!(recent_entry.get("status_code").unwrap(), 502);

    gateway.stop().await.unwrap();
}

#[tokio::test]
async fn admin_endpoints_report_amqp_routes_and_requests() {
    let port = free_port();
    let config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![ProtocolConfig {
            name: "amqp".to_string(),
            enabled: true,
            settings: json!({}),
        }],
        routes: vec![amqp_route(
            "/amqp/events",
            "amqp://guest:guest@127.0.0.1:5673/%2f",
            "events.http",
            &["POST"],
        )],
        listeners: vec![],
        middleware: Default::default(),
    };

    let gateway = create_gateway(config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let amqp_resp = client
        .post(format!("http://127.0.0.1:{}/amqp/events", port))
        .body("hello amqp")
        .send()
        .await
        .unwrap();
    assert_eq!(amqp_resp.status().as_u16(), 502);

    let routes: serde_json::Value = reqwest::get(format!(
        "http://127.0.0.1:{}/admin/api/routes",
        port
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();

    let amqp_route = routes
        .as_array()
        .unwrap()
        .iter()
        .find(|route| route.get("path") == Some(&serde_json::Value::String("/amqp/events".to_string())))
        .expect("amqp route should appear in admin routes");
    assert_eq!(amqp_route.get("transport_type").unwrap(), "amqp");
    assert_eq!(
        amqp_route.get("target").unwrap(),
        "amqp://guest:guest@127.0.0.1:5673/%2f"
    );
    assert_eq!(amqp_route.get("total_requests").unwrap(), 1);
    assert_eq!(amqp_route.get("error_count").unwrap(), 1);

    let metrics: serde_json::Value = reqwest::get(format!(
        "http://127.0.0.1:{}/admin/api/metrics",
        port
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(metrics.get("total_requests").unwrap(), 1);
    assert_eq!(
        metrics
            .get("requests_by_route")
            .unwrap()
            .get("/amqp/events")
            .unwrap(),
        1
    );
    assert_eq!(
        metrics
            .get("status_code_counts")
            .unwrap()
            .get("502")
            .unwrap(),
        1
    );

    let recent: serde_json::Value = reqwest::get(format!(
        "http://127.0.0.1:{}/admin/api/requests/recent?n=5",
        port
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let recent_entry = recent.as_array().unwrap().first().unwrap();
    assert_eq!(recent_entry.get("matched_route").unwrap(), "/amqp/events");
    assert_eq!(
        recent_entry.get("upstream_target").unwrap(),
        "amqp://guest:guest@127.0.0.1:5673/%2f"
    );
    assert_eq!(recent_entry.get("status_code").unwrap(), 502);

    gateway.stop().await.unwrap();
}

#[tokio::test]
async fn admin_config_endpoints_expose_schema_and_persist_valid_drafts() {
    let mock_server = MockServer::start().await;
    let port = free_port();
    let config = GatewayConfig {
        host: "127.0.0.1".to_string(),
        port,
        protocols: vec![
            http_protocol(true),
            ProtocolConfig {
                name: "mqtt".to_string(),
                enabled: true,
                settings: json!({}),
            },
            ProtocolConfig {
                name: "amqp".to_string(),
                enabled: true,
                settings: json!({}),
            },
        ],
        routes: vec![http_route("/api", &mock_server.uri(), &["GET"])],
        listeners: vec![],
        middleware: Default::default(),
    };

    let gateway = create_gateway(config).unwrap();
    gateway.start().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();

    let snapshot: serde_json::Value = client
        .get(format!("{}/admin/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot["active"]["routes"][0]["path"], "/api");
    assert_eq!(snapshot["draft"]["routes"][0]["path"], "/api");

    let schema: serde_json::Value = client
        .get(format!("{}/admin/api/config/schema", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let route_kinds = schema["route_templates"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["kind"].as_str())
        .collect::<Vec<_>>();
    assert!(route_kinds.contains(&"mqtt"));
    assert!(route_kinds.contains(&"amqp"));
    let listener_kinds = schema["listener_templates"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["kind"].as_str())
        .collect::<Vec<_>>();
    assert!(listener_kinds.contains(&"mqtt_sub"));
    assert!(listener_kinds.contains(&"amqp_consume"));

    let invalid_validate: serde_json::Value = client
        .post(format!("{}/admin/api/config/validate", base))
        .json(&json!({
            "host": "",
            "port": 0,
            "protocols": [],
            "routes": [],
            "listeners": [],
            "middleware": {
                "auth": { "enabled": false, "api_keys": [] },
                "rate_limit": { "enabled": false, "requests_per_window": 60, "window_secs": 60 },
                "logging": { "enabled": true }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invalid_validate["valid"], false);
    assert!(invalid_validate["errors"][0].as_str().unwrap().contains("host must not be empty"));

    let new_draft = json!({
        "host": "127.0.0.1",
        "port": port,
        "protocols": [
            { "name": "http", "enabled": true, "settings": {} },
            { "name": "mqtt", "enabled": true, "settings": {} },
            { "name": "amqp", "enabled": true, "settings": {} }
        ],
        "routes": [
            {
                "path": "/api",
                "methods": ["GET"],
                "transport": {
                    "type": "http",
                    "url": mock_server.uri(),
                    "timeout_secs": 5,
                    "strip_prefix": false
                }
            },
            {
                "path": "/amqp/events",
                "methods": ["POST"],
                "transport": {
                    "type": "amqp",
                    "broker_url": "amqp://guest:guest@127.0.0.1:5672/%2f",
                    "exchange": "",
                    "routing_key": "events.http",
                    "mandatory": false,
                    "persistent": true,
                    "timeout_secs": 10
                }
            }
        ],
        "listeners": [
            {
                "type": "amqp_consume",
                "broker_url": "amqp://guest:guest@127.0.0.1:5672/%2f",
                "queue": "events.inbox",
                "auto_ack": false,
                "forward_to": format!("{}/amqp-webhook", mock_server.uri())
            }
        ],
        "middleware": {
            "auth": { "enabled": false, "api_keys": [] },
            "rate_limit": { "enabled": false, "requests_per_window": 60, "window_secs": 60 },
            "logging": { "enabled": true }
        }
    });

    let saved: serde_json::Value = client
        .put(format!("{}/admin/api/config/draft", base))
        .json(&new_draft)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(saved["saved"], true);
    assert_eq!(saved["draft"]["routes"][1]["transport"]["type"], "amqp");

    let snapshot_after: serde_json::Value = client
        .get(format!("{}/admin/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot_after["active"]["routes"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot_after["draft"]["routes"].as_array().unwrap().len(), 2);
    assert_eq!(
        snapshot_after["draft"]["listeners"][0]["type"],
        "amqp_consume"
    );

    gateway.stop().await.unwrap();
}
