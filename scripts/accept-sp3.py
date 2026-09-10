#!/usr/bin/env python3
"""IntelHub SP3 acceptance — Unified Console (spec 2026-09-09-intelhub-console-3-design.md).

Covers: static serving + SPA fallback + asset caching + traversal safety,
auth boundaries (static public / api+mcp protected), all 8 console API
endpoints, per-page backing data, SSE live event stream, L3 policy deny.

Usage: accept-sp3.py <agent_key>
"""
import json, sys, time, urllib.request, urllib.error

HUB = "http://10.10.10.41:8800"
KEY = sys.argv[1]
PASS = FAIL = 0

def check(name, ok, detail=""):
    global PASS, FAIL
    if ok: PASS += 1; print(f"PASS {name}  | {detail}")
    else: FAIL += 1; print(f"FAIL {name}  | {detail}")

def get(path, key=None, timeout=30):
    r = urllib.request.Request(HUB + path)
    if key: r.add_header("Authorization", f"Bearer {key}")
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, resp.read().decode(), dict(resp.headers)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(), {}

print("== SP3 acceptance ==")

# ── static serving ───────────────────────────────────────────────────
st, body, hdrs = get("/")
check("GET / serves console", st == 200 and "IntelHub Console" in body and 'id="root"' in body, f"{st}")
st, body, _ = get("/investigations/some-uuid/alerts")
check("SPA fallback on deep route", st == 200 and 'id="root"' in body, f"{st}")
asset = None
st0, body0, _ = get("/")
import re
m = re.search(r'src="(/assets/[^"]+)"', body0)
if m:
    asset = m.group(1)
    st, _, hdrs = get(asset)
    cc = next((v for k, v in hdrs.items() if k.lower() == "cache-control"), "")
    check("hashed asset 200 + immutable cache", st == 200 and "immutable" in cc, f"{asset} cc={cc[:40]}")
st, body, _ = get("/assets/nonexistent.js")
check("missing asset → 404 (no SPA fallback for assets)", st == 404, f"{st}")
st, body, _ = get("/%2e%2e/%2e%2e/etc/passwd")
check("encoded traversal harmless (404 or shell, never file content)", st in (400, 404) or "root:" not in body, f"{st}")

# ── auth boundaries (§24) ────────────────────────────────────────────
st, _, _ = get("/api/v1/overview")
check("/api without key → 401", st == 401, f"{st}")
st, _, _ = get("/api/v1/overview", KEY)
check("/api with key → 200", st == 200, f"{st}")
st, _, _ = get("/healthz")
check("/healthz public", st == 200, f"{st}")

# ── 8 console API endpoints (per-page backing data) ──────────────────
endpoints = [
    ("/api/v1/overview", ["investigations", "agents", "cloud", "health", "memory", "graph"]),
    ("/api/v1/search/unified?q=ACME", ["entities", "documents", "findings", "alerts", "investigations"]),
    ("/api/v1/documents?limit=3", ["items", "total"]),
    ("/api/v1/entities?limit=3", ["items"]),
    ("/api/v1/agents/activity", ["agents"]),
    ("/api/v1/audit?limit=3", ["items"]),
    ("/api/v1/tasks?limit=3", ["items"]),
]
for path, keys in endpoints:
    st, body, _ = get(path, KEY)
    try:
        j = json.loads(body)
        ok = st == 200 and all(k in j for k in keys)
    except Exception:
        ok = False
    check(f"endpoint {path}", ok, f"{st}")
st, body, _ = get("/api/v1/investigations?limit=1", KEY)
inv = json.loads(body).get("items", [])
if inv:
    iid = inv[0]["investigation_id"]
    st, body, _ = get(f"/api/v1/investigations/{iid}/workspace", KEY)
    j = json.loads(body)
    check("endpoint /investigations/{id}/workspace",
          st == 200 and all(k in j for k in ["investigation", "findings", "documents", "entities", "tasks", "alerts", "audit"]), f"{st}")
st, body, _ = get(f"/api/v1/documents/{json.loads(get('/api/v1/documents?limit=1', KEY)[1])['items'][0]['document_id']}", KEY)
j = json.loads(body)
check("document detail + reverse refs",
      st == 200 and all(k in j for k in ["document", "referenced_by_findings", "referenced_by_claims"]), f"{st}")

# ── SSE live stream ──────────────────────────────────────────────────
import threading, http.client
got_event = []
def listen():
    try:
        conn = http.client.HTTPConnection("10.10.10.41", 8800, timeout=20)
        conn.request("GET", "/api/v1/events", headers={"Authorization": f"Bearer {KEY}"})
        resp = conn.getresponse()
        start = time.time()
        buf = b""
        while time.time() - start < 15:
            chunk = resp.read1(4096) if hasattr(resp, "read1") else resp.read(4096)
            if not chunk: break
            buf += chunk
            if b'"event_type"' in buf and b"keepalive" not in buf.split(b'"event_type"')[0][-20:]:
                got_event.append(buf)
                buf = b""
                # monitor sweeps now fire bus events constantly — the FIRST
                # frame is often a sweep event, not ours; keep listening until
                # the manufactured TASK_CREATED arrives or the window closes
                if b"TASK_CREATED" in got_event[-1]:
                    break
    except Exception:
        pass
t = threading.Thread(target=listen, daemon=True)
t.start()
time.sleep(1)
# manufacture an event
data = json.dumps({"title": "SP3 SSE acceptance probe"}).encode()
r = urllib.request.Request(HUB + "/api/v1/investigations", data=data, method="POST")
r.add_header("Authorization", f"Bearer {KEY}")
r.add_header("Content-Type", "application/json")
urllib.request.urlopen(r, timeout=30).read()
t.join(timeout=18)
check("SSE live event received", any(b"TASK_CREATED" in f for f in got_event), f"frames={len(got_event)}")

# ── L3 policy via console path ───────────────────────────────────────
data = json.dumps({"action": "backup"}).encode()
r = urllib.request.Request(HUB + "/api/v1/components/redis/actions", data=data, method="POST")
r.add_header("Authorization", f"Bearer {KEY}")
r.add_header("Content-Type", "application/json")
try:
    urllib.request.urlopen(r, timeout=30)
    st = 200
except urllib.error.HTTPError as e:
    st = e.code
check("L3 action without admin token → 403 policy_denied", st == 403, f"{st}")

print(f"\n== {PASS} passed, {FAIL} failed ==")
sys.exit(1 if FAIL else 0)
