# Security Guide

IronBabel is designed as a security-aware gateway. This document describes every built-in protection, how each is implemented, how to configure it, the threat model the gateway operates under, and recommendations for hardening a production deployment.

---

## Threat Model

IronBabel sits between untrusted HTTP clients and trusted backend services. The threat model assumes:

- **Clients are untrusted.** Request bodies, headers, paths, query strings, and methods may be malicious.
- **Backend targets are trusted.** Configuration YAML is written by an operator with access to the system. If an attacker can modify `gateway.yaml`, the system is already compromised.
- **The gateway process is trusted.** IronBabel does not implement multi-tenant isolation; all routes share the same middleware and backend access.
- **Network is partially trusted.** TLS is supported for upstream connections (`https://`, `wss://`), but the gateway does not currently terminate TLS for inbound connections (there is no built-in HTTPS listener). A TLS-terminating reverse proxy such as nginx or a cloud load balancer should be placed in front for production use.

---

## Built-in Protections

### 1. SSRF Prevention (Server-Side Request Forgery)

**What it prevents:** An attacker crafting a request that causes the gateway to connect to an unintended target — including internal services, metadata endpoints (e.g., `http://169.254.169.254`), or resources on other protocols.

**How it works:**

All outbound target URLs are taken exclusively from the YAML configuration. Request data (path, headers, body) never influences the destination URL or scheme. The `HttpGateway::proxy`, `GrpcGateway::proxy`, and `GraphQLGateway::proxy` methods each enforce:

```rust
if !target_base.starts_with("http://") && !target_base.starts_with("https://") {
    return Err(Error::Protocol("target_base must start with http:// or https://"));
}
```

Non-HTTP schemes (`file://`, `ftp://`, `gopher://`, `ldap://`, etc.) are rejected before any connection attempt.

ZMQ targets are likewise taken from the configuration and never from request data.

**Operator responsibility:** Ensure that configured URLs point only to internal, trusted services. Do not allow user-provided input to be written into `gateway.yaml`.

---

### 2. Path Traversal Prevention

**What it prevents:** An attacker appending `../` sequences (raw or percent-encoded) to a request path to read files or access paths outside the intended scope on the upstream server.

**How it works:**

The `path_contains_traversal` function in `src/protocols/http.rs` iteratively decodes percent-encoded sequences and then splits the path on `/` to check for `..` segments. It catches:

- Raw `..`: `/api/../etc/passwd`
- Percent-encoded: `/%2e%2e/etc/passwd`, `/.%2e/etc`, `/%2e./etc`
- Double-encoded: `/%252e%252e/etc` (decoded to `/%2e%2e/etc` then to `/../etc`)

The check runs in two places for defence-in-depth:
1. In `handle_request` (the main Axum handler) — before route matching. Path traversal attempts return `400 Bad Request` immediately and are recorded in metrics.
2. Inside `HttpGateway::proxy` and `GrpcGateway::proxy` — as a safety check even if called directly (e.g., in tests).

No configuration is required. Path traversal protection is always active.

---

### 3. Header Injection and Hop-by-Hop Header Stripping

**What it prevents:**
- Header injection attacks via non-visible ASCII characters in header values.
- Leaking connection-management headers that should not cross proxy boundaries.

**How it works:**

Before the request body is extracted, the handler filters out any header whose value cannot be converted to a valid UTF-8 string using `.to_str().ok()`. Header values containing non-visible ASCII (control characters, null bytes, CR/LF) are silently dropped.

Hop-by-hop headers are stripped in both directions (request and response) by `strip_hop_by_hop_headers` in `src/protocols/http.rs`. The stripped headers are:

- `connection`
- `keep-alive`
- `proxy-authenticate`
- `proxy-authorization`
- `te`
- `trailers`
- `transfer-encoding`
- `upgrade`

The `host` header is additionally stripped before forwarding, allowing `reqwest` to set the correct `Host` for the target server.

