//! Load generator for IronBabel.
//!
//! Spins up two mock HTTP backends on :9000 and :9001, then pumps varied
//! traffic through the gateway and prints live stats.
//!
//! Usage:
//!   cargo run --bin load_gen -- [--gateway URL] [--rps N] [--error-rate F] [--slow-rate F]
//!       [--include-mqtt] [--include-amqp]
//!
//! Defaults: gateway=http://127.0.0.1:8080, rps=10, error-rate=0.10, slow-rate=0.05

use axum::{extract::State, response::IntoResponse, routing::any, Router};
use reqwest::Client;
use std::{
    env,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::time::interval;

// ---------------------------------------------------------------------------
// Mock backend
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BackendState {
    error_rate: f64,
    slow_rate: f64,
    slow_delay_ms: u64,
    counter: Arc<AtomicU64>,
}

/// Hash-based pseudo-random float in [0, 1) — no external crates needed.
fn frand(seed: u64) -> f64 {
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    (h.finish() as f64) / (u64::MAX as f64)
}

async fn mock_handler(State(s): State<BackendState>) -> impl IntoResponse {
    let n = s.counter.fetch_add(1, Ordering::Relaxed);

    if frand(n.wrapping_mul(7)) < s.slow_rate {
        tokio::time::sleep(Duration::from_millis(s.slow_delay_ms)).await;
    }

    if frand(n.wrapping_mul(13).wrapping_add(1)) < s.error_rate {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "simulated failure" })),
        )
            .into_response();
    }

    axum::Json(serde_json::json!({ "ok": true, "seq": n })).into_response()
}

fn spawn_mock_backend(port: u16, state: BackendState) {
    let app = Router::new()
        .fallback(any(mock_handler))
        .with_state(state);
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
            .await
            .unwrap_or_else(|e| panic!("mock backend :{port} bind failed: {e}"));
        axum::serve(listener, app)
            .await
            .expect("mock backend crashed");
    });
}

// ---------------------------------------------------------------------------
// Traffic routes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Route {
    method: &'static str,
    path: &'static str,
    /// JSON body template (None = no body).
    body: Option<&'static str>,
}

