#!/usr/bin/env python3
"""SP6B acceptance — finance data plane (monitor phase B).

Covers: migration 0005, FRED/EIA/Treasury series, markets fallback chain,
finintel evidence, signal_query / financials_fetch / watchlist_manage tools,
Signals REST, alert rule integrity.

Usage: accept-sp7.py <agent-api-key> [base-url]
"""
import json
import subprocess
import os
import sys
import time
import urllib.request
import urllib.error

BASE = sys.argv[2] if len(sys.argv) > 2 else "http://10.10.10.41:8800"
HUB = BASE
KEY = sys.argv[1]
# SSH alias for VM-side checks. Override with INTELHUB_SSH=IntelHub-test
# when running acceptance against the 415 test VM (default: production).
SSH_HOST = os.environ.get("INTELHUB_SSH", "IntelHub")
passed = failed = 0


def check(name, cond, detail=""):
    global passed, failed
    if cond:
        passed += 1
        print(f"PASS {name}  | {detail}")
    else:
        failed += 1
        print(f"FAIL {name}  | {detail}")


def req(path, key=KEY, timeout=20, method="GET", body=None):
    r = urllib.request.Request(BASE + path, method=method,
                               data=json.dumps(body).encode() if body is not None else None)
    r.add_header("Authorization", f"Bearer {key}")
    if body is not None:
        r.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"{}")
        except Exception:
            return e.code, {}


def vm(cmd):
    return subprocess.run(
        ["ssh", "-o", "BatchMode=yes", SSH_HOST, cmd],
        capture_output=True, text=True, timeout=90,
    ).stdout.strip()


PSQL = 'DBURL=$(grep "^DATABASE_URL=" /home/zou/IntelHub/core/hub.env | cut -d= -f2-); U=$(echo $DBURL | sed -E "s|.*://([^:]+):.*|\\1|"); docker exec intelhub-postgres psql -U "$U" -d intelhub -t -A -c'


def pg1(sql):
    out = vm(f'{PSQL} "{sql}"')
    return out.splitlines()[-1].strip() if out else ""


def mcp_initialize():
    body = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "accept-7", "version": "0.1"}}}
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


def tool(sid, name, args, id_):
    body = {"jsonrpc": "2.0", "id": id_, "method": "tools/call",
            "params": {"name": name, "arguments": args}}
    r = urllib.request.Request(HUB + "/mcp", data=json.dumps(body).encode(), method="POST")
    r.add_header("Authorization", f"Bearer {KEY}")
    r.add_header("Content-Type", "application/json")
    r.add_header("Accept", "application/json, text/event-stream")
    r.add_header("Mcp-Session-Id", sid)
    try:
        with urllib.request.urlopen(r, timeout=240) as resp:
            text = resp.read().decode()
    except urllib.error.HTTPError as e:
        return {"http_error": e.code, "body": e.read().decode()}
    for line in text.splitlines():
        if line.startswith("data:") and "{" in line:
            try:
                return json.loads(line[line.index("{"):])
            except Exception:
                pass
    return {"raw": text}


def tool_json(resp):
    content = resp.get("result", {}).get("content", [])
    if content and content[0].get("type") == "text":
        try:
            return json.loads(content[0]["text"])
        except Exception:
            return {"text": content[0]["text"]}
    return resp


print("== SP6B acceptance: finance data plane ==")

# 1. migration 0005: tables exist, watchlist seeded
tabs = pg1("SELECT count(*) FROM information_schema.tables WHERE table_name IN ('signal_observations','monitor_watchlist','fd_cache')")
check("migration 0005 tables (3)", tabs == "3", tabs)
n = pg1("SELECT count(*) FROM monitor_watchlist")
check("watchlist seeded >= 15", n.isdigit() and int(n) >= 15, n)

# 2. finance collectors report health
REDIS = 'docker exec intelhub-redis redis-cli --no-auth-warning -a $(grep "^REDIS_PASSWORD=" /home/zou/IntelHub/compose/.env | cut -d= -f2)'
cells = vm(f"{REDIS} HKEYS hub:monitor:health").split()
fin_cells = [c for c in ["fred", "eia", "treasury", "markets", "finintel"] if c in cells]
check("finance collector health cells", len(fin_cells) >= 4, ",".join(fin_cells))

# 3. series flowing per source (allow sources w/o keys to be empty → check those with keys)
n = pg1("SELECT count(DISTINCT series) FROM signal_observations WHERE source='monitor:fred'")
check("FRED series observed >= 5", n.isdigit() and int(n) >= 5, f"fred={n}")
n = pg1("SELECT count(DISTINCT series) FROM signal_observations WHERE source='monitor:treasury'")
check("Treasury series observed >= 1", n.isdigit() and int(n) >= 1, f"treasury={n}")
n = pg1("SELECT count(*) FROM signal_observations WHERE series='gscpi:index'")
check("GSCPI series observed (SP8-A)", n.isdigit() and int(n) >= 1, f"gscpi={n}")
n = pg1("SELECT count(DISTINCT series) FROM signal_observations WHERE series LIKE 'comtrade:%'")
check("Comtrade series observed >= 3 (SP8-B)", n.isdigit() and int(n) >= 3, f"comtrade={n}")
for s, lo in [("fred:PAYEMS_K", 1), ("fred:ICSA", 1), ("fred:AWHE_YOY_PCT", 1)]:
    n = pg1(f"SELECT count(*) FROM signal_observations WHERE series='{s}'")
    check(f"BLS-via-FRED mirror {s}", n.isdigit() and int(n) >= lo, f"{s}={n}")