For gRPC routes, the gateway also strips `content-type`, `content-length`, and `te` from the forwarded request, then sets them explicitly to the correct values (`application/grpc` and `trailers`).

No configuration is required.

---

### 4. Method Injection Prevention

**What it prevents:** Requests where the HTTP method string contains control characters, newlines, or non-ASCII bytes that could exploit downstream parsers.

**How it works:**

The router's `route` method rejects any method string that contains characters outside the `[A-Za-z0-9]` set:

```rust
if !method.chars().all(|c| c.is_ascii_alphanumeric()) {
    return None;
}
```

A method like `GET\r\nX-Injected: evil` returns `None` (404) rather than matching any route.

No configuration is required.

---

### 5. Request Body Size Limit

**What it prevents:** Memory exhaustion through excessively large request bodies.

**How it works:**

Two independent limits are enforced:

1. **Tower layer:** `RequestBodyLimitLayer::new(10 * 1024 * 1024)` — applied at the HTTP server layer before any handler runs. Bodies exceeding 10 MB result in a `413 Payload Too Large` response.
2. **Handler:** `axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)` — a second cap inside the request handler that also returns `413` if exceeded.

The `HttpProtocol::encode` and `ZmqProtocol::encode` methods additionally check the body size and return errors for payloads over 10 MB.

No configuration is required. The 10 MB limit is currently hard-coded.

---

### 6. Global Request Timeout

**What it prevents:** Slowloris-style attacks, upstream hangs, or stalled connections consuming gateway threads indefinitely.

**How it works:**

A `TimeoutLayer` is applied at the tower middleware level with a 30-second global timeout and a `504 Gateway Timeout` status code:

```rust
TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, Duration::from_secs(30))
```

Per-route `timeout_secs` values (configured in each transport block) provide additional, shorter timeouts at the upstream connection level via `reqwest`'s `ClientBuilder::timeout`.

No configuration is required for the global timeout. Per-route timeouts default to 30 seconds and can be shortened in `gateway.yaml`.

---

### 7. Bearer Token Authentication

**What it prevents:** Unauthorized clients accessing any route without a valid credential.

**How it works:**

When `middleware.auth.enabled` is `true` and `api_keys` is non-empty, the `AuthMiddleware` checks every request for an `Authorization: Bearer <token>` header. The token is compared against the configured list. Any request without the header, with the wrong format, or with an invalid token receives `401 Unauthorized` before reaching the upstream.

The comparison is a direct string equality check — no hashing or timing-safe comparison is currently implemented. Tokens should be long, randomly generated values to mitigate brute force risk.

```yaml
middleware:
  auth:
    enabled: true
    api_keys:
      - "efd3a4b2-9e1c-4f8a-b7d6-123456789abc"
      - "f9a1c2d3-5e6b-4c7d-8a9b-fedcba987654"
```

If `api_keys` is empty but `enabled` is `true`, all tokens are accepted. This configuration is not useful in practice.

**Important:** The admin API endpoints under `/admin/` are **not** subject to the auth middleware. They are served by a separate router that is mounted before the main request handler. See hardening recommendations below.

---

### 8. Rate Limiting

**What it prevents:** Brute force attacks, denial-of-service via request flooding, API abuse by a single client.

**How it works:**

The `RateLimitMiddleware` uses an in-memory sliding window per client. The client is identified by the value of the `X-Forwarded-For` header. If the header is absent, all requests fall into a single `"global"` bucket.

When a client exceeds `requests_per_window` within a `window_secs` sliding window, subsequent requests receive `429 Too Many Requests` until the window rolls forward.

```yaml
middleware:
  rate_limit:
    enabled: true
    requests_per_window: 100
    window_secs: 60
```

