# Configuration Reference

IronBabel is configured entirely through a YAML file. The default location is `config/gateway.yaml` relative to the working directory. Override with the `IRON_BABEL_CONFIG` environment variable.

```sh
IRON_BABEL_CONFIG=/etc/ironbabel/gateway.yaml iron-babel
```

---

## Top-Level Fields

```yaml
host: "127.0.0.1"
port: 8080
protocols: [...]
routes: [...]
listeners: [...]
middleware:
  auth: {...}
  rate_limit: {...}
  logging: {...}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `host` | string | yes | IP address the gateway listens on. Use `"0.0.0.0"` to listen on all interfaces. Can be overridden with `IRON_BABEL_HOST`. |
| `port` | integer | yes | TCP port the gateway listens on. Valid range 1–65535. Can be overridden with `IRON_BABEL_PORT`. |
| `protocols` | array | yes | List of protocol descriptors. Declares which protocols are compiled into the gateway at startup. |
| `routes` | array | no | Routing table. Each entry maps a path prefix to a backend transport. Defaults to an empty list. |
| `listeners` | array | no | Inbound background listeners. Each entry binds a non-HTTP socket and forwards received frames to an HTTP URL. Defaults to an empty list. |
| `middleware` | object | no | Global middleware configuration. All sub-sections default to disabled. |

---

## `protocols` Section

The `protocols` list tells the gateway which protocol implementations to instantiate at startup. It does not directly control routing — that is handled by the `routes` section. Disabling a protocol here prevents its code from being loaded.

```yaml
protocols:
  - name: "http"
    enabled: true
    settings: {}
  - name: "grpc"
    enabled: true
    settings: {}
  - name: "graphql"
    enabled: true
    settings: {}
  - name: "websocket"
    enabled: true
    settings: {}
  - name: "zmq"
    enabled: true
    settings: {}
  - name: "mqtt"
    enabled: true
    settings: {}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Protocol identifier. One of: `http`, `grpc`, `graphql`, `websocket`, `zmq`, `mqtt`, `amqp`. |
| `enabled` | boolean | yes | When `false`, the protocol is not instantiated and cannot be used in routes. |
| `settings` | object | yes | Protocol-specific settings passed as a JSON value. Currently informational for most protocols. Use `{}` when no settings are needed. |

---

## `routes` Section

Each entry in `routes` maps a path prefix to a backend transport. The router selects the most specific (longest) prefix that matches the incoming request path, then checks whether the HTTP method is allowed.

```yaml
routes:
  - path: "/api/v1"
    methods: ["GET", "POST"]
    transport:
      type: http
      url: "http://127.0.0.1:9000"
      timeout_secs: 30
      strip_prefix: false
```

### Route Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | yes | Path prefix to match. The router checks that the incoming path starts with this value and is followed by `/` or end-of-string (preventing `/api` from matching `/apiv2`). |
| `methods` | string array | no | Allowed HTTP methods, case-insensitive. An empty list (`[]`) allows any method. |
| `transport` | object | yes | Transport configuration block. The `type` field determines which variant is used. |

### Routing Priority

Routes are sorted by path length (longest first). The first matching route wins. Given routes `/api` and `/api/v1`, a request to `/api/v1/users` matches `/api/v1`.

---

## Transport Types

The `transport.type` field selects the variant. Each variant has its own set of fields.

### `http` Transport

Proxies incoming HTTP requests to an HTTP/HTTPS backend. Query strings and response headers are forwarded. Hop-by-hop headers (`connection`, `keep-alive`, `transfer-encoding`, etc.) are stripped in both directions.

```yaml
transport:
  type: http
  url: "http://127.0.0.1:9000"
  timeout_secs: 30
  strip_prefix: false
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | string | required | Base URL of the upstream server. Must begin with `http://` or `https://`. The incoming path is appended to this value. |
| `timeout_secs` | integer | `30` | Per-request timeout in seconds. The gateway returns `504 Gateway Timeout` if the upstream does not respond within this time. |
| `strip_prefix` | boolean | `false` | When `true`, the matched route prefix is removed from the path before forwarding. For example, a request to `/api/v1/users` on a route with `path: "/api/v1"` would be forwarded as `/users`. |

### `zmq` Transport

Forwards the request body to a ZeroMQ endpoint. Three messaging patterns are supported.

