#!/usr/bin/env python3
"""
IronBabel Gateway Test Script
==============================
Spins up lightweight mock upstream servers on the ports configured in
gateway.yaml, fires a suite of HTTP requests through the gateway, and
prints a colour-coded report of every hop — including ZMQ scenarios.

Usage:
    python scripts/test_gateway.py              # run built-in test suite
    python scripts/test_gateway.py --continuous # keep sending traffic (Ctrl-C to stop)
    python scripts/test_gateway.py --gateway http://127.0.0.1:8080
    python scripts/test_gateway.py --no-upstream  # if your real upstreams are already up
    python scripts/test_gateway.py --no-zmq       # skip ZMQ scenarios
    python scripts/test_gateway.py --mqtt         # add HTTP -> MQTT route scenario
    python scripts/test_gateway.py --amqp         # add HTTP -> AMQP route scenario

Requires only Python 3.8+ standard library for HTTP tests.
ZMQ scenarios additionally require:  pip install pyzmq
"""

import argparse
import json
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Optional

# ── Optional ZMQ support ──────────────────────────────────────────────────────
try:
    import zmq
    HAS_ZMQ = True
except ImportError:
    HAS_ZMQ = False

# ─────────────────────────────────────────────────────────────────────────────
# Terminal colours
# ─────────────────────────────────────────────────────────────────────────────
try:
    import os
    _USE_COLOUR = sys.stdout.isatty() or os.environ.get("FORCE_COLOR")
except Exception:
    _USE_COLOUR = False

def _c(code: str, text: str) -> str:
    return f"\033[{code}m{text}\033[0m" if _USE_COLOUR else text

def green(t):  return _c("32", t)
def red(t):    return _c("31", t)
def yellow(t): return _c("33", t)
def cyan(t):   return _c("36", t)
def bold(t):   return _c("1",  t)
def dim(t):    return _c("2",  t)

# ─────────────────────────────────────────────────────────────────────────────
# Mock HTTP upstream server
# ─────────────────────────────────────────────────────────────────────────────
class EchoHandler(BaseHTTPRequestHandler):
    """Echoes every request back as a JSON blob. Also stores webhook calls
    made by the gateway's ZMQ PULL → HTTP forwarding path."""

    # Class-level store for ZMQ→HTTP webhook calls so tests can verify them.
    zmq_webhooks: list = []

    def log_message(self, fmt, *args):
        pass

    def _read_body(self) -> bytes:
        length = int(self.headers.get("content-length", 0))
        return self.rfile.read(length) if length else b""

    def _respond(self, status: int, payload: dict):
        body = json.dumps(payload, indent=2).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("X-Upstream-Port", str(self.server.server_address[1]))
        self.end_headers()
        self.wfile.write(body)

    def _handle(self):
        raw_body = self._read_body()
        try:
            parsed_body = json.loads(raw_body) if raw_body else None
        except json.JSONDecodeError:
            parsed_body = raw_body.decode(errors="replace")

        # Track webhook calls from the ZMQ PULL listener.
        if self.path == "/zmq-webhook":
            EchoHandler.zmq_webhooks.append({
                "path": self.path,
                "body": parsed_body,
                "raw": raw_body,
            })

        payload = {
            "upstream_port": self.server.server_address[1],
            "received": {
                "method": self.command,
                "path": self.path,
                "headers": dict(self.headers),
                "body": parsed_body,
            },
            "reply": "echo from mock upstream",
        }
        status = 200 if self.command != "DELETE" else 204
        self._respond(status, payload)

    do_GET     = _handle
    do_POST    = _handle
    do_PUT     = _handle
    do_PATCH   = _handle
    do_DELETE  = _handle


def start_upstream(port: int) -> HTTPServer:
    server = HTTPServer(("127.0.0.1", port), EchoHandler)
    t = threading.Thread(target=server.serve_forever, daemon=True)
    t.start()
    return server


# ─────────────────────────────────────────────────────────────────────────────
# Mock ZMQ upstream servers
# ─────────────────────────────────────────────────────────────────────────────

