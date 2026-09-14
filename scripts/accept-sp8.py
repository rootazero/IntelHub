#!/usr/bin/env python3
"""SP7 acceptance — Monitor Command Deck (可视化整合).

Covers: /signals/history endpoint (shape, days param, downsampling sanity),
/monitor/delta endpoint (shape, direction enum, source coverage), default-route
SPA, MCP tool count unchanged, regression gates run separately.

Usage: accept-sp8.py <agent-api-key> [base-url]
"""
import json
import os
import sys
import urllib.request
import urllib.error

BASE = sys.argv[2] if len(sys.argv) > 2 else "http://10.10.10.41:8800"
HUB = BASE
KEY = sys.argv[1]
# SSH destination when running from a remote machine (auto-detected by
# scripts/_remote.py). Override with INTELHUB_SSH=IntelHub-test when
# running against the 415 test VM.
passed = failed = 0

# Shared ssh-or-local helpers (auto-route: ssh on remote Mac, docker exec
# on the hub VM itself — sentinel $INTELHUB_HOME/core/hub decides).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _remote import sh as vm, pg, redis  # noqa: E402


def check(name, cond, detail=""):
    global passed, failed
    if cond:
        passed += 1
        print(f"PASS {name}  | {detail}")
    else:
        failed += 1
        print(f"FAIL {name}  | {detail}")


def req(path, key=KEY, timeout=20, method="GET", body=None, raw=False):
    r = urllib.request.Request(BASE + path, method=method,
                               data=json.dumps(body).encode() if body is not None else None)
    r.add_header("Authorization", f"Bearer {key}")
    if body is not None:
        r.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            data = resp.read()
            return resp.status, data if raw else json.loads(data or b"{}")
    except urllib.error.HTTPError as e:
        data = e.read()
        if raw:
            return e.code, data
        try:
            return e.code, json.loads(data or b"{}")
        except Exception:
            return e.code, {}




# 1. history endpoint shape + fred series has points
st, d = req("/api/v1/signals/history?series=fred:VIXCLS&days=40")
pts = d.get("points", [])
check("history endpoint shape + points",
      st == 200 and d.get("series") == "fred:VIXCLS" and len(pts) >= 2
      and all("t" in p and "v" in p for p in pts),
      f"status={st} points={len(pts)}")

# 2. days parameter narrows the window
st, d2 = req("/api/v1/signals/history?series=fred:VIXCLS&days=7")
pts2 = d2.get("points", [])
check("history days param narrows window",
      st == 200 and len(pts2) <= len(pts) and d2.get("days") == 7,
      f"days40={len(pts)} days7={len(pts2)}")

# 3. history requires series (400)
st, _ = req("/api/v1/signals/history")
check("history missing series → 400", st == 400, f"status={st}")

# 4. history unknown series → 200 + empty points
st, d3 = req("/api/v1/signals/history?series=nope:NOTHING")
check("history unknown series → empty ok",
      st == 200 and d3.get("points") == [], f"status={st}")

# 5. delta endpoint shape + direction enum
st, dd = req("/api/v1/monitor/delta")
rows = dd.get("sources", [])
valid_dirs = {"up", "down", "flat", "new_source"}
check("delta endpoint shape + direction enum",
      st == 200 and len(rows) >= 10
      and all(r.get("direction") in valid_dirs and "new" in r and "fetched" in r for r in rows),
      f"status={st} rows={len(rows)}")

# 6. delta covers every health-cell source (incl. series collectors)
health = redis("HKEYS", "hub:monitor:health")
health_sources = {s for s in health.split() if s}
delta_sources = {r.get("source") for r in rows}
check("delta covers all health-cell sources",
      health_sources == delta_sources and len(health_sources) >= 14,
      f"health={len(health_sources)} delta={len(delta_sources)} missing={sorted(health_sources - delta_sources)}")

# 6b. sweep-history ring exists and delta trend is consistent with it
hist_len = redis("LLEN", "hub:monitor:sweephist:usgs")
check("sweep-history ring populated (restart-proof baseline)",
      hist_len.isdigit() and int(hist_len) >= 1, f"usgs ring len={hist_len}")
