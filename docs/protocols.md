# Protocols

IronBabel supports five transport protocols for routing incoming HTTP requests to backend services: HTTP, GraphQL, gRPC, WebSocket, and ZeroMQ. Each protocol has a corresponding implementation in `src/protocols/` (encoding/validation logic) and `src/gateway/` (proxy/connection logic).

---

## Protocol Architecture

Every protocol implements the `Protocol` trait:

```rust
trait Protocol: Send + Sync {
    fn name(&self) -> &str;
    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>>;
    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>>;
}
```

`encode` is called on the inbound request body before it is forwarded upstream. `decode` is called on the upstream response body before it is returned to the client. Each gateway implementation (`ProtocolGateway`) holds a reference to its corresponding `Protocol` and calls `encode`/`decode` as part of the proxy lifecycle.

---

## HTTP Protocol

**Source files:** `src/protocols/http.rs`, `src/gateway/http.rs`

### What it does

The HTTP gateway is the most general transport. It forwards incoming HTTP requests (including query strings and sanitized headers) to an upstream HTTP or HTTPS server and relays the response back to the client.

### How proxying works

1. The gateway validates that the configured target URL scheme is `http://` or `https://`. Any other scheme is rejected immediately (SSRF mitigation).
2. The request path is checked for path traversal sequences (`..`, `%2e%2e`, mixed encodings). A traversal is detected in both the request handler and inside `HttpGateway::proxy` for defence in depth.
3. Hop-by-hop headers (`connection`, `keep-alive`, `proxy-authenticate`, `proxy-authorization`, `te`, `trailers`, `transfer-encoding`, `upgrade`) are stripped from both the request and response.
4. The `host` header is removed so `reqwest` can set it correctly for the upstream host.
5. An `X-Forwarded-For: gateway` header is added to the outbound request.
6. The response headers are sanitized (hop-by-hop headers stripped) before being forwarded to the client.

### Body size limit

The `HttpProtocol` encoder rejects bodies larger than 10 MB. A separate tower `RequestBodyLimitLayer` enforces the same cap at the server layer before routing occurs.

### Security considerations

- Target URLs are always taken from the YAML configuration — request data can never alter the destination (SSRF prevention).
- Path traversal is rejected before the request reaches the upstream.
- Non-visible-ASCII header values are filtered out in the request handler before the body is read.
- Only `http://` and `https://` schemes are forwarded; `file://`, `ftp://`, and others are rejected.

### Example route config

```yaml
- path: "/api/v1"
  methods: ["GET", "POST", "PUT", "DELETE"]
  transport:
    type: http
    url: "http://backend-service:9000"
    timeout_secs: 30
    strip_prefix: false
```

With `strip_prefix: false` (the default), a request to `/api/v1/users` is forwarded as `http://backend-service:9000/api/v1/users`.

With `strip_prefix: true`, the same request is forwarded as `http://backend-service:9000/users`.

---

## GraphQL Protocol

**Source files:** `src/protocols/graphql.rs`, `src/gateway/graphql.rs`

### What it does

The GraphQL gateway validates that the request body is a legal GraphQL-over-HTTP request and forwards it to an upstream GraphQL endpoint as HTTP POST with `Content-Type: application/json`.

### How proxying works