def start_zmq_rep_echo(port: int) -> threading.Thread:
    """ZMQ REP server that echoes each frame back as JSON.
    Used to test the gateway's REQ/REP pattern (HTTP → ZMQ)."""
    def _run():
        ctx = zmq.Context()
        sock = ctx.socket(zmq.REP)
        sock.bind(f"tcp://127.0.0.1:{port}")
        while True:
            try:
                raw = sock.recv()
                try:
                    payload = json.loads(raw)
                except Exception:
                    payload = raw.decode(errors="replace")
                reply = json.dumps({
                    "zmq_upstream": f"tcp://127.0.0.1:{port}",
                    "received": payload,
                    "reply": "echo from zmq rep",
                }).encode()
                sock.send(reply)
            except zmq.ContextTerminated:
                break
            except Exception:
                break
    t = threading.Thread(target=_run, daemon=True)
    t.start()
    return t


def start_zmq_pull_receiver(port: int) -> tuple:
    """ZMQ PULL server that collects received frames into a shared list.
    Used to verify the gateway's PUSH pattern (HTTP → ZMQ)."""
    received: list = []

    def _run():
        ctx = zmq.Context()
        sock = ctx.socket(zmq.PULL)
        sock.bind(f"tcp://127.0.0.1:{port}")
        sock.setsockopt(zmq.RCVTIMEO, 200)  # 200 ms poll interval
        while True:
            try:
                raw = sock.recv()
                received.append(raw)
            except zmq.Again:
                continue
            except zmq.ContextTerminated:
                break
            except Exception:
                break

    t = threading.Thread(target=_run, daemon=True)
    t.start()
    return t, received


def zmq_push(port: int, message: bytes) -> bool:
    """Push a single ZMQ frame to the gateway's bound PULL socket.
    Used to test ZMQ → HTTP forwarding (zmq_listeners config)."""
    ctx = zmq.Context()
    sock = ctx.socket(zmq.PUSH)
    sock.connect(f"tcp://127.0.0.1:{port}")
    time.sleep(0.05)  # let connect settle
    try:
        sock.send(message)
        return True
    except Exception:
        return False
    finally:
        sock.close()
        ctx.destroy(linger=0)


# ─────────────────────────────────────────────────────────────────────────────
# HTTP helpers
# ─────────────────────────────────────────────────────────────────────────────
def http(
    method: str,
    url: str,
    body: Optional[dict] = None,
    headers: Optional[dict] = None,
    timeout: int = 10,
) -> tuple:
    data = json.dumps(body).encode() if body is not None else None
    hdrs = {"Content-Type": "application/json", "Accept": "application/json"}
    if headers:
        hdrs.update(headers)
    if data is None:
        hdrs.pop("Content-Type", None)

    req = urllib.request.Request(url, data=data, headers=hdrs, method=method)
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            elapsed = (time.perf_counter() - t0) * 1000
            raw = resp.read()
            try:
                return resp.status, json.loads(raw), elapsed
            except json.JSONDecodeError:
                return resp.status, {"_raw": raw.decode(errors="replace")}, elapsed
    except urllib.error.HTTPError as e:
        elapsed = (time.perf_counter() - t0) * 1000
        raw = e.read()
        try:
            return e.code, json.loads(raw), elapsed
        except Exception:
            return e.code, {"_raw": raw.decode(errors="replace")}, elapsed
    except urllib.error.URLError as e:
        elapsed = (time.perf_counter() - t0) * 1000
        return 0, {"_error": str(e.reason)}, elapsed


# ─────────────────────────────────────────────────────────────────────────────
# Test scenarios
# ─────────────────────────────────────────────────────────────────────────────
def make_http_scenarios(gateway: str) -> list:
    return [
        # ── /api/v1 → upstream :9000 ──────────────────────────────────────────
        {
            "label": "GET  /api/v1/users          → HTTP :9000",
            "method": "GET",
            "url": f"{gateway}/api/v1/users",
        },
        {
            "label": "POST /api/v1/orders         → HTTP :9000  (JSON body)",
            "method": "POST",
            "url": f"{gateway}/api/v1/orders",
            "body": {"item": "widget", "qty": 3, "price_cents": 999},
        },
        {
            "label": "PUT  /api/v1/users/42       → HTTP :9000  (update)",
            "method": "PUT",
            "url": f"{gateway}/api/v1/users/42",
            "body": {"name": "Alice", "role": "admin"},
        },
        {
            "label": "DELETE /api/v1/sessions/99  → HTTP :9000",
            "method": "DELETE",
            "url": f"{gateway}/api/v1/sessions/99",
        },
        # ── /api → upstream :9001 ─────────────────────────────────────────────
        {
            "label": "GET  /api/status            → HTTP :9001",
            "method": "GET",
            "url": f"{gateway}/api/status",
        },
        {
            "label": "POST /api/events            → HTTP :9001  (JSON body)",
            "method": "POST",
            "url": f"{gateway}/api/events",
            "body": {"type": "user.signup", "user_id": "u_abc123", "timestamp": int(time.time())},
        },
        # ── /health ───────────────────────────────────────────────────────────
        {
            "label": "GET  /health                → HTTP :9000  (any method)",
            "method": "GET",
            "url": f"{gateway}/health",
        },
        # ── Custom headers ────────────────────────────────────────────────────
        {
            "label": "GET  /api/v1/secure         → HTTP :9000  (custom headers)",
            "method": "GET",
            "url": f"{gateway}/api/v1/secure",
            "headers": {"Authorization": "Bearer test-token", "X-Request-ID": "req-12345"},
        },
        # ── Expected 404 ──────────────────────────────────────────────────────
        {
            "label": "GET  /unknown/path          → 404 (no route)",
            "method": "GET",
            "url": f"{gateway}/unknown/path",
            "expect_status": 404,
        },
    ]