```yaml
transport:
  type: zmq
  address: "127.0.0.1:5555"
  pattern: req_rep
  timeout_secs: 10
  topic: null
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | required | ZMQ endpoint in `host:port` form. Also accepted: `tcp://host:port` or `zmq://host:port`. |
| `pattern` | enum | required | ZMQ messaging pattern. One of `req_rep`, `push`, or `pub_sub`. |
| `timeout_secs` | integer | `30` | Timeout waiting for a reply (applies to `req_rep` only). |
| `topic` | string or null | `null` | Topic prefix prepended to the message frame for `pub_sub` pattern. Ignored by `req_rep` and `push`. |

**ZMQ Patterns:**

| Pattern | Socket type | Gateway returns | Use case |
|---------|-------------|-----------------|----------|
| `req_rep` | REQ socket connects to upstream REP | `200` with reply body | Synchronous RPC |
| `push` | PUSH socket connects to upstream PULL | `202 Accepted` immediately | Fire-and-forget events |
| `pub_sub` | PUB socket connects; topic prefixed to frame | `202 Accepted` immediately | Broadcasting to multiple subscribers |

### `graphql` Transport

Validates that the request body is a JSON object containing a `query` field, then forwards as HTTP POST with `Content-Type: application/json`.

```yaml
transport:
  type: graphql
  url: "http://api-service:4000/graphql"
  timeout_secs: 30
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | string | required | Full URL of the upstream GraphQL endpoint. Must begin with `http://` or `https://`. |
| `timeout_secs` | integer | `30` | Per-request timeout in seconds. |

The gateway rejects requests that are not valid JSON, not a JSON object, or missing the `query` field — returning `400 Bad Request` before reaching the upstream.

### `grpc` Transport

Wraps the raw request body in a gRPC length-prefixed frame (5-byte header: 1 byte compression flag + 4 bytes big-endian length), then forwards over HTTP/2 POST with `Content-Type: application/grpc`. The framing is stripped from the response body before returning it to the client.

```yaml
transport:
  type: grpc
  url: "http://grpc-service:50051"
  timeout_secs: 30
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | string | required | Base URL of the upstream gRPC server. Must begin with `http://` or `https://`. The incoming path (e.g. `/helloworld.Greeter/SayHello`) is appended. |
| `timeout_secs` | integer | `30` | Per-request timeout in seconds. |

The gRPC transport uses `http2_prior_knowledge` — it requires the upstream to speak HTTP/2 directly without the HTTP/1.1 upgrade dance.

### `websocket` Transport

Upgrades the incoming connection to a WebSocket and proxies frames bidirectionally to a backend WebSocket server. The upgrade is handled before the body is read.

```yaml
transport:
  type: websocket
  url: "ws://realtime-service:8080"
  timeout_secs: 30
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | string | required | Backend WebSocket URL. Accepted schemes: `ws://`, `wss://`, `http://` (converted to `ws://`), `https://` (converted to `wss://`), or a bare `host:port` (treated as `ws://`). |
| `timeout_secs` | integer | `30` | Connection establishment timeout. |

### `mqtt` Transport

Publishes the incoming HTTP request body to an MQTT broker topic and returns `202 Accepted` when the publish succeeds.

```yaml
transport:
  type: mqtt
  broker_url: "mqtt://broker:1883"
  topic: "events.http"
  qos: 1
  retain: false
  client_id: null
  timeout_secs: 10
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `broker_url` | string | required | MQTT broker URL. Accepted schemes: `mqtt://`, `mqtts://`, `tcp://`, `ssl://`. |
| `topic` | string | required | MQTT topic to publish to. |
| `qos` | integer | `0` | MQTT QoS level. Must be `0`, `1`, or `2`. |
| `retain` | boolean | `false` | Whether the published message should be retained by the broker. |
| `client_id` | string or null | `null` | Optional MQTT client ID. If omitted, the gateway generates one. |
| `timeout_secs` | integer | `30` | Maximum time to wait for the publish workflow to complete. |

### `amqp` Transport

Publishes the incoming HTTP request body to an AMQP broker and returns `202 Accepted` when the publish succeeds.