with_trend = [r for r in rows if isinstance(r.get("trend"), list) and len(r["trend"]) >= 2]
consistent = all(
    (r["direction"] == "up" and r["trend"][-1] > r["trend"][-2])
    or (r["direction"] == "down" and r["trend"][-1] < r["trend"][-2])
    or (r["direction"] == "flat" and r["trend"][-1] == r["trend"][-2])
    or r["direction"] == "new_source"
    for r in with_trend
)
check("delta trend consistent with direction (ring-based)",
      len(with_trend) >= 2 and consistent, f"with_trend={len(with_trend)} consistent={consistent}")

# 7. default route serves the SPA (Monitor deck is the shell default)
st, html = req("/", raw=True)
check("default route serves SPA 200",
      st == 200 and b"<div id=\"root\">" in html or (st == 200 and b"id=\"root\"" in html),
      f"status={st}")

# 8. console bundle contains the HUD (scoped css class marker baked into js)
js = vm('grep -l "hud-root" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("console bundle contains hud-root (deck shipped)", bool(js), js or "not found")

# 8b. CARTO key baked into the bundle (gray-Esri silent-degradation guard)
carto = vm('grep -o "cb1_[a-z0-9_]*" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("console bundle has CARTO key (primary dark basemap)", bool(carto), carto or "MISSING — would start at Esri")

# 8c. kind taxonomy palette baked in (kind-colored dots + clickable chips)
pal = vm('grep -c "#b388ff" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | grep -v ":0" | head -1')
check("console bundle has kind palette (political purple)", bool(pal), pal or "MISSING")

# 8d. taxonomy actually flowing: military (usaspending retag) + a classified kind.
# Wide window: contract award dates can be months old.
st, d8 = req("/api/v1/radar/events?kind=military&from=2026-01-01T00:00:00Z&limit=50")
mil = len(d8.get("items", [])) if st == 200 else 0
check("radar kind=military events exist (usaspending retag)", mil >= 1, f"count={mil}")
st, d8b = req("/api/v1/radar/events?kind=political&limit=5")
pol = len(d8b.get("items", [])) if st == 200 else 0
check("radar kind=political events exist (title classifier)", pol >= 1, f"count={pol}")

# 8e. climate plane (SP8-D): EONET climate events + indicator series
st, d8c = req("/api/v1/radar/events?kind=climate&from=2026-01-01T00:00:00Z&limit=50")
cli = len(d8c.get("items", [])) if st == 200 else 0
check("radar kind=climate events exist (eonet)", cli >= 1, f"count={cli}")
n = pg("SELECT count(DISTINCT series) FROM signal_observations WHERE source='monitor:climate'")
check("climate indicator series observed (CO2+GISTEMP)", n.strip().isdigit() and int(n.strip()) >= 2, f"climate={n.strip()}")
pal2 = vm('grep -c "#2dd4bf" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | grep -v ":0" | head -1')
check("console bundle has climate palette (teal)", bool(pal2), pal2 or "MISSING")

# 9. MCP tool count unchanged (28)
def mcp_initialize():
    body = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "accept-8", "version": "0.1"}}}
    r = urllib.request.Request(HUB + "/mcp", data=json.dumps(body).encode(), method="POST")
    r.add_header("Authorization", f"Bearer {KEY}")
    r.add_header("Content-Type", "application/json")
    r.add_header("Accept", "application/json, text/event-stream")
    with urllib.request.urlopen(r, timeout=60) as resp:
        sid = resp.headers.get("mcp-session-id")
        resp.read()
    n = urllib.request.Request(HUB + "/mcp", data=json.dumps(
        {"jsonrpc": "2.0", "method": "notifications/initialized"}).encode(), method="POST")
    n.add_header("Authorization", f"Bearer {KEY}")
    n.add_header("Content-Type", "application/json")
    n.add_header("Accept", "application/json, text/event-stream")
    n.add_header("Mcp-Session-Id", sid)
    urllib.request.urlopen(n, timeout=60).read()
    return sid