def make_zmq_scenarios(gateway: str, pull_received: list) -> list:
    return [
        # ── REQ/REP: HTTP → ZMQ → reply back as HTTP response ─────────────────
        {
            "label": "POST /zmq/orders  → ZMQ REQ/REP :5555  (sync reply)",
            "method": "POST",
            "url": f"{gateway}/zmq/orders",
            "body": {"order_id": "ord-001", "amount": 99.99, "currency": "USD"},
            "check": lambda status, body: (
                status == 200
                and isinstance(body, dict)
                and body.get("zmq_upstream") == "tcp://127.0.0.1:5555"
            ),
            "check_desc": "status=200, body contains zmq_upstream echo",
        },
        {
            "label": "POST /zmq/orders  → ZMQ REQ/REP :5555  (raw text payload)",
            "method": "POST",
            "url": f"{gateway}/zmq/orders",
            "body": {"message": "hello zmq", "ts": int(time.time())},
            "check": lambda status, body: status == 200,
            "check_desc": "status=200",
        },
        # ── PUSH: HTTP → ZMQ fire-and-forget (202 + message arrives at receiver)
        {
            "label": "POST /zmq/events  → ZMQ PUSH :5556   (fire-and-forget, 202)",
            "method": "POST",
            "url": f"{gateway}/zmq/events",
            "body": {"event": "user.click", "user_id": "u-123", "ts": int(time.time())},
            "expect_status": 202,
            "post_check": lambda: _verify_push_received(pull_received, "user.click"),
            "post_check_desc": "frame arrives at ZMQ PULL receiver",
        },
        {
            "label": "POST /zmq/events  → ZMQ PUSH :5556   (large payload)",
            "method": "POST",
            "url": f"{gateway}/zmq/events",
            "body": {"records": [{"id": i, "data": f"item-{i}"} for i in range(50)]},
            "expect_status": 202,
        },
    ]


def make_broker_scenarios(gateway: str, mqtt_expect_status: int, amqp_expect_status: int,
                          include_mqtt: bool, include_amqp: bool) -> list:
    scenarios = []

    if include_mqtt:
        scenarios.append({
            "label": f"POST /mqtt/events → MQTT publish route  (expect {mqtt_expect_status})",
            "method": "POST",
            "url": f"{gateway}/mqtt/events",
            "body": {"event": "device.ping", "device_id": "sim-001", "ts": int(time.time())},
            "expect_status": mqtt_expect_status,
        })

    if include_amqp:
        scenarios.append({
            "label": f"POST /amqp/events → AMQP publish route  (expect {amqp_expect_status})",
            "method": "POST",
            "url": f"{gateway}/amqp/events",
            "body": {"event": "job.created", "job_id": "job-001", "ts": int(time.time())},
            "expect_status": amqp_expect_status,
        })

    return scenarios


def _verify_push_received(pull_received: list, expected_event: str, wait_secs: float = 1.0) -> bool:
    """Poll until the PUSH receiver has seen a frame containing expected_event."""
    deadline = time.time() + wait_secs
    while time.time() < deadline:
        for raw in pull_received:
            try:
                obj = json.loads(raw)
                if obj.get("event") == expected_event:
                    return True
            except Exception:
                pass
        time.sleep(0.05)
    return False