n = pg1("SELECT count(DISTINCT series) FROM signal_observations WHERE series LIKE 'quote:%'")
w = pg1("SELECT count(*) FROM monitor_watchlist WHERE enabled")
ok = n.isdigit() and w.isdigit() and int(n) >= max(1, int(int(w) * 0.8))
check("markets cover >= 80% of enabled watchlist", ok, f"quotes={n}/{w}")
n = pg1("SELECT count(*) FROM signal_observations WHERE source='monitor:eia'")
check("EIA observations present", n.isdigit() and int(n) > 0, f"eia={n}")

# 4. quote payload carries audit source
src = pg1("SELECT payload->>'source' FROM signal_observations WHERE series LIKE 'quote:%' LIMIT 1")
check("quote payload has source audit tag", src in ("fmp", "finnhub"), src)

# 5. finintel evidence (documents→sources.origin; first sweep landed 186 during deploy)
n = pg1("SELECT count(*) FROM documents d JOIN sources s ON s.source_id=d.source_id WHERE s.origin IN ('finnhub','stocktwits')")
check("finintel evidence present", n.isdigit() and int(n) > 0, f"evidence={n}")

# 6. MCP: signal_query
sid = mcp_initialize()
r = tool_json(tool(sid, "signal_query", {"series": "fred:%", "limit": 10}, 2))
check("signal_query returns observations", r.get("count", 0) > 0, f"count={r.get('count')}")

# 7. MCP: watchlist_manage closed loop
r = tool_json(tool(sid, "watchlist_manage", {"action": "add", "symbol": "ACME7", "asset_class": "us_stock", "label": "accept test"}, 3))
check("watchlist add", r.get("ok") is True, json.dumps(r)[:80])
r = tool_json(tool(sid, "watchlist_manage", {"action": "toggle", "symbol": "ACME7"}, 4))
check("watchlist toggle off", r.get("ok") is True and r.get("enabled") is False, json.dumps(r)[:80])
r = tool_json(tool(sid, "watchlist_manage", {"action": "remove", "symbol": "ACME7"}, 5))
check("watchlist remove", r.get("ok") is True, json.dumps(r)[:80])
r = tool_json(tool(sid, "watchlist_manage", {"action": "list"}, 6))
syms = [i["symbol"] for i in r.get("watchlist", [])]
check("watchlist list clean after loop", "ACME7" not in syms, f"n={len(syms)}")

# 8. MCP: financials_fetch — graceful on empty-balance account; cache logic
#    verified via a synthetic complete row (real-credit fetch tested at deploy).
vm(f'{PSQL} "DELETE FROM fd_cache WHERE ticker = \'ZZTEST\'" > /dev/null')
r0 = tool_json(tool(sid, "financials_fetch", {"ticker": "ZZTEST"}, 7))
brief0 = r0.get("brief", {})
check("financials_fetch graceful (no crash, honest incomplete)",
      "brief" in r0 and brief0.get("complete") in (True, False),
      f"complete={brief0.get('complete')} failed={len(brief0.get('failed_endpoints', []))}")
# seed a synthetic complete core row → second call must be a zero-cost HIT
import base64
_seed_sql = ("INSERT INTO fd_cache (ticker, fetched_at, endpoints, payload) VALUES "
             "('ZZTEST', now(), '{\"company_facts\":{\"name\":\"Test Co\"},\"income_statements\":[]}'::jsonb, "
             "'{\"ticker\":\"ZZTEST\",\"complete\":true,\"company\":{\"name\":\"Test Co\"}}'::jsonb);")
_b64 = base64.b64encode(_seed_sql.encode()).decode()
vm(f'echo {_b64} | base64 -d | docker exec -i intelhub-postgres psql -U $(DBURL=$(grep "^DATABASE_URL=" /home/zou/IntelHub/core/hub.env | cut -d= -f2-); echo $DBURL | sed -E "s|.*://([^:]+):.*|\\1|") -d intelhub -q')
seeded = pg1("SELECT count(*) FROM fd_cache WHERE ticker='ZZTEST'")
if seeded != "1":
    check("financials_fetch cache seed inserted", False, f"seeded={seeded}")
else:
    r2 = tool_json(tool(sid, "financials_fetch", {"ticker": "ZZTEST"}, 8))
    check("financials_fetch cache HIT (zero upstream cost)",
          r2.get("cached") is True and r2.get("brief", {}).get("ticker") == "ZZTEST",
          f"cached={r2.get('cached')}")
vm(f'{PSQL} "DELETE FROM fd_cache WHERE ticker = \'ZZTEST\'" > /dev/null')

# 9. REST: signals latest + watchlist list (console backing)
code, latest = req("/api/v1/signals/latest")
check("REST /signals/latest", code == 200 and latest.get("count", 0) > 0, f"count={latest.get('count')}")
code, wl = req("/api/v1/signals/watchlist")
check("REST /signals/watchlist", code == 200 and wl.get("count", 0) >= 15, f"count={wl.get('count')}")

# 10. series naming invariant (units carried — atlas UUP lesson)
bad = pg1("SELECT count(*) FROM signal_observations WHERE series NOT LIKE '%:%'")
check("all series names namespaced", bad == "0", bad)

print(f"\n== {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