const BASE_ROUTES: &[Route] = &[
    Route { method: "GET",    path: "/api/v1/users",      body: None },
    Route { method: "GET",    path: "/api/v1/products",   body: None },
    Route { method: "POST",   path: "/api/v1/orders",     body: Some(r#"{"item":"widget","qty":3}"#) },
    Route { method: "GET",    path: "/api/v1/items/42",   body: None },
    Route { method: "PUT",    path: "/api/v1/items/42",   body: Some(r#"{"name":"updated"}"#) },
    Route { method: "DELETE", path: "/api/v1/items/42",   body: None },
    Route { method: "GET",    path: "/health",            body: None },
    Route { method: "GET",    path: "/api/status",        body: None },
    Route { method: "POST",   path: "/api/events",        body: Some(r#"{"event":"click","user":1}"#) },
    Route { method: "GET",    path: "/nonexistent",       body: None }, // intentional 404
];

const MQTT_ROUTES: &[Route] = &[
    Route { method: "POST", path: "/mqtt/events", body: Some(r#"{"event":"device.ping","device_id":"sim-001","temperature_c":21.4}"#) },
];

const AMQP_ROUTES: &[Route] = &[
    Route { method: "POST", path: "/amqp/events", body: Some(r#"{"event":"job.created","job_id":"job-001","priority":"normal"}"#) },
];

fn build_routes(cfg: &Config) -> Vec<Route> {
    let mut routes = BASE_ROUTES.to_vec();
    if cfg.include_mqtt {
        routes.extend_from_slice(MQTT_ROUTES);
    }
    if cfg.include_amqp {
        routes.extend_from_slice(AMQP_ROUTES);
    }
    routes
}

// ---------------------------------------------------------------------------
// Traffic pump
// ---------------------------------------------------------------------------

async fn pump_traffic(
    client: Arc<Client>,
    gateway: String,
    routes: Arc<Vec<Route>>,
    rps: u64,
    sent: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
) {
    let rps = rps.max(1);
    for worker_id in 0..rps {
        let client = client.clone();
        let gateway = gateway.clone();
        let routes = routes.clone();
        let sent = sent.clone();
        let errors = errors.clone();

        tokio::spawn(async move {
            // Stagger workers evenly across the first second.
            let stagger_ms = 1000 / rps * worker_id;
            tokio::time::sleep(Duration::from_millis(stagger_ms)).await;

            let mut ticker = interval(Duration::from_secs(1));
            let mut seq: u64 = worker_id;

            loop {
                ticker.tick().await;

                let route = routes[(seq as usize) % routes.len()];
                let url = format!("{}{}", gateway, route.path);

                let method = reqwest::Method::from_bytes(route.method.as_bytes())
                    .unwrap_or(reqwest::Method::GET);

                let req = client.request(method, &url);
                let req = match route.body {
                    Some(b) => req
                        .header("Content-Type", "application/json")
                        .body(b.to_string()),
                    None => req,
                };

                match req.send().await {
                    Ok(resp) if resp.status().is_server_error() => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }

                sent.fetch_add(1, Ordering::Relaxed);
                seq = seq.wrapping_add(rps);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

struct Config {
    gateway: String,
    rps: u64,
    error_rate: f64,
    slow_rate: f64,
    include_mqtt: bool,
    include_amqp: bool,
}

fn parse_args() -> Config {
    parse_args_from(env::args().collect())
}

fn parse_args_from(args: Vec<String>) -> Config {
    let mut cfg = Config {
        gateway: "http://127.0.0.1:8080".into(),
        rps: 10,
        error_rate: 0.10,
        slow_rate: 0.05,
        include_mqtt: false,
        include_amqp: false,
    };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--gateway" if i + 1 < args.len() => {
                cfg.gateway = args[i + 1].clone();
                i += 2;
            }
            "--rps" if i + 1 < args.len() => {
                cfg.rps = args[i + 1].parse().unwrap_or(10);
                i += 2;
            }
            "--error-rate" if i + 1 < args.len() => {
                cfg.error_rate = args[i + 1].parse().unwrap_or(0.10);
                i += 2;
            }
            "--slow-rate" if i + 1 < args.len() => {
                cfg.slow_rate = args[i + 1].parse().unwrap_or(0.05);
                i += 2;
            }
            "--include-mqtt" => {
                cfg.include_mqtt = true;
                i += 1;
            }
            "--include-amqp" => {
                cfg.include_amqp = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    cfg
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let cfg = parse_args();

    println!("IronBabel Load Generator");
    println!("  gateway    : {}", cfg.gateway);
    println!("  rps        : {}", cfg.rps);
    println!("  error rate : {:.0}%  (backend 500s)", cfg.error_rate * 100.0);
    println!("  slow rate  : {:.0}%  (200 ms added latency)", cfg.slow_rate * 100.0);
    println!(
        "  protocols  : http{}{}",
        if cfg.include_mqtt { ", mqtt" } else { "" },
        if cfg.include_amqp { ", amqp" } else { "" },
    );
    println!();

    let backend_state = BackendState {
        error_rate: cfg.error_rate,
        slow_rate: cfg.slow_rate,
        slow_delay_ms: 200,
        counter: Arc::new(AtomicU64::new(0)),
    };

    spawn_mock_backend(9000, backend_state.clone());
    spawn_mock_backend(9001, backend_state);
    tokio::time::sleep(Duration::from_millis(150)).await;
    println!("Mock backends listening on :9000 and :9001");

    let client = Arc::new(
        Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client"),
    );

    // Wait for the gateway to be reachable.
    print!("Waiting for gateway");
    let _ = std::io::stdout().flush();
    loop {
        let probe = client
            .get(format!("{}/health", cfg.gateway))
            .send()
            .await;
        if probe.is_ok() {
            break;
        }
        print!(".");
        let _ = std::io::stdout().flush();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    println!("  connected!\n");

    let sent = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let routes = Arc::new(build_routes(&cfg));

    pump_traffic(
        client,
        cfg.gateway.clone(),
        routes,
        cfg.rps,
        sent.clone(),
        errors.clone(),
    )
    .await;

    // Live stats — one line per second.
    println!(
        "{:<8}  {:>10}  {:>8}  {:>8}  {:>7}",
        "elapsed", "total", "rps", "errors", "err%"
    );
    println!("{}", "─".repeat(48));

    let mut ticker = interval(Duration::from_secs(1));
    let mut last_sent = 0u64;
    let start = Instant::now();

    loop {
        ticker.tick().await;
        let total = sent.load(Ordering::Relaxed);
        let errs = errors.load(Ordering::Relaxed);
        let current_rps = total - last_sent;
        last_sent = total;
        let err_pct = if total > 0 {
            errs as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "{:<8}  {:>10}  {:>8}  {:>8}  {:>6.1}%",
            format!("{}s", start.elapsed().as_secs()),
            total,
            current_rps,
            errs,
            err_pct,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_routes_is_http_only_by_default() {
        let cfg = Config {
            gateway: "http://127.0.0.1:8080".to_string(),
            rps: 10,
            error_rate: 0.1,
            slow_rate: 0.05,
            include_mqtt: false,
            include_amqp: false,
        };

        let routes = build_routes(&cfg);
        assert_eq!(routes.len(), BASE_ROUTES.len());
        assert!(!routes.iter().any(|route| route.path == "/mqtt/events"));
        assert!(!routes.iter().any(|route| route.path == "/amqp/events"));
    }

    #[test]
    fn build_routes_includes_optional_protocol_paths() {
        let cfg = Config {
            gateway: "http://127.0.0.1:8080".to_string(),
            rps: 10,
            error_rate: 0.1,
            slow_rate: 0.05,
            include_mqtt: true,
            include_amqp: true,
        };

        let routes = build_routes(&cfg);
        assert!(routes.iter().any(|route| route.path == "/mqtt/events"));
        assert!(routes.iter().any(|route| route.path == "/amqp/events"));
    }

    #[test]
    fn parse_args_from_reads_optional_protocol_flags() {
        let cfg = parse_args_from(vec![
            "load_gen".to_string(),
            "--gateway".to_string(),
            "http://localhost:8080".to_string(),
            "--include-mqtt".to_string(),
            "--include-amqp".to_string(),
            "--rps".to_string(),
            "25".to_string(),
        ]);

        assert_eq!(cfg.gateway, "http://localhost:8080");
        assert_eq!(cfg.rps, 25);
        assert!(cfg.include_mqtt);
        assert!(cfg.include_amqp);
    }
}