# ─────────────────────────────────────────────────────────────────────────────
# ZMQ → HTTP listener test (gateway PULL socket → HTTP webhook)
# ─────────────────────────────────────────────────────────────────────────────
def run_zmq_listener_test(gateway_pull_port: int) -> tuple:
    """
    Push a ZMQ frame directly to the gateway's bound PULL socket on
    `gateway_pull_port`. The gateway should forward it as an HTTP POST to
    http://127.0.0.1:9000/zmq-webhook, which our EchoHandler captures.

    Returns (passed: bool, label: str, detail: str).
    """
    label = f"ZMQ PUSH :5557 → gateway PULL → HTTP POST :9000/zmq-webhook"
    payload = {"source": "zmq-listener-test", "ts": int(time.time()), "data": "hello from zmq"}
    frame = json.dumps(payload).encode()

    EchoHandler.zmq_webhooks.clear()
    ok = zmq_push(gateway_pull_port, frame)
    if not ok:
        return False, label, "failed to push ZMQ frame"

    # Give the gateway time to forward the HTTP call.
    deadline = time.time() + 3.0
    while time.time() < deadline:
        if EchoHandler.zmq_webhooks:
            wh = EchoHandler.zmq_webhooks[0]
            received_body = wh.get("body") or {}
            if isinstance(received_body, dict) and received_body.get("source") == "zmq-listener-test":
                return True, label, f"webhook received at /zmq-webhook with correct payload"
            return True, label, f"webhook received (body={wh.get('body')})"
        time.sleep(0.1)

    return False, label, "webhook not received within 3s (is gateway zmq_listener running?)"


# ─────────────────────────────────────────────────────────────────────────────
# Reporting
# ─────────────────────────────────────────────────────────────────────────────
def status_colour(code: int) -> str:
    if code == 0:    return red("ERR")
    if code < 300:   return green(str(code))
    if code < 400:   return cyan(str(code))
    if code == 404:  return yellow(str(code))
    return red(str(code))


def print_result(scenario: dict, status: int, body: dict, elapsed_ms: float) -> bool:
    expected = scenario.get("expect_status")
    custom_check = scenario.get("check")

    if custom_check:
        ok = custom_check(status, body)
    elif expected is not None:
        ok = status == expected
    else:
        ok = 200 <= status < 300

    marker = green("✓") if ok else red("✗")
    print(f"  {marker}  {bold(scenario['label'])}")
    print(f"       status={status_colour(status)}  latency={elapsed_ms:.1f}ms", end="")

    if scenario.get("check_desc"):
        print(f"  [{scenario['check_desc']}]", end="")
    print()

    if "_error" in body:
        print(f"       {red('error:')} {body['_error']}")
    elif "received" in body:
        r = body["received"]
        up = body.get("upstream_port") or body.get("zmq_upstream", "?")
        print(f"       upstream={up}  method={r.get('method','?')}  path={r.get('path','?')}")
        if r.get("body"):
            snippet = json.dumps(r["body"])
            if len(snippet) > 80:
                snippet = snippet[:77] + "…"
            print(f"       body={dim(snippet)}")
    elif "_raw" in body:
        snippet = body["_raw"][:120]
        print(f"       {dim(snippet)}")

    # Post-check (e.g. verify ZMQ PUSH receiver got the frame)
    post_check = scenario.get("post_check")
    if post_check:
        post_ok = post_check()
        post_marker = green("✓") if post_ok else red("✗")
        post_desc = scenario.get("post_check_desc", "post-check")
        print(f"       {post_marker} {post_desc}")
        ok = ok and post_ok

    print()
    return ok


def print_zmq_listener_result(passed: bool, label: str, detail: str):
    marker = green("✓") if passed else red("✗")
    print(f"  {marker}  {bold(label)}")
    print(f"       {detail}")
    print()
    return passed


def print_admin_metrics(gateway: str):
    status, body, _ = http("GET", f"{gateway}/admin/api/metrics")
    if status != 200:
        print(f"  {yellow('⚠')}  Could not reach admin metrics (is the gateway running?)\n")
        return

    print(bold("  Admin metrics snapshot:"))
    print(f"    total_requests : {body.get('total_requests', 0)}")
    print(f"    rps            : {body.get('rps', 0):.2f}")
    err_pct = body.get('error_rate', 0) * 100
    colour = red if err_pct > 5 else green
    print(f"    error_rate     : {colour(f'{err_pct:.1f}%')}")
    print(f"    p50 latency    : {body.get('p50_latency_ms', 0):.1f} ms")
    print(f"    p95 latency    : {body.get('p95_latency_ms', 0):.1f} ms")
    counts = body.get("status_code_counts", {})
    if counts:
        print(f"    status codes   : {'  '.join(f'{k}:{v}' for k, v in sorted(counts.items()))}")
    print()


