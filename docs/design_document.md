# Iron Babel – Design & Architecture Documentation

---

## Table of Contents

| Section | Topic |
|---------|-------|
| 1 | Project Overview |
| 2 | High‑Level Architecture |
| 3 | Configuration |
| 4 | Protocol Support |
| 5 | Gateway Core |
| 6 | Transformation Layer |
| 7 | Schema Management |
| 8 | Metrics & Logging |
| 9 | Testing & CI |
| 10 | Contribution Workflow |
| 11 | Security & Hardening |
| 12 | Future‑Proofing & Enhancements |
| A | Key Code Paths & Module Diagram |

> **NOTE** – This document is a *static reference*.
> Because the repository is in a read‑only phase, the text below **cannot** be written automatically into the repository.  Please copy‑paste the content into `docs/design_document.md` (or your preferred location) if you wish to store it.

---

## 1. Project Overview

Iron Babel is a **cross‑protocol API gateway** implemented in Rust.  Its purpose is to allow services that speak different protocols (REST, GraphQL, gRPC, WebSocket, MQTT, etc.) to interoperate without the caller needing to know the intricacies of each protocol.

*Key Features*
- Runtime protocol translation (HTTP ↔ GraphQL, HTTP ↔ gRPC, HTTP ↔ WebSocket, HTTP ↔ MQTT, HTTP polling, SSE).
- Automatic schema discovery / generation per endpoint.
- Request/response transformation (JSON ↔ Protobuf, custom adapters).
- Configurable routing, rate‑limiting, circuit‑breaking.
- Built‑in observability (Prometheus, debug logging).
- Highly modular – new protocols or adapters can be added without touching core logic.

---

## 2. High‑Level Architecture

```
┌─────────────────────┐   ←  User / Client
│    HTTP Client       │
└─┬───────────────────┘
  │ request
  ▼
 ┌─────────────────────┐
 │  Iron Babel          │   <─ GatewayCore ──► ProtocolGateway(s)
 │   (Rust lib)         │
 └─┬───────────────────┘
   │
 ┌───┴───────────────┐
 │  Metrics /        │
 │  Logging API      │
 └───┬───────────────┘
   │
 ┌───┴───────────────┐
 │  Config /         │
 │  Schemas          │
 └───┬───────────────┘
   │
```

1. **Entry point** – The binary (`src/main.rs`) starts a Tokio runtime, initializes logging, loads configuration (`src/config/`), constructs the gateway, and waits for shutdown.
2. **Gateway Core** – `src/core/` hosts `Gateway` trait, `Router`, and request/response types.
3. **Routing** – `core/router.rs` matches incoming HTTP requests to `RouteConfig` entries.
4. **Protocol Gateways** – Each backend protocol implements `gateway::ProtocolGateway`. For HTTP we have `gateway::http::HttpGateway`, which internally delegates to the HTTP protocol implementation in `protocols/http.rs`.
5. **Protocol Trait** – `protocols/` defines a generic `Protocol` trait used by gateways, enabling pluggable encoders/decoders.
6. **Transformers** – `transform/` holds generic transformers (JSON, Protobuf) and can be extended with custom format conversions.
7. **Schema** – `schema/` offers discovery & generation abstractions, currently stubs.
8. **Observability** – Logging via `utils/logging.rs`, metrics placeholder via `utils/metrics.rs`.

---

## 3. Configuration

Located in `src/config/`.

| File | Role |
|------|------|
| `config/mod.rs` | Exposes `GatewayConfig`, `ProtocolConfig`, `RouteConfig`, and helper loaders. |
| `config/file.rs` | Reads YAML/JSON files (`config/gateway.yaml` by default). |
| `config/env.rs` | Stub for environment‑variable based config (future implementation). |

### `GatewayConfig`
```rust
pub struct GatewayConfig {
    pub port: u16,
    pub host: String,
    pub protocols: Vec<ProtocolConfig>,   // which protocols are enabled
    #[serde(default)]
    pub routes: Vec<RouteConfig>,         // routing table
}
```

### `ProtocolConfig`
```rust
pub struct ProtocolConfig {
    pub name: String,        // e.g. "http", "grpc"
    pub enabled: bool,
    pub settings: serde_json::Value,  // protocol‑specific settings
}
```

### `RouteConfig`
Provides URL routing & target mapping.
```rust
pub struct RouteConfig {
    pub path: String,          // prefix to match
    pub target: String,        // backend URL
    pub methods: Vec<String>,  // empty => all
    pub timeout_secs: Option<u64>,
    pub strip_prefix: Option<bool>,
}
```
> **Security note** – `target` is strictly validated (`HttpGateway::proxy` ensures http/https only and blocks `..` paths).

---

## 4. Protocol Support

All protocols implement the generic `protocols::Protocol` trait.

| Protocol | File | Notes |
|---|---|---|
| **HTTP** | `protocols/http.rs` | Pass‑through, size check, sanitises hop‑by‑hop headers |
| **gRPC** | `protocols/grpc.rs` | Stub – encode/decode currently no‑op |
| **GraphQL** | `protocols/graphql.rs` | Stub |
| **MQTT** | `protocols/mqtt.rs` | Stub |
| **WebSocket** | `protocols/ws.rs` | Stub |

The `Protocol` trait defines:
```rust
async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>>;
async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>>;
fn name(&self) -> &str;
```

---

