#!/usr/bin/env python3
"""IntelHub SP2B acceptance driver — runs from Mac against http://10.10.10.41:8800.

Covers spec 2026-09-09-intelhub-hub-core-2b-design.md acceptance criteria:
  1. Level 3 policy deny/allow (admin token) + security alert + audit
  2. Graph write trio (entity/claim/relationship) → PG + Neo4j; neo4j-down queue replay
  4. Embedding activation: crawl → DONE → semantic mode:vector; §42 cache proof
  5. Alert closed loop: sensor stop → ALERT_RAISED + webhook DELIVERED + ack
  6. /api/v1/components listing
Budget drill (criterion 3) runs separately with tiny budgets (see report).

Usage: accept-sp2b.py <agent_key> <admin_token>
"""
import json, sys, time, urllib.request, urllib.error
import os

HUB = os.environ.get("INTELHUB_HUB", "http://10.10.10.41:8800")
KEY = sys.argv[1]
# SSH destination when running from a remote machine (auto-detected by
# scripts/_remote.py). Override with INTELHUB_SSH=IntelHub-test when
# running acceptance against the 415 test VM (default: production).
ADMIN = sys.argv[2]
PASS = FAIL = 0

# Shared ssh-or-local helpers (scripts/_remote.py). On a remote Mac
# these route via ssh; on the hub VM itself they run locally via docker
# exec + subprocess shell. No INTELHUB_LOCAL env needed.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _remote import pg as sql, sh  # noqa: E402

def check(name, ok, detail=""):
    global PASS, FAIL
    if ok: PASS += 1; print(f"PASS {name}  | {detail}")
    else: FAIL += 1; print(f"FAIL {name}  | {detail}")

def req(method, path, body=None, key=KEY, admin=None, raw=False):
    r = urllib.request.Request(HUB + path, method=method)
    r.add_header("Authorization", f"Bearer {key}")
    if admin: r.add_header("X-Admin-Token", admin)
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        r.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(r, data, timeout=120) as resp:
            payload = resp.read().decode()
            return resp.status, (payload if raw else json.loads(payload))
    except urllib.error.HTTPError as e:
        t = e.read().decode()
        try: return e.code, json.loads(t)
        except Exception: return e.code, {"error": t}

def mcp_initialize(key=KEY):
    body = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "accept-2b", "version": "0.1"}}}
    r = urllib.request.Request(HUB + "/mcp", data=json.dumps(body).encode(), method="POST")
    r.add_header("Authorization", f"Bearer {key}")
    r.add_header("Content-Type", "application/json")
    r.add_header("Accept", "application/json, text/event-stream")
    with urllib.request.urlopen(r, timeout=60) as resp:
        sid = resp.headers.get("mcp-session-id")
        resp.read()
    # initialized notification
    n = urllib.request.Request(HUB + "/mcp", data=json.dumps(
        {"jsonrpc": "2.0", "method": "notifications/initialized"}).encode(), method="POST")
    n.add_header("Authorization", f"Bearer {key}")
    n.add_header("Content-Type", "application/json")
    n.add_header("Accept", "application/json, text/event-stream")
    n.add_header("Mcp-Session-Id", sid)
    urllib.request.urlopen(n, timeout=60).read()
    return sid

def tool(sid, name, args, id_, admin=None):
    body = {"jsonrpc": "2.0", "id": id_, "method": "tools/call",
            "params": {"name": name, "arguments": args}}
    r = urllib.request.Request(HUB + "/mcp", data=json.dumps(body).encode(), method="POST")
    r.add_header("Authorization", f"Bearer {KEY}")
    r.add_header("Content-Type", "application/json")
    r.add_header("Accept", "application/json, text/event-stream")
    r.add_header("Mcp-Session-Id", sid)
    if admin: r.add_header("X-Admin-Token", admin)
    try:
        with urllib.request.urlopen(r, timeout=180) as resp:
            text = resp.read().decode()
    except urllib.error.HTTPError as e:
        return {"http_error": e.code, "body": e.read().decode()}
    for line in text.splitlines():
        if line.startswith("data: {") or (line.startswith("data:") and "{" in line):
            try: return json.loads(line[line.index("{"):])
            except Exception: pass
    return {"raw": text}

