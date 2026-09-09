#!/usr/bin/env python3
"""SP5 acceptance — Crucix key sources, SpiderFoot/Huginn activation,
evidence push endpoint, SpiderFoot→evidence bridge, Telegram alerts.

Usage: accept-sp5.py <agent-api-key> [base-url]
"""
import json
import subprocess
import sys
import time
import urllib.request
import urllib.error

BASE = sys.argv[2] if len(sys.argv) > 2 else "http://10.10.10.41:8800"
KEY = sys.argv[1]
passed = failed = 0


def check(name, cond, detail=""):
    global passed, failed
    if cond:
        passed += 1
        print(f"PASS {name}  | {detail}")
    else:
        failed += 1
        print(f"FAIL {name}  | {detail}")


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


def vm(cmd, timeout=90):
    return subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "IntelHub", cmd],
        capture_output=True, text=True, timeout=timeout,
    ).stdout.strip()


def vm_json(cmd, timeout=90):
    out = vm(cmd, timeout)
    try:
        return json.loads(out) if out else {}
    except Exception:
        return {}


PSQL = 'DBURL=$(grep "^DATABASE_URL=" /home/zou/IntelHub/core/hub.env | cut -d= -f2-); U=$(echo $DBURL | sed -E "s|.*://([^:]+):.*|\\1|"); docker exec intelhub-postgres psql -U "$U" -d intelhub -t -A -c'

print("== SP5 acceptance ==")

# 1. crucix key-gated sources live (FIRMS/EIA/ACLED)
ch = vm_json("curl -s -m 10 http://172.30.3.21:3117/api/health")
check("crucix sourcesOk >= 27 after keys", ch.get("sourcesOk", 0) >= 27, f"ok={ch.get('sourcesOk')} failed={ch.get('sourcesFailed')}")
fires = vm(f"""{PSQL} "SELECT count(*) FROM geo_events WHERE kind='fire'" """).splitlines()[-1]
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
        doc = vm(f"""{PSQL} "SELECT document_id FROM documents WHERE metadata->>'scan_id'='{scan_id}' LIMIT 1" """).splitlines()[-1]
        if doc and "-" in doc:
            break
        time.sleep(10)
    check("spiderfoot scan → evidence document", bool(doc and "-" in doc), f"doc={doc[:8] if doc else 'none'}")

# 5. telegram channel delivered (drill rows from implementation phase)
tg = vm(f"""{PSQL} "SELECT status FROM alert_deliveries WHERE endpoint='telegram://chat' ORDER BY created_at DESC LIMIT 1" """).splitlines()[-1]
check("telegram delivery DELIVERED", tg == "DELIVERED", tg)

print(f"\n== {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