**Limitations:**
- Rate limit state is in-process memory. It resets on restart and is not shared across multiple gateway instances.
- The client key is `X-Forwarded-For`. In environments without a trusted reverse proxy setting this header, clients can spoof their key by sending a crafted `X-Forwarded-For` value.
- A single "global" bucket is used when `X-Forwarded-For` is absent, meaning all clients without this header share one rate limit.

**Recommendation:** If running behind a reverse proxy, ensure the proxy sets `X-Forwarded-For` to the actual client IP and that the gateway cannot be reached directly from untrusted networks.

---

## Hardening Recommendations

### Restrict admin access

The admin API (`/admin/`) is unprotected by the auth middleware and exposes request logs, route configuration, and latency metrics. In production:

1. Place a reverse proxy (nginx, Caddy, etc.) in front of IronBabel and restrict access to `/admin/` by IP or require separate authentication at the proxy level.
2. Alternatively, bind the gateway to a non-public interface (e.g., `host: "127.0.0.1"`) and access the admin interface via SSH tunnel or a VPN.

### Use TLS for all connections

IronBabel does not terminate TLS for inbound connections. Use a TLS-terminating reverse proxy for all production deployments to ensure that credentials (Bearer tokens) and request data are not transmitted in cleartext.

For upstream connections, use `https://` and `wss://` target URLs wherever the upstream supports TLS.

### Set strong, unique API keys

When auth is enabled, use randomly generated tokens of at least 32 bytes:

```sh
openssl rand -hex 32
```

Rotate keys by updating `gateway.yaml` and restarting the gateway. There is no hot-reload of configuration.

### Minimize the `api_keys` list

Each key in the list is a credential. Remove keys that are no longer in use. Use separate keys for different calling services so individual keys can be rotated without affecting all clients.

### Tune timeouts aggressively

Set `timeout_secs` to the shortest value that is still realistic for your upstream. Short timeouts limit the damage from slow or hung backends and reduce the window for slowloris attacks:

```yaml
- path: "/health"
  transport:
    type: http
    url: "http://backend:9000"
    timeout_secs: 3   # health checks should be fast
```

### Bind to internal addresses

Unless the gateway is the only public-facing component, bind to an internal address:

```yaml
host: "10.0.0.5"  # internal network address
port: 8080
```

Set the actual public address in the reverse proxy or load balancer configuration.

### Treat ZMQ addresses as sensitive

ZMQ endpoints are unencrypted and unauthenticated by default. Ensure ZMQ `address` values in your configuration point to loopback or trusted internal addresses. Do not expose ZMQ sockets on public interfaces.

### Monitor the admin API for anomalies

The `/admin/api/requests/recent` endpoint provides a full request log including paths and upstream targets. Use the SSE stream (`/admin/events`) to feed a security information and event management (SIEM) system or alerting pipeline to detect patterns such as unusual 401/403 rates, unexpected route activity, or latency spikes.

---

## Security Feature Matrix

| Feature | Always active | Configurable | Location |
|---------|--------------|--------------|----------|
| SSRF prevention (scheme validation) | yes | no | `gateway/http.rs`, `gateway/grpc.rs`, `gateway/graphql.rs` |
| Path traversal prevention | yes | no | `protocols/http.rs`, `gateway/http.rs`, `gateway/grpc.rs` |
| Header injection filtering | yes | no | `core/gateway.rs` |
| Hop-by-hop header stripping | yes | no | `protocols/http.rs` |
| Method injection prevention | yes | no | `core/router.rs` |
| Request body size limit (10 MB) | yes | no | `core/gateway.rs`, tower layer |
| Global request timeout (30s) | yes | no | tower layer |
| Per-route timeouts | yes | via `timeout_secs` | each transport config |
| Bearer token auth | no | `middleware.auth` | `core/middleware/auth.rs` |
| Rate limiting (sliding window) | no | `middleware.rate_limit` | `core/middleware/rate_limit.rs` |
| Admin API access control | no | via reverse proxy | — |
| Inbound TLS | no | via reverse proxy | — |