def wait_for_gateway(gateway: str, retries: int = 10) -> bool:
    print(f"  Waiting for gateway at {gateway} ", end="", flush=True)
    for _ in range(retries):
        try:
            with urllib.request.urlopen(f"{gateway}/admin/api/health", timeout=2):
                print(green(" ready"))
                return True
        except Exception:
            print(".", end="", flush=True)
            time.sleep(0.5)
    print(red(" timed out"))
    return False


# ─────────────────────────────────────────────────────────────────────────────
# Suite runners
# ─────────────────────────────────────────────────────────────────────────────
def run_suite(gateway: str, pull_received: list, run_zmq: bool, include_mqtt: bool,
              include_amqp: bool, mqtt_expect_status: int, amqp_expect_status: int) -> tuple:
    passed = failed = 0

    print(bold("  HTTP scenarios"))
    print(dim("  " + "─" * 56))
    print()
    for s in make_http_scenarios(gateway):
        status, body, elapsed = http(s["method"], s["url"], body=s.get("body"), headers=s.get("headers"))
        ok = print_result(s, status, body, elapsed)
        if ok: passed += 1
        else: failed += 1

    broker_scenarios = make_broker_scenarios(
        gateway,
        mqtt_expect_status,
        amqp_expect_status,
        include_mqtt,
        include_amqp,
    )
    if broker_scenarios:
        print(bold("  MQTT / AMQP scenarios"))
        print(dim("  " + "─" * 56))
        print()
        for s in broker_scenarios:
            status, body, elapsed = http(s["method"], s["url"], body=s.get("body"), headers=s.get("headers"))
            ok = print_result(s, status, body, elapsed)
            if ok: passed += 1
            else: failed += 1

    if not run_zmq:
        return passed, failed

    print(bold("  ZMQ scenarios"))
    print(dim("  " + "─" * 56))
    print()

    if not HAS_ZMQ:
        print(f"  {yellow('⚠')}  pyzmq not installed — skipping ZMQ scenarios")
        print(f"     Install with:  pip install pyzmq\n")
        return passed, failed

    for s in make_zmq_scenarios(gateway, pull_received):
        status, body, elapsed = http(s["method"], s["url"], body=s.get("body"))
        ok = print_result(s, status, body, elapsed)
        if ok: passed += 1
        else: failed += 1

    # ZMQ → HTTP listener test
    print(bold("  ZMQ → HTTP listener"))
    print(dim("  " + "─" * 56))
    print()
    ok, label, detail = run_zmq_listener_test(5557)
    ok = print_zmq_listener_result(ok, label, detail)
    if ok: passed += 1
    else: failed += 1

    return passed, failed


def run_continuous(gateway: str, delay: float = 0.5, include_mqtt: bool = False, include_amqp: bool = False,
                   mqtt_expect_status: int = 502, amqp_expect_status: int = 502):
    scenarios = make_http_scenarios(gateway)
    scenarios.extend(make_broker_scenarios(
        gateway,
        mqtt_expect_status,
        amqp_expect_status,
        include_mqtt,
        include_amqp,
    ))
    idx = 0
    total = 0
    print(f"  Sending continuous traffic to {gateway}  (Ctrl-C to stop)\n")
    try:
        while True:
            s = scenarios[idx % len(scenarios)]
            status, _, elapsed = http(s["method"], s["url"], body=s.get("body"))
            ok = 200 <= status < 300 or status == 404
            marker = green("✓") if ok else red("✗")
            path_ = urllib.parse.urlparse(s["url"]).path
            print(f"  {marker} [{total+1:4d}]  {s['method']:6s}  {path_:<30s}  "
                  f"→ {status_colour(status)}  {elapsed:.0f}ms")
            idx += 1
            total += 1
            time.sleep(delay)
    except KeyboardInterrupt:
        print(f"\n  Stopped after {total} requests.\n")