1. The request body is parsed as JSON. If parsing fails, `400 Bad Request` is returned immediately.
2. The parsed value must be a JSON object. Arrays or scalar values are rejected.
3. The object must contain a `"query"` key. This handles queries, mutations, and subscriptions — all are expressed as `query`-keyed objects per the GraphQL-over-HTTP specification.
4. The validated body is forwarded as an HTTP POST to the configured URL with `Content-Type: application/json` forced (the client's value is ignored).
5. The `host`, `content-type`, and `content-length` headers are stripped from the forwarded request; all other non-hop-by-hop headers are passed through.

### Wire format

Inbound request from client:
```
POST /graphql HTTP/1.1
Content-Type: application/json

{"query": "{ users { id name } }", "variables": {}}
```

Forwarded to upstream:
```
POST http://api-service:4000/graphql HTTP/1.1
Content-Type: application/json
X-Forwarded-For: gateway

{"query": "{ users { id name } }", "variables": {}}
```

The response is passed through as-is (hop-by-hop headers stripped). GraphQL errors delivered as `200` with an `errors` key in the JSON body are passed through transparently — the gateway does not inspect response payloads.

### Security considerations

- Target URL must be `http://` or `https://` (enforced in `GraphQLGateway::proxy`).
- Request body is validated before forwarding; invalid JSON is rejected at the gateway, reducing load on the upstream.
- The `query` field requirement prevents accidentally routing non-GraphQL traffic through a GraphQL route.

### Example route config

```yaml
- path: "/graphql"
  methods: ["POST"]
  transport:
    type: graphql
    url: "http://api-service:4000/graphql"
    timeout_secs: 30
```

### Example client request

```sh
curl -X POST http://127.0.0.1:8080/graphql \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer my-token" \
  -d '{"query": "{ users { id name email } }"}'
```

---

## gRPC Protocol

**Source files:** `src/protocols/grpc.rs`, `src/gateway/grpc.rs`

### What it does

The gRPC gateway wraps the raw request body in a gRPC length-prefixed frame, forwards it over HTTP/2 to an upstream gRPC server, and strips the framing from the response before returning raw bytes to the client.

### gRPC wire format

The gRPC protocol specifies a 5-byte length-delimited framing header prepended to each message:

```
Byte 0:     Compression flag (0 = uncompressed, 1 = compressed)
Bytes 1-4:  Message length as a 4-byte big-endian unsigned integer
Bytes 5+:   Raw protobuf payload
```

Example: encoding the 5-byte protobuf payload `\x0a\x05hello`:

```
[00]                 # no compression
[00 00 00 05]        # length = 5
[0a 05 68 65 6c 6c 6f]  # protobuf payload
```

The gateway:
- **On request:** calls `GrpcProtocol::encode`, which prepends the 5-byte header with compression flag `0`.
- **On response:** calls `GrpcProtocol::decode`, which validates and strips the 5-byte header, returning the raw protobuf payload.

### How proxying works

1. The target URL scheme must be `http://` or `https://`. Other schemes are rejected.
2. The request path is checked for traversal sequences.
3. The raw request body is wrapped in a gRPC frame.
4. An HTTP/2 connection is established with `http2_prior_knowledge` (no upgrade negotiation).
5. The framed body is POSTed with `Content-Type: application/grpc` and `TE: trailers`.
6. The `content-type`, `content-length`, `te`, `host`, and all hop-by-hop headers are stripped from the forwarded request; other headers pass through.
7. The framing is stripped from the response body before returning to the client.

The incoming request path is appended to the configured base URL to form the full gRPC method path. For example, a request to `/helloworld.Greeter/SayHello` on a route with `url: "http://grpc-service:50051"` is forwarded to `http://grpc-service:50051/helloworld.Greeter/SayHello`.

### gRPC response decoding errors

If the upstream returns a response body that does not match the expected frame format (too short, non-zero compression flag, or truncated payload), `decode` returns an empty byte slice rather than an error. The HTTP status code from the upstream is preserved.

### Compression

Only uncompressed frames (compression flag `0`) are supported. Frames with a non-zero compression flag are rejected with an error. This means compressed gRPC streams are not currently supported.

### Security considerations

- Target URL is always from the configuration file.
- Path traversal is checked before forwarding.
- The HTTP/2 requirement limits the attack surface compared to HTTP/1.1 upgrade paths.
- Only scheme-validated URLs are forwarded.

### Example route config

```yaml
- path: "/helloworld.Greeter"
  methods: ["POST"]
  transport:
    type: grpc
    url: "http://grpc-service:50051"
    timeout_secs: 30
```

### Example client request

The client must send a gRPC-framed protobuf body. If the client is a standard gRPC tool, it will handle framing. If sending raw bytes with curl for testing:

```sh
# Build a minimal gRPC frame: [0x00][0x00 0x00 0x00 0x05][payload bytes]
printf '\x00\x00\x00\x00\x05\x0a\x03\x42\x6f\x62' | \
  curl -X POST http://127.0.0.1:8080/helloworld.Greeter/SayHello \
    --data-binary @- \
    -H "Content-Type: application/grpc" \
    -H "Authorization: Bearer my-token"
```

---

## WebSocket Protocol

**Source files:** `src/protocols/ws.rs`, `src/gateway/ws.rs`

### What it does

The WebSocket gateway performs a bidirectional frame proxy between the client and a backend WebSocket server. All message types (text, binary, ping, pong) are forwarded. The session remains open until either side sends a close frame or disconnects.

### How proxying works

1. When a WebSocket upgrade is detected on an incoming request, it is handled before the body is consumed (WebSocket upgrades have no body in the HTTP sense).
2. The backend URL is normalized: `http://` becomes `ws://`, `https://` becomes `wss://`, bare `host:port` is treated as `ws://`.
3. The gateway connects to the backend using `tokio-tungstenite`.
4. Two concurrent tasks run:
   - **Client → backend:** reads frames from the client WebSocket, converts them to tungstenite format, and sends to the backend.
   - **Backend → client:** reads frames from the backend, converts them to axum format, and sends to the client.
5. When either direction closes (close frame or error), the other direction is also closed.

### Message type mapping

| Client frame type | Forwarded as |
|-------------------|-------------|
| Text | Text |
| Binary | Binary |
| Ping | Ping |
| Pong | Pong |
| Close | Connection closed |

### Security considerations

- The backend URL comes from the configuration file only; the client cannot influence where the WebSocket connects.
- Invalid backend URL schemes (e.g., `ftp://`) are rejected when the session starts, before any data is exchanged.
- The WebSocket upgrade is validated by axum before proxying begins; non-upgrade requests to a WebSocket route receive `426 Upgrade Required`.

### Example route config

```yaml
- path: "/ws"
  methods: []
  transport:
    type: websocket
    url: "ws://realtime-service:8080"
    timeout_secs: 30
```

### Example client connection

```sh
# Using websocat (https://github.com/vi/websocat)
websocat ws://127.0.0.1:8080/ws
```

Or with JavaScript in a browser:

```js
const ws = new WebSocket("ws://127.0.0.1:8080/ws");
ws.onmessage = (e) => console.log("received:", e.data);
ws.send("hello");
```

---

## ZeroMQ Protocol

**Source files:** `src/protocols/zmq.rs`, `src/gateway/zmq.rs`

### What it does

The ZMQ gateway bridges HTTP clients to ZeroMQ backends. It supports three patterns with different semantics, and also supports inbound listening (forwarding ZMQ frames to HTTP).

### Address format

Addresses in `transport.address` can be in any of these forms; all are normalized internally to `tcp://host:port`:

- `"127.0.0.1:5555"` → `tcp://127.0.0.1:5555`
- `"zmq://127.0.0.1:5555"` → `tcp://127.0.0.1:5555`
- `"tcp://127.0.0.1:5555"` → `tcp://127.0.0.1:5555`

### REQ/REP Pattern

The gateway opens a REQ socket, connects to the upstream REP socket, sends the request body as a single frame, and waits for a reply frame. The reply bytes are returned to the HTTP client with status `200` and `Content-Type: application/octet-stream`.

If no reply arrives within `timeout_secs`, the gateway returns `502 Bad Gateway` with a timeout error message.

```yaml
transport:
  type: zmq
  address: "127.0.0.1:5555"
  pattern: req_rep
  timeout_secs: 10
```

A new REQ socket is created per HTTP request. Concurrent requests do not share socket state.

### PUSH Pattern

The gateway opens a PUSH socket, connects to an upstream PULL socket, sends the request body as a single frame, and immediately returns `202 Accepted` to the HTTP client. No reply is waited for.

```yaml
transport:
  type: zmq
  address: "127.0.0.1:5556"
  pattern: push
```

### PUB/SUB Pattern

The gateway opens a PUB socket, connects to an upstream subscriber, and sends a single frame containing an optional topic prefix followed by the request body. Returns `202 Accepted` immediately.

If `topic` is set in the config, the frame is structured as `[topic_bytes][body_bytes]` in a single frame. Subscribers filter using `set_subscribe` matching the topic prefix.

```yaml
transport:
  type: zmq
  address: "127.0.0.1:5558"
  pattern: pub_sub
  topic: "orders.created"
```

### Inbound ZMQ PULL Listener

Listeners work in the opposite direction: the gateway binds a PULL socket and receives frames sent by external publishers. Each received frame is forwarded to an HTTP URL as a POST with:
- `Content-Type: application/octet-stream`
- `X-ZMQ-Source: ironbabel-pull-listener`

```yaml
listeners:
  - type: zmq_pull
    bind: "127.0.0.1:5557"
    forward_to: "http://127.0.0.1:9000/zmq-webhook"
```

The listener runs as a background Tokio task and forwards each frame in a separate spawned task. If the HTTP POST fails, the error is logged and the listener continues with the next frame.

### Frame size limit

The `ZmqProtocol` encoder and decoder both reject frames larger than 10 MB.

### Security considerations

- ZMQ addresses are always taken from the configuration; clients cannot redirect frames to arbitrary endpoints.
- Each request creates a fresh socket, preventing state from leaking between requests.
- The 10 MB frame size cap prevents excessively large messages from consuming memory.

### Example client requests

REQ/REP (synchronous):
```sh
curl -X POST http://127.0.0.1:8080/zmq/orders \
  -H "Content-Type: application/json" \
  -d '{"order_id": "ord-001", "amount": 49.99}'
# → 200 OK with reply from ZMQ upstream
```

PUSH (fire-and-forget):
```sh
curl -X POST http://127.0.0.1:8080/zmq/events \
  -H "Content-Type: application/json" \
  -d '{"event": "user.signup", "user_id": "u-123"}'
# → 202 Accepted
```

---

## MQTT Protocol

**Source files:** `src/protocols/mqtt.rs`

The MQTT protocol implementation is currently a passthrough stub — `encode` and `decode` return the data unchanged. MQTT routing through the gateway is not yet implemented. The protocol descriptor can be listed in the `protocols` section, but no route transport type exists for MQTT yet.

---

## Protocol Comparison Summary

| Protocol | Direction | Blocking | Response body | Status on success |
|----------|-----------|----------|---------------|-------------------|
| HTTP | client → upstream | yes | upstream response | upstream status |
| GraphQL | client → upstream (HTTP POST) | yes | upstream JSON | upstream status |
| gRPC | client → upstream (HTTP/2 POST) | yes | decoded protobuf | upstream status |
| WebSocket | bidirectional | no (persistent) | N/A (streaming) | `101 Switching Protocols` |
| ZMQ req_rep | client → upstream | yes | ZMQ reply bytes | `200 OK` |
| ZMQ push | client → upstream | no | empty | `202 Accepted` |
| ZMQ pub_sub | client → upstream | no | empty | `202 Accepted` |