def tool_json(resp):
    content = resp.get("result", {}).get("content", [])
    if content and content[0].get("type") == "text":
        try: return json.loads(content[0]["text"])
        except Exception: return {"text": content[0]["text"]}
    return resp

def tool_error(resp):
    return resp.get("error", {}).get("message", "")

print("== SP2B acceptance ==")
sid = mcp_initialize()

# ── 1. Level 3 policy ────────────────────────────────────────────────
r = tool(sid, "run_component_action", {"component": "redis", "action": "backup"}, 10)
err = tool_error(r)
check("L3 without admin token → policy_denied", "policy_denied" in err, err[:80])
time.sleep(1)
cnt = sql("SELECT count(*) FROM audit_records WHERE action='policy_denied' AND object_id='run_component_action'")
check("L3 denial audited", cnt.isdigit() and int(cnt) >= 1, f"audit rows={cnt}")
acnt = sql("SELECT count(*) FROM alerts WHERE source='security' AND title LIKE '%Level 3 denied%'")
check("L3 denial raised security alert", acnt.isdigit() and int(acnt) >= 1, f"alerts={acnt}")

# ── 2. Graph write trio ──────────────────────────────────────────────
r = tool_json(tool(sid, "create_entity", {"kind": "org", "name": "ACME Intelligence Ltd",
                                          "attributes": {"country": "XX", "evil_key": "dropped"}}, 20))
e1 = r.get("entity_id")
check("create_entity org", bool(e1) and r.get("graph_synced") is True, f"id={str(e1)[:8]} synced={r.get('graph_synced')}")
r = tool_json(tool(sid, "create_entity", {"kind": "domain", "name": "acme-intel.example.com"}, 21))
e2 = r.get("entity_id")
check("create_entity domain", bool(e2), f"id={str(e2)[:8]}")
r = tool(sid, "create_entity", {"kind": "dragon", "name": "bad"}, 22)
check("invalid kind rejected", "invalid entity kind" in tool_error(r), tool_error(r)[:50])
r = tool_json(tool(sid, "create_relationship", {
    "from_kind": "org", "from_name": "ACME Intelligence Ltd",
    "to_kind": "domain", "to_name": "acme-intel.example.com", "rel_type": "owns"}, 23))
check("create_relationship", bool(r.get("relationship_id")) and r.get("graph_synced") is True,
      f"id={str(r.get('relationship_id'))[:8]}")
# claim needs evidence: reuse any stored document
doc = sql("SELECT document_id FROM documents ORDER BY created_at DESC LIMIT 1")
r = tool_json(tool(sid, "create_claim", {
    "text": "ACME Intelligence Ltd operates the domain acme-intel.example.com for testing.",
    "entities": [{"kind": "org", "name": "ACME Intelligence Ltd", "role": "subject"},
                 {"kind": "domain", "name": "acme-intel.example.com", "role": "object"}],
    "evidence_document_ids": [doc]}, 24))
check("create_claim with evidence", bool(r.get("claim_id")), f"id={str(r.get('claim_id'))[:8]}")
r = tool(sid, "create_claim", {"text": "no evidence claim here", "evidence_document_ids": []}, 25)
check("claim without evidence rejected", "evidence" in tool_error(r), tool_error(r)[:60])
neo = sql("SELECT count(*) FROM entities WHERE name='ACME Intelligence Ltd'")
check("entity in PG canonical", neo == "1", f"rows={neo}")
r = tool_json(tool(sid, "query_entity", {"name": "ACME Intelligence", "limit": 5}, 26))
check("entity queryable in Neo4j", r.get("count", 0) >= 1, f"count={r.get('count')}")
r = tool_json(tool(sid, "query_relationship", {"name": "ACME Intelligence Ltd", "limit": 10}, 27))
check("relationship queryable in Neo4j", r.get("count", 0) >= 1, f"count={r.get('count')}")