## 5. Gateway Core

### 5.1 Core Traits

- **`core::Gateway`** – Starts, stops, lists protocols.
```rust
async fn start(&self) -> Result<()>;
async fn stop(&self) -> Result<()>;
fn protocols(&self) -> Vec<Arc<dyn Protocol>>;
```
- **`gateway::ProtocolGateway`** – Handles raw requests.
```rust
async fn handle_request(&self, request: Vec<u8>) -> Result<Vec<u8>>;
fn protocol(&self) -> Arc<dyn Protocol>;
```

### 5.2 Routing (`core/router.rs`)

- Routes sorted by path length (longest match wins).
- Validates HTTP methods (non‑ASCII control chars rejected).
- Provides `route(&self, method: &str, path: &str) -> Option<&RouteConfig>`.

### 5.3 HTTP Gateway (`gateway/http.rs`)

- Implements `ProtocolGateway`.
- Main operation is `proxy()` which:
  - Validates target base schema, forbids SSRF.
  - Sanitises headers (`protocols/http.rs::strip_hop_by_hop_headers`).
  - Uses `reqwest` (async client) to forward request.
  - Returns `(status_code, headers, body)` to be wrapped into a `Response` object.

---

## 6. Transformation Layer

`transform/` defines a generic transformer trait:
```rust
trait Transformer: Send + Sync {
    async fn transform(&self, input: Vec<u8>, from: &str, to: &str) -> Result<Vec<u8>>;
}
```

Currently two concrete transformers:

| Transformer | File | Function |
|------------|------|---------|
| Json | `transform/json.rs` | Echo (no‑op) |
| Protobuf | `transform/protobuf.rs` | Echo (no‑op) |

> **Future** – Replace or extend with actual serde/flatbuffers conversions.

---

## 7. Schema Management

`schema/` provides interfaces for schema discovery/generation.

```rust
pub struct Schema {
    name: String,
    version: String,
    content: String,
    protocol: String,
}
```

Traits for discovery and generation are defined in `schema/mod.rs`; the actual logic is in `schema/discovery.rs` and `schema/generation.rs`. These are current stubs (TODO) – developers can plug in service discovery or introspection logic.

---

## 8. Metrics & Logging

| Submodule | Purpose | Status |
|-----------|---------|--------|
| `utils/logging.rs` | Simple console logger using `tracing`.  Provides `info!`, `error!`, `debug!`. | **Implemented** – default init in `main.rs`. |
| `utils/metrics.rs` | Trait placeholder for request/ error counters. | **Stub** – needs real Prometheus integration. |

---

## 9. Testing & CI

- All modules have unit tests in `src/**/*.rs` (e.g., `core/router.rs`).  
- `cargo test` runs the entire suite.  
- CI (GitHub Actions) is configured in `.github/workflows/`.  Make sure tests pass before PR.

---

## 10. Contribution Workflow

1. **Fork** the repo and create a feature branch (`feature/…`).
2. **Set up** Rust toolchain 1.75+.  
3. **Run** `cargo test` locally.  
4. **Commit** changes with conventional commit style (e.g., `feat: add example protocol`).  
5. **Push** and open a PR against `main`.  
6. **CI** will run lint, test, build.  
7. **Review** by maintainers; keep commits lean.  
8. **Merge** after approvals.

> **Tip** – Add `#[cfg(test)]` modules to new files to keep test surface isolated.

---

## 11. Security & Hardening

- **SSRF Prevention** – all outbound URLs validated as HTTP/HTTPS; path traversal (`../`) rejected.
- **Method Sanitisation** – `Router::route` rejects non‑ASCII/control chars.
- **Header Sanitisation** – hop‑by‑hop headers removed before forwarding.
- **Timeouts** – user‑configurable per‑route (`timeout_secs`) and client‑side timeout (`reqwest` timeout).
- **Secrets** – the `config/env.rs` stub ensures env‑vars aren’t read accidentally; sensitive config should go into `gateway.yaml` only.

---

## 12. Future‑Proofing & Enhancements

| Area | Next Steps |
|------|------------|
| **Protocol Expansion** | Implement gRPC/GraphQL forwarding logic, WebSocket proxy. |
| **Real Metrics** | Wire `utils/metrics` to Prometheus via `prometheus` crate. |
| **Schema Engine** | Build discovery logic (OpenAPI for REST, introspection for GraphQL, gRPC reflection). |
| **Transformation Plugins** | Add JSON ↔ Protobuf conversion, custom transformers. |
| **Circuit Breaking** | Add `middleware::circuit_breaker` with fallback. |
| **CLI** | Advanced command‑line flags for runtime config overrides. |
| **Testing** | Add integration tests against mock backends for each protocol. |

---

## Appendix: Key Code Paths

| Module | Key File | Responsibility |
|--------|----------|----------------|
| **Gateway Core** | `src/core/mod.rs` | Trait definitions |
| **Configuration** | `src/config/mod.rs` | Loading config |
| **Routing** | `src/core/router.rs` | Path matching |
| **HTTP Gateway** | `src/gateway/http.rs` | HTTP proxying |
| **Protocol HTTP** | `src/protocols/http.rs` | HTTP encode/decode |
| **Transformer** | `src/transform/` | Data transformation |
| **Schema** | `src/schema/` | Schema struct and stubs |

> The module diagram can be visualised as per the architecture ASCII above.