# ─────────────────────────────────────────────────────────────────────────────
# Entry point
# ─────────────────────────────────────────────────────────────────────────────
def main():
    parser = argparse.ArgumentParser(
        description="IronBabel gateway test script",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--gateway", default="http://127.0.0.1:8080")
    parser.add_argument("--no-upstream", action="store_true",
                        help="Skip starting mock HTTP upstream servers")
    parser.add_argument("--no-zmq", action="store_true",
                        help="Skip ZMQ mock servers and ZMQ test scenarios")
    parser.add_argument("--continuous", action="store_true",
                        help="Loop continuously (HTTP only; Ctrl-C to stop)")
    parser.add_argument("--delay", type=float, default=0.5,
                        help="Delay between requests in continuous mode")
    parser.add_argument("--mqtt", action="store_true",
                        help="Add HTTP -> MQTT publish route scenarios")
    parser.add_argument("--amqp", action="store_true",
                        help="Add HTTP -> AMQP publish route scenarios")
    parser.add_argument("--mqtt-expect-status", type=int, default=502,
                        help="Expected HTTP status for the MQTT publish route scenario")
    parser.add_argument("--amqp-expect-status", type=int, default=502,
                        help="Expected HTTP status for the AMQP publish route scenario")
    parser.add_argument("--upstream-ports", nargs="+", type=int, default=[9000, 9001])
    parser.add_argument("--zmq-rep-port",  type=int, default=5555)
    parser.add_argument("--zmq-push-port", type=int, default=5556)
    args = parser.parse_args()

    print()
    print(bold("═" * 62))
    print(bold("  IronBabel Gateway Test"))
    print(bold("═" * 62))
    print()

    # ── Start mock HTTP upstreams ─────────────────────────────────────────────
    if not args.no_upstream:
        for port in args.upstream_ports:
            try:
                start_upstream(port)
                print(f"  {green('▶')} Mock HTTP upstream started on :{port}")
            except OSError as e:
                print(f"  {yellow('⚠')}  Could not bind :{port} — {e}")
        print()
        time.sleep(0.1)

    # ── Start mock ZMQ servers ────────────────────────────────────────────────
    pull_received: list = []
    run_zmq = not args.no_zmq

    if run_zmq:
        if not HAS_ZMQ:
            print(f"  {yellow('⚠')}  pyzmq not found — ZMQ mock servers will not start")
            print(f"     Install with:  {dim('pip install pyzmq')}")
            print()
        else:
            try:
                start_zmq_rep_echo(args.zmq_rep_port)
                print(f"  {green('▶')} ZMQ REP echo server  started on :{args.zmq_rep_port}  (req_rep upstream)")
            except Exception as e:
                print(f"  {yellow('⚠')}  ZMQ REP server failed: {e}")

            try:
                _, pull_received = start_zmq_pull_receiver(args.zmq_push_port)
                print(f"  {green('▶')} ZMQ PULL receiver    started on :{args.zmq_push_port}  (push upstream)")
            except Exception as e:
                print(f"  {yellow('⚠')}  ZMQ PULL receiver failed: {e}")

            print(f"  {dim('(ZMQ PULL listener on :5557 is bound by the gateway itself)')}")
            print()
            time.sleep(0.1)

    # ── Wait for gateway ──────────────────────────────────────────────────────
    if not wait_for_gateway(args.gateway):
        print(f"\n  {red('Gateway is not reachable.')} Start IronBabel first:\n")
        print(f"    cargo run\n")
        sys.exit(1)
    print()

    # ── Run ───────────────────────────────────────────────────────────────────
    if args.continuous:
        run_continuous(
            args.gateway,
            delay=args.delay,
            include_mqtt=args.mqtt,
            include_amqp=args.amqp,
            mqtt_expect_status=args.mqtt_expect_status,
            amqp_expect_status=args.amqp_expect_status,
        )
        return

    passed, failed = run_suite(
        args.gateway,
        pull_received,
        run_zmq,
        args.mqtt,
        args.amqp,
        args.mqtt_expect_status,
        args.amqp_expect_status,
    )

    print(bold("═" * 62))
    print_admin_metrics(args.gateway)

    total = passed + failed
    pct = int(passed / total * 100) if total else 0
    colour = green if failed == 0 else (yellow if failed < 3 else red)
    print(f"  {colour(f'{passed}/{total} passed ({pct}%)')}")
    if failed:
        print(f"  {red(str(failed) + ' failed')}")
    print()
    print(f"  Dashboard: {cyan(args.gateway + '/admin/')}")
    print()

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