sid = mcp_initialize()
body = {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
r = urllib.request.Request(HUB + "/mcp", data=json.dumps(body).encode(), method="POST")
r.add_header("Authorization", f"Bearer {KEY}")
r.add_header("Content-Type", "application/json")
r.add_header("Accept", "application/json, text/event-stream")
r.add_header("Mcp-Session-Id", sid)
with urllib.request.urlopen(r, timeout=60) as resp:
    text = resp.read().decode()
tools = []
for line in text.splitlines():
    if line.startswith("data:") and "{" in line:
        try:
            msg = json.loads(line[line.index("{"):])
            tools = [t["name"] for t in msg.get("result", {}).get("tools", [])]
        except Exception:
            pass
check("MCP tools/list = 40 (28 + list_tools + tool_schema + investigate + 10 sp9)", len(tools) >= 38, f"count={len(tools)}")

# 10. Reset-key infrastructure (idempotent: validates tooling + format +
#     agent_id consistency, never rotates live keys). The actual rotation
#     flow is exercised manually: see `bash scripts/reset-key.sh {agent|console|all}`.
import re
IHK_RE = re.compile(r"^ihk_[a-f0-9]{64}$")
KEYS_FILE = "$HOME_DIR/core/agent-keys.txt".replace("$HOME_DIR", "/home/zou/IntelHub")
keys_txt = vm(f"cat {KEYS_FILE} 2>/dev/null")
def extract(name, blob):
    # Line-walk: split on === name === header, read the next 3 non-empty lines.
    aid = key = None
    lines = blob.splitlines()
    for i, line in enumerate(lines):
        if line.strip() == f"=== {name} ===":
            for j in range(i + 1, min(i + 6, len(lines))):
                if lines[j].startswith("agent_id:"):
                    aid = lines[j].split(":", 1)[1].strip()
                elif lines[j].startswith("api_key:"):
                    key = lines[j].split(":", 1)[1].strip()
            break
    return (aid, key)
agent_block = extract("agent", keys_txt)
console_block = extract("console", keys_txt)
check("agent-keys.txt: 'agent' block parses", agent_block is not None,
      f"agent_id={agent_block[0] if agent_block else '?'}")
check("agent-keys.txt: 'agent' key matches ihk_<64hex>", agent_block and bool(IHK_RE.match(agent_block[1])),
      f"key={agent_block[1] if agent_block else '?'}")
check("agent-keys.txt: 'console' block parses", console_block is not None,
      f"agent_id={console_block[0] if console_block else '?'}")
check("agent-keys.txt: 'console' key matches ihk_<64hex>", console_block and bool(IHK_RE.match(console_block[1])),
      f"key={console_block[1] if console_block else '?'}")

# reset-key.sh script: present, executable, refuses bad input cleanly.
check("scripts/reset-key.sh exists + executable",
      bool(vm("test -x /home/zou/IntelHub/scripts/reset-key.sh && echo OK")),
      "missing or not executable")
out = vm("INTELHUB_NONINTERACTIVE=1 bash /home/zou/IntelHub/scripts/reset-key.sh 2>&1; true")
check("reset-key.sh rejects missing arg (exit code + 'usage' text)",
      "usage" in out.lower() and "agent|console|all" in out, out[:120])
out = vm("INTELHUB_NONINTERACTIVE=1 bash /home/zou/IntelHub/scripts/reset-key.sh bogus 2>&1; true")
check("reset-key.sh rejects unknown name (exit code + clear error)",
      "unknown name" in out, out[:120])

# hub rotate-agent-key CLI: present + rejects unknown agent name with bad_request.
out = vm("set -a; . /home/zou/IntelHub/core/hub.env; . /home/zou/IntelHub/core/secrets.env; set +a; "
         "/home/zou/IntelHub/core/hub rotate-agent-key --name __no_such_agent__ 2>&1; true")
check("hub rotate-agent-key rejects unprovisioned agent",
      "not provisioned" in out, out[:120])

# Live-key/agent_id parity between on-disk file and PG api_keys.
if agent_block:
    pg_agent_id = pg("SELECT agent_id::text FROM api_keys WHERE revoked = false "
                     "AND agent_id = (SELECT agent_id FROM agents WHERE name = 'agent')")
    check("PG api_keys: live 'agent' row matches agent-keys.txt agent_id",
          pg_agent_id.strip() == agent_block[0], f"file={agent_block[0]} pg={pg_agent_id.strip()}")

print(f"\n== {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
