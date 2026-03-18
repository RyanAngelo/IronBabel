# Admin Dashboard and API

IronBabel includes a built-in admin interface that provides real-time observability into the gateway without requiring any external monitoring infrastructure. The admin interface is available as both a browser-accessible dashboard and a JSON API.

---

## Accessing the Dashboard

Once the gateway is running, navigate to:

```
http://<host>:<port>/admin/
```

For the default configuration (`host: 127.0.0.1`, `port: 8080`):

```
http://127.0.0.1:8080/admin/
```

The dashboard is a single-page HTML application served directly from the gateway binary (the HTML is embedded at compile time from `assets/admin/index.html`). It uses `htmx` for data fetching and `Chart.js` for time-series charts. No external build step or asset serving is required.

The dashboard auto-refreshes metrics data periodically and maintains a live stream of request events via Server-Sent Events (SSE).

---

## Admin API Endpoints

All admin endpoints are served under the `/admin/` path prefix. They are not protected by the auth middleware — if you enable API key authentication in your config, the admin API is **not** subject to that middleware. See the Security Guide for hardening recommendations.

### `GET /admin/api/health`

Returns the gateway health status and basic runtime information.

**Response schema:**

```json
{
  "status": "ok",
  "uptime_secs": 3742,
  "version": "0.1.0",
  "active_routes": 5
}
```

| Field | Type | Description |
|-------|------|-------------|
| `status` | string | Always `"ok"` when the gateway is running. |
| `uptime_secs` | integer | Seconds elapsed since the gateway process started. |
| `version` | string | Gateway version from `Cargo.toml`. |
| `active_routes` | integer | Number of routes defined in the current configuration. |

**Example:**

```sh
curl -s http://127.0.0.1:8080/admin/api/health | jq
```

```json
{
  "status": "ok",
  "uptime_secs": 142,
  "version": "0.1.0",
  "active_routes": 5
}
```

---

### `GET /admin/api/metrics`

Returns aggregated metrics for the current gateway session, including request counts, latency percentiles, status code distribution, per-route statistics, and time-series data for charts.

**Response schema:**

```json
{
  "total_requests": 1042,
  "rps": 2.4,
  "error_rate": 0.012,
  "p50_latency_ms": 18.0,
  "p95_latency_ms": 87.0,
  "p99_latency_ms": 210.0,
  "status_code_counts": {
    "200": 980,
    "401": 42,
    "404": 15,
    "502": 5
  },
  "requests_by_route": {
    "/api/v1": 860,
    "/health": 182
  },
  "rps_series": [
    {"timestamp_secs": 1742300000, "value": 3.0},
    {"timestamp_secs": 1742300001, "value": 2.0}
  ],
  "latency_series": [
    {"timestamp_secs": 1742300000, "value": 21.5},
    {"timestamp_secs": 1742300001, "value": 18.0}
  ]
}
```

| Field | Type | Description |
|-------|------|-------------|
| `total_requests` | integer | Total requests processed since startup. |
| `rps` | float | Current requests per second, computed over the most recent 5-second window. |
| `error_rate` | float | Fraction of total requests that resulted in a 5xx status code or an internal error. Range 0.0–1.0. |
| `p50_latency_ms` | float | 50th percentile (median) request latency in milliseconds, over the last 10,000 requests. |
| `p95_latency_ms` | float | 95th percentile request latency in milliseconds. |
| `p99_latency_ms` | float | 99th percentile request latency in milliseconds. |
| `status_code_counts` | object | Map of HTTP status code (as string) to count. |
| `requests_by_route` | object | Map of matched route path to total request count. Requests that did not match any route are not included. |
| `rps_series` | array of `BucketPoint` | Time-series of per-second request counts, up to the last 60 seconds. |
| `latency_series` | array of `BucketPoint` | Time-series of average latency per second, up to the last 60 seconds. |

**`BucketPoint` schema:**

| Field | Type | Description |
|-------|------|-------------|
| `timestamp_secs` | integer | Unix timestamp in seconds. |
| `value` | float | Request count (for `rps_series`) or average latency in ms (for `latency_series`). |

**Notes on time buckets:**
- Buckets are maintained at 1-second granularity.
- Up to 60 buckets are retained (the last 60 seconds).
- A background task ticks every second to ensure the bucket timeline advances even during idle periods.
- The RPS calculation uses only the last 5 seconds to reflect recent load, not the full 60-second history.

**Example:**

```sh
curl -s http://127.0.0.1:8080/admin/api/metrics | jq '.p95_latency_ms, .rps'
```

---

### `GET /admin/api/routes`

Returns the list of configured routes along with per-route runtime statistics.

**Response schema:** array of route objects.

```json
[
  {
    "path": "/api/v1",
    "transport_type": "http",
    "target": "http://127.0.0.1:9000",
    "methods": ["GET", "POST", "PUT", "DELETE"],
    "timeout_secs": 30,
    "total_requests": 860,
    "error_count": 3,
    "avg_latency_ms": 22.4
  },
  {
    "path": "/zmq/orders",
    "transport_type": "zmq",
    "target": "127.0.0.1:5555",
    "methods": ["POST"],
    "timeout_secs": 10,
    "total_requests": 47,
    "error_count": 0,
    "avg_latency_ms": 4.1
  }
]
```

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Route path prefix as configured. |
| `transport_type` | string | One of `http`, `zmq`, `graphql`, `grpc`, `websocket`. |
| `target` | string | Upstream target address or URL. |
| `methods` | string array | Allowed HTTP methods. Empty array means all methods are allowed. |
| `timeout_secs` | integer | Configured timeout for this route. |
| `total_requests` | integer | Total requests matched to this route since startup. |
| `error_count` | integer | Number of requests to this route that resulted in a 5xx status or internal error. |
| `avg_latency_ms` | float | Mean latency in milliseconds across all requests to this route. `0.0` if no requests have been made. |

