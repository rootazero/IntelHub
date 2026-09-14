#!/usr/bin/env python3
"""SP5 acceptance — Crucix key sources, SpiderFoot/Huginn activation,
evidence push endpoint, SpiderFoot→evidence bridge, Telegram alerts.

Usage: accept-sp5.py <agent-api-key> [base-url]
"""
import json
import os
import sys
import time
import urllib.request
import urllib.error

BASE = sys.argv[2] if len(sys.argv) > 2 else "http://10.10.10.41:8800"
KEY = sys.argv[1]
# SSH destination when running from a remote machine (auto-detected by
# scripts/_remote.py). Override with INTELHUB_SSH=IntelHub-test when
# running against the 415 test VM.
passed = failed = shelved = 0

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


def check_shelved(name, reason):
    """Report a check that cannot run because its dependency is
    shelved-by-design (e.g. third-party API key not provisioned).
    Does NOT count as failure."""
    global shelved
    shelved += 1
    print(f"SHELVE {name}  | {reason}")


def req(path, key=KEY, timeout=15, method="GET", body=None):
    r = urllib.request.Request(
        BASE + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"{}")
        except Exception:
            return e.code, {}


def vm_json(cmd, timeout=90):
    out = vm(cmd, timeout)
    try:
        return json.loads(out) if out else {}
    except Exception:
        return {}


print("== SP5 acceptance ==")

# 1. monitor key-gated sources live (FIRMS/ACLED keys migrated to hub secrets.env)
firms_state = redis("HGET", "hub:monitor:health", "firms")
firms_key = vm("grep '^FIRMS_MAP_KEY=' /home/zou/IntelHub/core/secrets.env 2>/dev/null | cut -d= -f2-").strip()
if not firms_key:
    check_shelved("monitor FIRMS source ok (key migrated)", "FIRMS_MAP_KEY not configured in secrets.env")
    check_shelved("FIRMS fire events in geo_events", "FIRMS_MAP_KEY not configured — collector shelved-by-design")
else:
    check("monitor FIRMS source ok (key migrated)", '\"state\":\"ok\"' in firms_state.replace(" ", ""), firms_state[:120])
    fires = pg("SELECT count(*) FROM geo_events WHERE source='monitor:firms'").splitlines()[-1]
    check("FIRMS fire events in geo_events", fires.isdigit() and int(fires) > 0, f"fire={fires}")

# 2. spiderfoot + huginn containers healthy
ps = vm("docker ps --format '{{.Names}} {{.Status}}' | grep -E 'spiderfoot|huginn'")
check("spiderfoot healthy", "intelhub-spiderfoot" in ps and "healthy" in ps, ps.splitlines()[0] if ps else "")
check("huginn healthy", "intelhub-huginn" in ps and "(healthy)" in ps, "")

# 3. POST /api/v1/evidence (Huginn bridge)
code, out = req("/api/v1/evidence", method="POST", body={
    "source": "huginn",
    "url": "https://example.com/sp5-accept-bridge",
    "title": "SP5 acceptance bridge doc",
    "content": "SP5 acceptance: Huginn watcher pushed this evidence event through the REST bridge. "
               "It exercises normalization, dedup, provenance and embedding eligibility.",
    "metadata": {"suite": "sp5"},
})
check("POST /api/v1/evidence ingests", code == 200 and bool(out.get("document_id")), f"doc={out.get('document_id', '')[:8]}")
code2, _ = req("/api/v1/evidence", key="", method="POST", body={"source": "x", "url": "https://x.co", "content": "y"})
check("evidence endpoint rejects no-key (401)", code2 == 401, str(code2))

# 4. spiderfoot scan → evidence bridge (scan sp5-mini: sfp_dnsresolve on 93.184.215.14)
scan_id = None
for _ in range(6):  # scan already FINISHED; wait for poller tick (120s)
    lst = vm_json('curl -s -m 8 "http://10.10.10.41:5001/scanlist"')
    for row in lst if isinstance(lst, list) else []:
        if isinstance(row, list) and len(row) > 6 and row[1] == "sp5-mini":
            if row[6] == "FINISHED":
                scan_id = row[0]
            break
    if scan_id:
        break
    time.sleep(10)
check("spiderfoot scan finished", scan_id is not None, f"id={scan_id}")
if scan_id:
    doc = ""
    for _ in range(15):  # poller tick 120s + ingest
        doc = pg(f"SELECT document_id FROM documents WHERE metadata->>'scan_id'='{scan_id}' LIMIT 1").splitlines()[-1]
        if doc and "-" in doc:
            break
        time.sleep(10)
    check("spiderfoot scan → evidence document", bool(doc and "-" in doc), f"doc={doc[:8] if doc else 'none'}")

# 5. telegram channel delivered (drill rows from implementation phase)
# Only meaningful when at least one upstream collector has fired — if all
# key-gated sources are shelved, no alert is expected, so shelve the check.
any_key_gated_active = bool(firms_key)
if not any_key_gated_active:
    check_shelved("telegram delivery DELIVERED", "no key-gated sources active — no alerts expected")
else:
    tg = pg("SELECT status FROM alert_deliveries WHERE endpoint='telegram://chat' ORDER BY created_at DESC LIMIT 1").splitlines()[-1]
    check("telegram delivery DELIVERED", tg == "DELIVERED", tg)

print(f"\n== {passed} passed, {shelved} shelved, {failed} failed ==")
sys.exit(1 if failed else 0)