# neo4j down → queue; recovery → replay
sh("docker stop intelhub-neo4j")
time.sleep(2)
r = tool_json(tool(sid, "create_entity", {"kind": "person", "name": "Queue Replay Test Person"}, 28))
check("neo4j down → graph_synced=false", r.get("graph_synced") is False, f"synced={r.get('graph_synced')}")
q = sql("SELECT count(*) FROM graph_sync_queue WHERE status='PENDING'")
check("op queued (§72)", q.isdigit() and int(q) >= 1, f"pending={q}")
sh("docker start intelhub-neo4j")
deadline = time.time() + 90
replayed = False
while time.time() < deadline:
    time.sleep(8)
    done = sql("SELECT count(*) FROM graph_sync_queue WHERE status='DONE'")
    if done.isdigit() and int(done) >= 1:
        replayed = True; break
check("graph op auto-replayed after recovery", replayed, f"done={done}")

# ── 4. Embedding pipeline + real semantic search ─────────────────────
r = tool_json(tool(sid, "crawl_url", {"url": "https://www.rfc-editor.org/rfc/rfc2544.txt"}, 30))
doc2 = r.get("document_id")
check("crawl substantial article", bool(doc2), f"doc={str(doc2)[:8]} dup={r.get('duplicate')}")
deadline = time.time() + 120
emb = ""
while time.time() < deadline:
    time.sleep(6)
    emb = sql(f"SELECT embedding_status FROM documents WHERE document_id='{doc2}'")
    if emb in ("DONE", "FAILED", "SKIPPED"): break
check("document embedded (status DONE)", emb == "DONE", f"status={emb}")
chunks = sql(f"SELECT count(*) FROM embedding_chunks WHERE document_id='{doc2}'")
check("embedding_chunks recorded (§42 key)", chunks.isdigit() and int(chunks) >= 1, f"chunks={chunks}")
pts = sh("docker run --rm --network intelhub-data curlimages/curl:8.14.1 -fsS http://qdrant:6333/collections/evidence__text-embedding-3-small__1536 | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"result\"][\"points_count\"])'")
check("qdrant points upserted", pts.isdigit() and int(pts) >= 1, f"points={pts}")
tok = sql("SELECT count(*) FROM cost_records WHERE kind='embedding_tokens'")
check("embedding tokens metered (§59)", tok.isdigit() and int(tok) >= 1, f"records={tok}")
time.sleep(2)
r = tool_json(tool(sid, "semantic_search", {"query": "measuring network device throughput and latency benchmarking methodology", "limit": 5}, 31))
check("semantic_search mode=vector", r.get("mode") == "vector", f"mode={r.get('mode')} count={r.get('count')}")
check("semantic result matches topic", any(doc2 in str(it.get("document_id")) for it in r.get("items", [])),
      f"top doc match")
r = tool_json(tool(sid, "hybrid_search", {"query": "benchmarking network performance", "limit": 5}, 32))
check("hybrid_search RRF fusion", "hybrid" in str(r.get("mode", "")), f"mode={r.get('mode')}")
# §42 cache proof: re-enqueue the same document → all chunks reused, zero new tokens
tok_before = sql("SELECT COALESCE(SUM(amount),0) FROM cost_records WHERE kind='embedding_tokens'")
sql(f"INSERT INTO embedding_jobs (job_id, document_id, status, model, force) VALUES (gen_random_uuid(), '{doc2}', 'PENDING', 'text-embedding-3-small', true)")
time.sleep(15)
reason = sql(f"SELECT reason FROM embedding_jobs WHERE document_id='{doc2}' AND status='DONE' ORDER BY updated_at DESC LIMIT 1")
tok_after = sql("SELECT COALESCE(SUM(amount),0) FROM cost_records WHERE kind='embedding_tokens'")
check("§42 cache: re-embed reuses chunks, 0 new tokens",
      "reused" in (reason or "") and tok_before == tok_after, f"reason={reason} tokens {tok_before}→{tok_after}")