**Example:**

```sh
curl -s http://127.0.0.1:8080/admin/api/routes | jq '.[] | {path, transport_type, avg_latency_ms}'
```

---

### `GET /admin/api/requests/recent`

Returns the most recent requests processed by the gateway, in reverse chronological order (newest first).

**Query parameters:**

| Parameter | Type | Default | Maximum | Description |
|-----------|------|---------|---------|-------------|
| `n` | integer | `50` | `500` | Number of recent requests to return. |

**Response schema:** array of request log entries.

```json
[
  {
    "id": 1042,
    "timestamp_ms": 1742300042000,
    "method": "GET",
    "path": "/api/v1/users",
    "matched_route": "/api/v1",
    "status_code": 200,
    "latency_ms": 18,
    "upstream_target": "http://127.0.0.1:9000",
    "error": null
  },
  {
    "id": 1041,
    "timestamp_ms": 1742300038000,
    "method": "POST",
    "path": "/api/v1/orders",
    "matched_route": "/api/v1",
    "status_code": 502,
    "latency_ms": 5003,
    "upstream_target": "http://127.0.0.1:9000",
    "error": "connection refused"
  }
]
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | integer | Monotonically increasing request ID, starting at 1. |
| `timestamp_ms` | integer | Unix timestamp in milliseconds when the request was completed. |
| `method` | string | HTTP method of the incoming request. |
| `path` | string | Full path of the incoming request. |
| `matched_route` | string or null | The route path prefix that matched, or `null` for unmatched requests (404s, path traversal rejections). |
| `status_code` | integer | HTTP status code returned to the client. |
| `latency_ms` | integer | Total gateway processing time in milliseconds, from request receipt to response sent. |
| `upstream_target` | string | The upstream address or URL that the request was forwarded to. Empty string for pre-routing rejections. |
| `error` | string or null | Error message if an internal error occurred, otherwise `null`. Present on 5xx responses and internal failures. |

The gateway retains up to 10,000 recent requests in memory in a ring buffer. Older entries are discarded as new ones arrive.

**Example:**

```sh
# Get the 10 most recent requests
curl -s "http://127.0.0.1:8080/admin/api/requests/recent?n=10" | jq

# Get only error responses
curl -s "http://127.0.0.1:8080/admin/api/requests/recent?n=500" | \
  jq '[.[] | select(.status_code >= 400)]'
```

---

### `GET /admin/events`

Server-Sent Events stream. Each event is a JSON-serialized `RequestLogEntry` delivered immediately after the gateway finishes processing the request.

**Response:** `text/event-stream` (SSE protocol)

**Event format:**

```
data: {"id":1042,"timestamp_ms":1742300042000,"method":"GET","path":"/api/v1/users","matched_route":"/api/v1","status_code":200,"latency_ms":18,"upstream_target":"http://127.0.0.1:9000","error":null}

data: {"id":1043,"timestamp_ms":1742300043000,...}
```

Each event is a single `data:` line containing the JSON payload, followed by a blank line as specified by the SSE protocol. The `event:` field is not set; clients should use the `message` event type.

Keep-alive frames are sent automatically at the default `axum` SSE interval to prevent proxy timeouts on idle connections.

The broadcast channel backing SSE has a capacity of 1,024 events. If a subscriber falls more than 1,024 events behind (e.g., a slow consumer), older events are dropped for that subscriber.

**Example with curl:**

```sh
curl -N -H "Accept: text/event-stream" http://127.0.0.1:8080/admin/events
```

**Example with JavaScript (EventSource):**

```js
const evtSource = new EventSource("http://127.0.0.1:8080/admin/events");
evtSource.onmessage = function(event) {
  const entry = JSON.parse(event.data);
  console.log(`[${entry.status_code}] ${entry.method} ${entry.path} — ${entry.latency_ms}ms`);
};
```

---

## Dashboard UI

The browser dashboard at `/admin/` renders:

- **Health badge** (top navigation): green when the gateway is responding, red otherwise.
- **Stat cards**: total requests, requests per second, error rate, P50/P95/P99 latency.
- **Charts**: a 60-second RPS time series and a 60-second average latency time series (auto-refreshed).
- **Routes table**: all configured routes with transport type, target, method list, request count, error count, and average latency.
- **Live request log**: a table populated via the SSE stream at `/admin/events`. New entries are prepended as requests complete.

The dashboard is self-contained — all JavaScript is loaded from CDN (htmx, htmx-sse extension, Chart.js). It requires outbound internet access to load these scripts. If the deployment environment is air-gapped, the asset file at `assets/admin/index.html` should be modified to use locally hosted script files before compiling.

---

## Programmatic Polling Example

The following shell script polls the metrics endpoint every 5 seconds and prints a one-line summary:

```sh
while true; do
  METRICS=$(curl -s http://127.0.0.1:8080/admin/api/metrics)
  RPS=$(echo "$METRICS" | jq -r '.rps')
  P95=$(echo "$METRICS" | jq -r '.p95_latency_ms')
  ERR=$(echo "$METRICS" | jq -r '.error_rate')
  echo "$(date '+%H:%M:%S') rps=${RPS} p95=${P95}ms error_rate=${ERR}"
  sleep 5
done
```
