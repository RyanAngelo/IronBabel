//! Integration tests for HTTP proxy behavior using wiremock as a backend.

use iron_babel::config::{HttpTransportConfig, RouteConfig, TransportConfig};
use iron_babel::core::Router;
use iron_babel::gateway::http::HttpGateway;
use iron_babel::protocols::http::HttpProtocol;
use std::sync::Arc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_route(p: &str, target: &str) -> RouteConfig {
    RouteConfig {
        path: p.to_string(),
        methods: vec![],
        transport: TransportConfig::Http(HttpTransportConfig {
            url: target.to_string(),
            timeout_secs: 5,
            strip_prefix: false,
        }),
    }
}

fn make_route_methods(p: &str, target: &str, methods: &[&str]) -> RouteConfig {
    RouteConfig {
        path: p.to_string(),
        methods: methods.iter().map(|s| s.to_string()).collect(),
        transport: TransportConfig::Http(HttpTransportConfig {
            url: target.to_string(),
            timeout_secs: 5,
            strip_prefix: false,
        }),
    }
}

fn gateway() -> HttpGateway {
    let protocol = Arc::new(HttpProtocol::new(serde_json::Value::Null).unwrap());
    HttpGateway::new(protocol)
}

// ---------------------------------------------------------------------------
// Router unit tests (using real RouteConfig)
// ---------------------------------------------------------------------------

#[test]
fn router_routes_to_longest_matching_prefix() {
    let router = Router::new(vec![
        make_route("/api", "http://backend-a"),
        make_route("/api/v2", "http://backend-b"),
    ]);
    let route = router.route("GET", "/api/v2/users").unwrap();
    assert_eq!(route.path, "/api/v2");
}

#[test]
fn router_falls_back_to_shorter_prefix() {
    let router = Router::new(vec![
        make_route("/api", "http://backend-a"),
        make_route("/api/v2", "http://backend-b"),
    ]);
    let route = router.route("GET", "/api/v1/users").unwrap();
    assert_eq!(route.path, "/api");
}

#[test]
fn router_returns_none_for_unmatched_path() {
    let router = Router::new(vec![make_route("/api", "http://backend-a")]);
    assert!(router.route("GET", "/health").is_none());
}

#[test]
fn router_returns_none_for_wrong_method() {
    let router = Router::new(vec![make_route_methods("/api", "http://backend-a", &["GET"])]);
    assert!(router.route("DELETE", "/api/resource").is_none());
}

#[test]
fn router_method_match_is_case_insensitive() {
    let router = Router::new(vec![make_route_methods("/api", "http://backend-a", &["get"])]);
    assert!(router.route("GET", "/api/users").is_some());
}

#[test]
fn router_empty_routes_returns_none() {
    let router = Router::new(vec![]);
    assert!(router.route("GET", "/anything").is_none());
}

// ---------------------------------------------------------------------------
// HTTP proxy integration tests (wiremock backend)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proxy_forwards_get_and_returns_response() {
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

    let gw = gateway();
    let (status, headers, body) = gw
        .proxy(
            reqwest::Method::GET,
            &mock_server.uri(),
            "/api/users",
            None,
            &[],
            vec![],
            5,
        )
        .await
        .unwrap();

    assert_eq!(status, 200);
    assert_eq!(body, br#"[{"id":1}]"#);
    assert!(headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")));
}

#[tokio::test]
async fn proxy_forwards_post_body() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/items"))
        .respond_with(ResponseTemplate::new(201).set_body_string("created"))
        .mount(&mock_server)
        .await;

    let gw = gateway();
    let (status, _headers, body) = gw
        .proxy(
            reqwest::Method::POST,
            &mock_server.uri(),
            "/api/items",
            None,
            &[(("content-type".to_string(), "application/json".to_string()))],
            br#"{"name":"test"}"#.to_vec(),
            5,
        )
        .await
        .unwrap();

    assert_eq!(status, 201);
    assert_eq!(body, b"created");
}

#[tokio::test]
async fn proxy_forwards_query_string() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string("results"))
        .mount(&mock_server)
        .await;

    let gw = gateway();
    let (status, _, _) = gw
        .proxy(
            reqwest::Method::GET,
            &mock_server.uri(),
            "/search",
            Some("q=rust&limit=10"),
            &[],
            vec![],
            5,
        )
        .await
        .unwrap();

    assert_eq!(status, 200);
}

#[tokio::test]
async fn proxy_strips_hop_by_hop_headers_from_request() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let gw = gateway();
    let headers = vec![
        ("transfer-encoding".to_string(), "chunked".to_string()),
        ("connection".to_string(), "keep-alive".to_string()),
        ("x-custom".to_string(), "preserved".to_string()),
    ];

    let (status, _, _) = gw
        .proxy(reqwest::Method::GET, &mock_server.uri(), "/", None, &headers, vec![], 5)
        .await
        .unwrap();

    assert_eq!(status, 200);

    let received = mock_server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1, "expected exactly one request at backend");
    let req = &received[0];

    assert!(
        req.headers.get("transfer-encoding").is_none(),
        "transfer-encoding should be stripped before forwarding"
    );
    assert!(
        req.headers.get("connection").is_none(),
        "connection should be stripped before forwarding"
    );
    assert_eq!(
        req.headers.get("x-custom").and_then(|v| v.to_str().ok()),
        Some("preserved"),
        "x-custom header should be forwarded unchanged"
    );
}

#[tokio::test]
async fn proxy_forwards_custom_request_header() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("x-request-id", "abc-123"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let gw = gateway();
    let (status, _, _) = gw
        .proxy(
            reqwest::Method::GET,
            &mock_server.uri(),
            "/",
            None,
            &[("x-request-id".to_string(), "abc-123".to_string())],
            vec![],
            5,
        )
        .await
        .unwrap();

    assert_eq!(status, 200);
}

#[tokio::test]
async fn proxy_returns_backend_error_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&mock_server)
        .await;

    let gw = gateway();
    let (status, _, body) = gw
        .proxy(reqwest::Method::GET, &mock_server.uri(), "/missing", None, &[], vec![], 5)
        .await
        .unwrap();

    assert_eq!(status, 404);
    assert_eq!(body, b"not found");
}