```yaml
transport:
  type: amqp
  broker_url: "amqp://guest:guest@rabbitmq:5672/%2f"
  exchange: ""
  routing_key: "events.http"
  mandatory: false
  persistent: true
  content_type: "application/octet-stream"
  timeout_secs: 10
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `broker_url` | string | required | AMQP broker URL. Accepted schemes: `amqp://` and `amqps://`. |
| `exchange` | string | `""` | Exchange to publish to. An empty string uses the default exchange. |
| `routing_key` | string | required | Routing key for the publish. |
| `mandatory` | boolean | `false` | Whether to request broker return handling for unroutable messages. |
| `persistent` | boolean | `true` | Whether the message should be published with persistent delivery mode. |
| `content_type` | string or null | `null` | Optional AMQP message content type property. |
| `timeout_secs` | integer | `30` | Maximum time to wait for the publish workflow to complete. |

---

## `listeners` Section

Listeners are background tasks that bind a non-HTTP inbound socket and forward each received frame as an HTTP POST to a configured target. Unlike routes (which handle client requests), listeners are server-side receivers.

### `zmq_pull` Listener

Binds a ZMQ PULL socket and POSTs each received frame to an HTTP URL.

```yaml
listeners:
  - type: zmq_pull
    bind: "127.0.0.1:5557"
    forward_to: "http://127.0.0.1:9000/zmq-webhook"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | Must be `zmq_pull`. |
| `bind` | string | yes | Address to bind the PULL socket on, in `host:port` form. |
| `forward_to` | string | yes | HTTP URL to POST each received frame to. Each frame is sent with `Content-Type: application/octet-stream` and an `X-ZMQ-Source: ironbabel-pull-listener` header. |

The listener uses a 30-second HTTP timeout for each forwarded frame. If the HTTP POST fails, an error is logged and the listener continues receiving subsequent frames.

### `mqtt_sub` Listener

Connects to an MQTT broker, subscribes to one or more topics, and POSTs each received payload to an HTTP URL.

```yaml
listeners:
  - type: mqtt_sub
    broker_url: "mqtt://broker:1883"
    topics: ["events.device"]
    qos: 1
    client_id: null
    forward_to: "http://127.0.0.1:9000/mqtt-webhook"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | Must be `mqtt_sub`. |
| `broker_url` | string | yes | MQTT broker URL. Accepted schemes: `mqtt://`, `mqtts://`, `tcp://`, `ssl://`. |
| `topics` | string array | yes | Topics to subscribe to. |
| `qos` | integer | no | MQTT QoS level for subscriptions. Must be `0`, `1`, or `2`. Defaults to `0`. |
| `client_id` | string or null | no | Optional MQTT client ID. If omitted, the gateway generates one. |
| `forward_to` | string | yes | HTTP URL to POST each received payload to. |

Each forwarded request includes `Content-Type: application/octet-stream` plus `X-MQTT-Source`, `X-MQTT-Topic`, `X-MQTT-QoS`, and `X-MQTT-Retain` headers.

### `amqp_consume` Listener

Connects to an AMQP broker, consumes messages from a queue, and POSTs each payload to an HTTP URL.

```yaml
listeners:
  - type: amqp_consume
    broker_url: "amqp://guest:guest@rabbitmq:5672/%2f"
    queue: "events.inbox"
    consumer_tag: null
    auto_ack: false
    forward_to: "http://127.0.0.1:9000/amqp-webhook"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | Must be `amqp_consume`. |
| `broker_url` | string | yes | AMQP broker URL. Accepted schemes: `amqp://` and `amqps://`. |
| `queue` | string | yes | Queue name to consume from. |
| `consumer_tag` | string or null | no | Optional consumer tag. If omitted, the gateway generates one. |
| `auto_ack` | boolean | no | Whether the broker should auto-acknowledge deliveries. Defaults to `false`. |
| `forward_to` | string | yes | HTTP URL to POST each received payload to. |

Each forwarded request includes `Content-Type: application/octet-stream` plus `X-AMQP-Source`, `X-AMQP-Exchange`, `X-AMQP-Routing-Key`, and `X-AMQP-Delivery-Tag` headers. When `auto_ack` is `false`, successful webhook delivery results in `ack`; failures result in `nack` with requeue enabled.

---

## `middleware` Section

The middleware section configures two built-in middleware components. Both are global — they apply to every route.

```yaml
middleware:
  auth:
    enabled: false
    api_keys: []
  rate_limit:
    enabled: false
    requests_per_window: 100
    window_secs: 60
```

### `auth` Sub-section

