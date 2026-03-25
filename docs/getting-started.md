# Getting Started with IronBabel

IronBabel is a cross-protocol API gateway written in Rust. It accepts HTTP requests and routes them to backends over HTTP, GraphQL, gRPC, WebSocket, ZeroMQ, or MQTT, translating wire formats and enforcing middleware policies along the way.

---

## Prerequisites

| Requirement | Minimum version | Notes |
|-------------|-----------------|-------|
| Rust toolchain | 1.75 | Install via [rustup.rs](https://rustup.rs) |
| `cargo` | ships with Rust | |
| `pkg-config` | any | Required by native TLS dependencies |
| `libssl-dev` | any | Required on Linux; macOS ships OpenSSL via Homebrew or the system |

On Debian/Ubuntu:

```sh
apt-get install -y pkg-config libssl-dev
```

On macOS (with Homebrew):

```sh
brew install openssl
```

---

## Building

Clone the repository and build in release mode:

```sh
git clone https://github.com/RyanAngelo/IronBabel.git
cd IronBabel
cargo build --release
```

The compiled binary is placed at `target/release/iron-babel`.

For a development build with debug symbols:

```sh
cargo build
# binary at target/debug/iron-babel
```

Run the test suite:

```sh
cargo test
```

---

## Running

IronBabel looks for its configuration file at `config/gateway.yaml` relative to the working directory by default. The path can be overridden with the `IRON_BABEL_CONFIG` environment variable.

```sh
# Default: reads config/gateway.yaml
./target/release/iron-babel

# Custom config path
IRON_BABEL_CONFIG=/etc/ironbabel/gateway.yaml ./target/release/iron-babel

# Override host and port at runtime without editing the file
IRON_BABEL_PORT=9090 IRON_BABEL_HOST=0.0.0.0 ./target/release/iron-babel
```

On startup you should see log output similar to:

```
INFO  iron_babel::core::gateway  IronBabel gateway listening on 127.0.0.1:8080
INFO  iron_babel::core::gateway  Admin dashboard available at http://127.0.0.1:8080/admin/
```

Shut the process down with `Ctrl-C`. The gateway performs a graceful shutdown, waiting up to 30 seconds for in-flight requests to complete before forcing an exit.

---

## 5-Minute Walkthrough

The following steps spin up IronBabel and demonstrate HTTP proxying, auth, and the admin dashboard. You need two terminal windows.

### Step 1 — Write a minimal config

Create `config/gateway.yaml`:

```yaml
host: "127.0.0.1"
port: 8080

protocols:
  - name: "http"
    enabled: true
    settings: {}

middleware:
  auth:
    enabled: true
    api_keys:
      - "my-secret-token"
  rate_limit:
    enabled: true
    requests_per_window: 60
    window_secs: 60

routes:
  - path: "/api"
    methods: ["GET", "POST"]
    transport:
      type: http
      url: "http://httpbin.org"
      timeout_secs: 10
      strip_prefix: false

  - path: "/health"
    methods: []
    transport:
      type: http
      url: "http://httpbin.org"
      timeout_secs: 5
```

### Step 2 — Start the gateway

```sh
cargo run
```

### Step 3 — Send an unauthenticated request (should be rejected)

```sh
curl -i http://127.0.0.1:8080/api/get
```

Expected response: `401 Unauthorized`

### Step 4 — Send an authenticated request

```sh
curl -i \
  -H "Authorization: Bearer my-secret-token" \
  http://127.0.0.1:8080/api/get
```

The gateway forwards this to `http://httpbin.org/api/get` and returns the upstream response.

### Step 5 — Hit the health route (no auth required because api_keys check applies globally)

Wait — with auth enabled and `api_keys` non-empty, every route requires a Bearer token. Send the token:

```sh
curl -i \
  -H "Authorization: Bearer my-secret-token" \
  http://127.0.0.1:8080/health
```

### Step 6 — Open the admin dashboard

Navigate to [http://127.0.0.1:8080/admin/](http://127.0.0.1:8080/admin/) in a browser. You will see the metrics dashboard with request counts, latency percentiles, and a live request log.

---

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `IRON_BABEL_CONFIG` | Path to the YAML configuration file | `config/gateway.yaml` |
| `IRON_BABEL_HOST` | Override `host` from config | — |
| `IRON_BABEL_PORT` | Override `port` from config | — |
| `RUST_LOG` | Log filter for the `tracing` subscriber | `info` (gateway module uses `debug`) |

The `RUST_LOG` variable accepts standard `tracing` filter syntax, for example:

```sh
RUST_LOG=iron_babel=debug,tower_http=trace ./target/release/iron-babel
```

---

## Running with Docker (Dev Container)

A `.devcontainer` configuration is included. Open the repository in VS Code with the Dev Containers extension and the environment will be set up automatically using the provided `Dockerfile` based on `rust:1.94-slim-bullseye`.

---

## Project Layout

```
IronBabel/
├── config/
│   └── gateway.yaml          # Default configuration file
├── assets/
│   └── admin/
│       └── index.html        # Embedded admin dashboard HTML
├── src/
│   ├── main.rs               # Binary entry point
│   ├── lib.rs                # Library crate root
│   ├── config/               # Configuration loading and structs
│   ├── core/                 # Gateway trait, Router, middleware chain
│   ├── gateway/              # Per-protocol proxy implementations
│   ├── protocols/            # Protocol encode/decode logic
│   ├── admin/                # Admin API handlers and metrics store
│   └── utils/                # Logging initialisation
├── docs/                     # Documentation
└── Cargo.toml
```