# ── 5. Alert closed loop (webhook) ───────────────────────────────────
# Dedupe semantics: an open alert with the same dedupe_key gets BUMPED (no new
# row, no new delivery) — correct anti-spam behavior. For a deterministic test,
# close lingering open flap alerts first so this run produces a FRESH alert +
# delivery row; match on updated_at to also catch bumps.
sql("UPDATE alerts SET status='ack', updated_at=now() WHERE dedupe_key LIKE 'flap:searxng%' AND status='open'")
# Webhook sink is a /tmp dev artifact (wiped by VM reboot) — recreate if gone.
import base64
_SINK_SRC = '''import http.server, datetime
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(n).decode("utf-8", "replace")
        open("/tmp/webhook-hits.log", "a").write(f"{datetime.datetime.utcnow().isoformat()} {self.path} {body}\\n")
        self.send_response(200); self.end_headers()
    def log_message(self, *a):
        pass
http.server.HTTPServer(("0.0.0.0", 18899), H).serve_forever()
'''
_b64 = base64.b64encode(_SINK_SRC.encode()).decode()
sh(f"test -f /tmp/webhook_sink.py || echo {_b64} | base64 -d > /tmp/webhook_sink.py")
sh("pgrep -f 'python3 /tmp/webhook_sink.py' >/dev/null || (setsid nohup python3 /tmp/webhook_sink.py >/dev/null 2>&1 < /dev/null &)")
cutoff = sql("SELECT now()")  # server-time cutoff: only alerts CREATED after cleanup qualify
sh("docker stop intelhub-searxng")
deadline = time.time() + 90
alert_id = ""
while time.time() < deadline:
    time.sleep(8)
    alert_id = sql(f"SELECT alert_id FROM alerts WHERE source='sensor' AND title LIKE '%searxng%DOWN%' AND created_at > '{cutoff}' ORDER BY created_at DESC LIMIT 1")
    if alert_id: break
check("sensor stop → critical alert raised", bool(alert_id), f"alert={alert_id[:8] if alert_id else 'none'}")
deadline = time.time() + 60
dstat = ""
tstat = ""
while time.time() < deadline:
    time.sleep(5)
    dstat = sql(f"SELECT string_agg(status, ',' ORDER BY status) FROM alert_deliveries WHERE alert_id='{alert_id}' AND endpoint != 'telegram://chat'" if alert_id else "SELECT ''")
    tstat = sql(f"SELECT string_agg(status, ',' ORDER BY status) FROM alert_deliveries WHERE alert_id='{alert_id}' AND endpoint = 'telegram://chat'" if alert_id else "SELECT ''")
    if dstat == "DELIVERED" and tstat == "DELIVERED": break
check("webhook DELIVERED", dstat == "DELIVERED", f"status={dstat}")
check("telegram DELIVERED (SP5 channel)", tstat == "DELIVERED", f"status={tstat}")
hits = sh("grep -c searxng /tmp/webhook-hits.log 2>/dev/null || echo 0")
check("webhook receiver got payload", hits.isdigit() and int(hits) >= 1, f"hits={hits}")
sh("docker start intelhub-searxng")
if alert_id:
    r = tool_json(tool(sid, "acknowledge_alert", {"alert_id": alert_id}, 40))
    check("acknowledge_alert works", r.get("status") == "ack", f"{r}")
r = tool_json(tool(sid, "list_alerts", {"limit": 10}, 41))
check("list_alerts returns items", r.get("count", 0) >= 1, f"count={r.get('count')}")

# ── 6. Components endpoint ───────────────────────────────────────────
code, body = req("GET", "/api/v1/components")
names = [i["name"] for i in body.get("items", [])] if code == 200 else []
check("components endpoint lists 8", code == 200 and len(names) == 8, f"{names}")
resolved = [i["name"] for i in body.get("items", []) if i.get("latest_known_version")]
check("registry latest versions resolved", len(resolved) >= 5, f"resolved={resolved}")

# ── 1b. Level 3 WITH admin token (backup — also the pre-upgrade restore point) ──
r = tool(sid, "run_component_action", {"component": "redis", "action": "backup"}, 50, admin=ADMIN)
j = tool_json(r)
check("L3 backup with admin token succeeds", j.get("ok") is True, str(j)[:100])

print(f"\n== {PASS} passed, {FAIL} failed ==")
sys.exit(1 if FAIL else 0)