Enforces Bearer token authentication. The `Authorization` header value must be in the form `Bearer <token>`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `false` | When `false`, all requests pass through regardless of the `Authorization` header. |
| `api_keys` | string array | `[]` | List of valid Bearer token values. When `enabled` is `true` and this list is non-empty, a request must carry one of these tokens. When the list is empty and `enabled` is `true`, any token (or no token) is accepted. |

Behavior matrix:

| `enabled` | `api_keys` | Result |
|-----------|-----------|--------|
| `false` | any | All requests allowed |
| `true` | empty | All requests allowed |
| `true` | non-empty | Requests without a matching token → `401 Unauthorized` |

### `rate_limit` Sub-section

Enforces per-client request rate limits using a sliding window algorithm. The client key is the verified remote socket IP address; if it is unavailable, requests fall back to a shared `"global"` bucket.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `false` | When `false`, no rate limiting is applied. |
| `requests_per_window` | integer | `100` | Maximum number of requests allowed within the window period. |
| `window_secs` | integer | `60` | Sliding window size in seconds. |

When a client exceeds the limit, the gateway returns `429 Too Many Requests`.

---

## Complete Example

The following is the full `config/gateway.yaml` shipped with the repository:

```yaml
host: "127.0.0.1"
port: 8080

protocols:
  - name: "http"
    enabled: true
    settings:
      port: 8081
  - name: "grpc"
    enabled: true
    settings:
      port: 8082
  - name: "graphql"
    enabled: true
    settings:
      port: 8083
  - name: "mqtt"
    enabled: true
    settings:
      client_id: "iron-babel-mqtt"
  - name: "amqp"
    enabled: true
    settings: {}

middleware:
  auth:
    enabled: false
    # api_keys:
    #   - "my-secret-token"
  rate_limit:
    enabled: false
    requests_per_window: 100
    window_secs: 60
  logging:
    enabled: false

routes:
  - path: "/api/v1"
    methods: ["GET", "POST", "PUT", "DELETE"]
    transport:
      type: http
      url: "http://127.0.0.1:9000"
      timeout_secs: 30

  - path: "/api"
    methods: ["GET", "POST"]
    transport:
      type: http
      url: "http://127.0.0.1:9001"
      timeout_secs: 30

  - path: "/health"
    methods: []
    transport:
      type: http
      url: "http://127.0.0.1:9000"
      timeout_secs: 5

  - path: "/zmq/orders"
    methods: ["POST"]
    transport:
      type: zmq
      address: "127.0.0.1:5555"
      pattern: req_rep
      timeout_secs: 10

  - path: "/zmq/events"
    methods: ["POST"]
    transport:
      type: zmq
      address: "127.0.0.1:5556"
      pattern: push

  # ZMQ PUB/SUB example (uncomment to use):
  # - path: "/zmq/broadcast"
  #   methods: ["POST"]
  #   transport:
  #     type: zmq
  #     address: "127.0.0.1:5558"
  #     pattern: pub_sub
  #     topic: "orders.created"

  - path: "/mqtt/events"
    methods: ["POST"]
    transport:
      type: mqtt
      broker_url: "mqtt://127.0.0.1:1883"
      topic: "events.http"
      qos: 1
      retain: false
      timeout_secs: 10

  - path: "/amqp/events"
    methods: ["POST"]
    transport:
      type: amqp
      broker_url: "amqp://guest:guest@127.0.0.1:5672/%2f"
      exchange: ""
      routing_key: "events.http"
      mandatory: false
      persistent: true
      timeout_secs: 10

listeners:
  - type: zmq_pull
    bind: "127.0.0.1:5557"
    forward_to: "http://127.0.0.1:9000/zmq-webhook"
  - type: mqtt_sub
    broker_url: "mqtt://127.0.0.1:1883"
    topics: ["events.device"]
    qos: 1
    forward_to: "http://127.0.0.1:9000/mqtt-webhook"
  - type: amqp_consume
    broker_url: "amqp://guest:guest@127.0.0.1:5672/%2f"
    queue: "events.inbox"
    auto_ack: false
    forward_to: "http://127.0.0.1:9000/amqp-webhook"
```

---

## Global Request Limits

Regardless of per-route `timeout_secs`, a global 30-second `TimeoutLayer` is applied at the tower middleware level. Any request that has not completed in 30 seconds receives a `504 Gateway Timeout` response.

Request bodies are capped at **10 MB**. Requests with bodies larger than this limit are rejected with `413 Payload Too Large` before any route matching occurs.
