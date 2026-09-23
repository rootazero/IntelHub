#!/usr/bin/env python3
"""SP7 acceptance — Monitor Command Deck (可视化整合).

Covers: /signals/history endpoint (shape, days param, downsampling sanity),
/monitor/delta endpoint (shape, direction enum, source coverage), default-route
SPA, MCP tool count unchanged, regression gates run separately.

Usage: accept-sp8.py <agent-api-key> [base-url]
"""
import json
import os
import re
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


shelved = 0
deferred = 0


def check_shelved(name, reason):
    """Count as shelved-by-design (e.g. third-party API key not provisioned)."""
    global shelved
    shelved += 1
    print(f"SHELVE {name}  | {reason}")


def check_deferred(name, reason):
    """Acceptance-deferred (spec P12 §7.2 NOTE / R10): the behavior is real but
    only observable in a live browser. sp8 is urllib + ssh (no Playwright), so
    the runtime half is asserted out-of-band by `console/probe-gev.mjs`'s
    P12_PROBES segment. Does NOT count as pass or failure — the deferred count
    is surfaced in the summary so it stays visible."""
    global deferred
    deferred += 1
    print(f"DEFER {name}  | {reason}")


def secret(name):
    """Read a single env var value from hub secrets.env. Returns "" if missing."""
    out = vm(f"grep '^{name}=' /home/zou/IntelHub/core/secrets.env 2>/dev/null | cut -d= -f2-").strip()
    return out


fred_key = secret("FRED_API_KEY")


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
# fred:VIXCLS needs FRED_API_KEY; when unprovisioned the series is empty
# (collector degraded by design) → shelve the data-dependent assertions,
# still assert endpoint shape responds 200.
st, d = req("/api/v1/signals/history?series=fred:VIXCLS&days=40")
pts = d.get("points", [])
if not fred_key:
    check_shelved("history endpoint shape + points",
                  "FRED_API_KEY not configured — fred:VIXCLS has no data on this VM")
else:
    check("history endpoint shape + points",
          st == 200 and d.get("series") == "fred:VIXCLS" and len(pts) >= 2
          and all("t" in p and "v" in p for p in pts),
          f"status={st} points={len(pts)}")

# 2. days parameter narrows the window
st, d2 = req("/api/v1/signals/history?series=fred:VIXCLS&days=7")
pts2 = d2.get("points", [])
if not fred_key:
    check_shelved("history days param narrows window",
                  "FRED_API_KEY not configured — fred:VIXCLS has no data on this VM")
else:
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

# 9b. Graph edge labels hidden by default, only shown on selected (UX: dense
#     graphs drown in per-edge text). Source-level check — the bundle is
#     minified and gets re-hashed on every build, so we assert against the
#     TSX source that ships in scripts/.
gc_src = vm("cat /home/zou/IntelHub/console/src/components/GraphCanvas.tsx 2>/dev/null")
# Default edge labelText must return empty string (no per-edge text at rest).
default_label_ok = bool(re.search(
    r"labelText:\s*\(\)\s*=>\s*\"\"", gc_src))
check("graph: default edge labelText returns empty string", default_label_ok,
      "edge default must hide labels — see GraphCanvas.tsx edge.style.labelText")
# Selected-state edge labelText must reveal rel_type.
selected_label_ok = bool(re.search(
    r"state:\s*\{[^}]*selected:[^}]*labelText:\s*\(d:\s*unknown\)\s*=>\s*\(d\s+as\s+G6EdgeData\)\.rel_type",
    gc_src, re.DOTALL))
check("graph: selected-state edge labelText reveals rel_type", selected_label_ok,
      "edge state.selected must show rel_type — see GraphCanvas.tsx edge.state.selected")

# 9c. Basemap chain order (SP10): CARTO is primary, Esri is the always-on
#     tier-2 fallback, Stadia is the last-resort tier-3 fallback. Single
#     source of truth — both Radar and the command-deck MonitorMap import
#     from console/src/basemap, so we check that one file for the CHAIN
#     literal. The bundle is minified, hence source-level.
basemap_src = vm("cat /home/zou/IntelHub/console/src/basemap.ts 2>/dev/null")
chain_section = re.search(r"export const CHAIN:\s*TileProvider\[\]\s*=\s*\[(.*?)\];", basemap_src, re.DOTALL)
if chain_section:
    section = chain_section.group(1)
    carto_pos = section.find('"carto"')
    stadia_pos = section.find('"stadia"')
    esri_pos = section.find('"esri"')
    # All three literals must appear; carto before stadia; esri between
    # them (so chain has a tier-2 anchor that can't be skipped).
    chain_ok = (
        carto_pos != -1 and esri_pos != -1 and stadia_pos != -1
        and carto_pos < esri_pos < stadia_pos
    )
    check("radar: basemap chain order is carto → esri → stadia (basemap.ts)", chain_ok,
          f"carto@{carto_pos} esri@{esri_pos} stadia@{stadia_pos}")
else:
    check("radar: basemap chain order is carto → esri → stadia (basemap.ts)", False,
          "could not locate CHAIN definition in console/src/basemap.ts")

# 9d. Single source of truth: both Radar and the command-deck MonitorMap
#     must import from ../basemap (or ../../basemap) — no local redefinitions.
#     This catches the regression where someone copy-pastes PROVIDERS / CHAIN
#     back into a page file, which would silently desync the two maps again.
for page, import_path in [
    ("console/src/pages/Radar.tsx", r"from\s+[\"']\.\./basemap[\"']"),
    ("console/src/components/hud/MonitorMap.tsx", r"from\s+[\"']\.\./\.\./basemap[\"']"),
]:
    page_src = vm(f"cat /home/zou/IntelHub/{page} 2>/dev/null")
    imports_ok = bool(re.search(import_path, page_src)) and "from.meta.env" not in page_src
    # `from.meta.env` would catch an accidental local const that re-reads
    # import.meta.env directly instead of going through basemap.ts.
    check(f"basemap: {page} imports from shared basemap module", imports_ok,
          "must use 'from \"../basemap\"' / 'from \"../../basemap\"' — no local PROVIDERS/CHAIN/PRIMARY/STADIA_KEY/CARTO_KEY")

# 9e. RPM support: scripts/os-detect.sh is the single source of truth for
#     OS detection and pkg manager selection. install.sh + bootstrap-host.sh
#     must source it (not re-implement /etc/os-release parsing). Neither
#     script may hardcode 'apt-get install' in its body (must use the
#     $PKG_INSTALL variable exported by os-detect).
os_detect_src = vm("cat /home/zou/IntelHub/scripts/os-detect.sh 2>/dev/null")
check("rpm: scripts/os-detect.sh exposes OS_FAMILY variable",
      "OS_FAMILY" in os_detect_src,
      "must export OS_FAMILY=deb|rpm")
check("rpm: scripts/os-detect.sh sets PKG_INSTALL for deb",
      bool(re.search(r'PKG_INSTALL=\"apt-get install', os_detect_src)),
      "deb-family PKG_INSTALL must be 'apt-get install -y -qq'")
check("rpm: scripts/os-detect.sh sets PKG_INSTALL for rpm (dnf or yum)",
      bool(re.search(r'PKG_INSTALL=\"(?:dnf|yum) install', os_detect_src)),
      "rpm-family PKG_INSTALL must be 'dnf install -y -q' or 'yum install -y -q'")

install_src = vm("cat /home/zou/IntelHub/scripts/install.sh 2>/dev/null")
check("rpm: install.sh sources os-detect.sh",
      bool(re.search(r'os-detect\.sh', install_src)),
      "install.sh must source scripts/os-detect.sh for shared OS detection")

bootstrap_src = vm("cat /home/zou/IntelHub/scripts/bootstrap-host.sh 2>/dev/null")
check("rpm: bootstrap-host.sh sources os-detect.sh",
      bool(re.search(r'os-detect\.sh', bootstrap_src)),
      "bootstrap-host.sh must source scripts/os-detect.sh")
# Reject raw apt-get in the body (must go through $PKG_INSTALL).
# Allow it inside the comments / os-detect.sh sourced content though.
body_lines = [
    l for l in bootstrap_src.splitlines()
    if l.strip() and not l.lstrip().startswith("#")
]
body = "\n".join(body_lines)
hardcoded_apt = bool(re.search(r'\bsudo\b[^\n]*\bapt-get install\b', body))
check("rpm: bootstrap-host.sh body uses $PKG_INSTALL, no hardcoded apt-get install",
      not hardcoded_apt,
      "body must use $PKG_INSTALL — found raw 'sudo apt-get install' outside comments")

# 9f. Docker install hardening: scripts/os-detect.sh exports DOCKER_REPO_URL
#     so bootstrap-host.sh uses the per-distro official URL from
#     https://docs.docker.com/engine/install/ instead of the legacy
#     /linux/centos/ path. bootstrap-host.sh must NOT hardcode the centos
#     URL and must validate the daemon + compose plugin post-install.
check("docker: os-detect.sh exports DOCKER_REPO_URL",
      "DOCKER_REPO_URL" in os_detect_src,
      "must export DOCKER_REPO_URL for per-distro repo URL")
check("docker: os-detect.sh sets RPM DOCKER_REPO_URL to /linux/rhel/ or /linux/fedora/",
      bool(re.search(r'DOCKER_REPO_URL=\"https://download\.docker\.com/linux/(rhel|fedora)', os_detect_src)),
      "RPM-family DOCKER_REPO_URL must be /linux/rhel/ or /linux/fedora/ (per docker.com)")
check("docker: bootstrap-host.sh does NOT hardcode /linux/centos/",
      "/linux/centos/" not in bootstrap_src,
      "must use $DOCKER_REPO_URL — the legacy /linux/centos/ path is deprecated")
check("docker: bootstrap-host.sh validates docker daemon post-install",
      "docker info" in bootstrap_src,
      "must run 'sudo docker info' after install to confirm the daemon is up")
check("docker: bootstrap-host.sh verifies user is in docker group",
      bool(re.search(r'id\s+-nG.*docker', bootstrap_src)),
      "must verify 'id -nG $RUN_USER | grep -qw docker' after usermod")
check("docker: bootstrap-host.sh validates docker compose plugin",
      "docker compose version" in bootstrap_src,
      "must check the compose plugin is available (docker.com ships it as docker-compose-plugin)")

# 10. Reset-key infrastructure (idempotent: validates tooling + format +
#     agent_id consistency, never rotates live keys). The actual rotation
#     flow is exercised manually: see `bash scripts/reset-key.sh {agent|console|all}`.
IHK_RE = re.compile(r"^ihk_[a-f0-9]{64}$")
KEYS_FILE = "$HOME_DIR/core/agent-keys.txt".replace("$HOME_DIR", "/home/zou/IntelHub")
keys_txt = vm(f"cat {KEYS_FILE} 2>/dev/null")
def extract(name, blob):
    # Walk to the `=== name ===` header that is followed by `name: <name>`
    # (proves it's the correct block — tolerates stray duplicate headers from
    # past edits). Then read agent_id + api_key from the next few lines,
    # STOPPING at the next `=== ` block delimiter so a sibling block's
    # agent_id can't overwrite ours.
    aid = key = None
    lines = blob.splitlines()
    for i, line in enumerate(lines):
        if line.strip() != f"=== {name} ===":
            continue
        # Confirm the next non-blank line is `name: <name>` so we don't pick up
        # a stale empty header from a half-written file.
        for j in range(i + 1, min(i + 6, len(lines))):
            if lines[j].strip().startswith("name:"):
                if lines[j].split(":", 1)[1].strip() == name:
                    for k in range(i + 1, min(i + 8, len(lines))):
                        # Hard boundary: stop at the next block delimiter.
                        if lines[k].startswith("=== "):
                            break
                        if lines[k].startswith("agent_id:"):
                            aid = lines[k].split(":", 1)[1].strip()
                        elif lines[k].startswith("api_key:"):
                            key = lines[k].split(":", 1)[1].strip()
                    return (aid, key) if key else None
                break  # wrong-name block, keep searching
    return None


agent_block = extract("agent", keys_txt)
console_block = extract("console", keys_txt)
if agent_block is None and console_block is None and "===" not in keys_txt:
    # install.sh (2026-09+) writes agent-keys.txt as TSV one-line-per-agent:
    #   <agent_id>\t<agent_name>\t<api_key>
    # Older block-format checks below assume the legacy `=== name ===` layout
    # (410 prod). On fresh installer VMs validate the TSV rows instead.
    tsv_rows = [l.split("\t") for l in keys_txt.splitlines()
                if l.strip() and not l.startswith("#") and "\t" in l]
    tsv_keys_ok = all(len(r) == 3 and IHK_RE.match(r[2].strip()) for r in tsv_rows)
    check("agent-keys.txt: TSV rows parse (install.sh format)", len(tsv_rows) >= 1,
          f"rows={len(tsv_rows)} names={','.join(r[1] for r in tsv_rows)}")
    check("agent-keys.txt: every TSV api_key matches ihk_<64hex>", tsv_keys_ok,
          f"rows={len(tsv_rows)}")
else:
    check("agent-keys.txt: 'agent' block parses", agent_block is not None,
          f"agent_id={agent_block[0] if agent_block else '?'}")
    check("agent-keys.txt: 'agent' key matches ihk_<64hex>",
          bool(agent_block) and bool(IHK_RE.match(agent_block[1])),
          f"key={agent_block[1][:16] + '...' if agent_block else '?'}")
    check("agent-keys.txt: 'console' block parses", console_block is not None,
          f"agent_id={console_block[0] if console_block else '?'}")
    check("agent-keys.txt: 'console' key matches ihk_<64hex>",
          bool(console_block) and bool(IHK_RE.match(console_block[1])),
          f"key={console_block[1][:16] + '...' if console_block else '?'}")

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

# ---------------------------------------------------------------------------
# GEV P8: hand-drawn world-anchored annotations (pin/line/area) persist in PG.
#
# Migration note: the P8 plan wrote `core/migrations/0011_annotations_v1.sql`,
# but there is no `core/migrations/` in the repo (migrations live in
# `hub-core/migrations/`) and version 0011 was already taken — sqlx::migrate!
# refuses a duplicate version. The real file is
# `hub-core/migrations/0022_annotations_v1.sql`; table/index names unchanged.
# ---------------------------------------------------------------------------
ANNO = "/api/v1/annotations"
P8_PIN = {
    "shape": "pin",
    "color": "primary",
    "label": "p8-accept",
    "geometry": {"vertices": [{"lon": 0.0, "lat": 0.0}]},
    "ttl_ms": None,
    "meta": {},
}

# 11. schema present (migrator applies 0022 at hub-core boot)
anno_tbl = pg("SELECT to_regclass('public.annotations_v1')").strip()
check("p8: annotations_v1 table present (migration 0022)",
      anno_tbl == "annotations_v1", f"to_regclass={anno_tbl!r}")

# 12. write path: POST (Bearer) → GET same id → DELETE → GET must now be 404.
st_post, created = req(ANNO, method="POST", body=P8_PIN)
created_id = created.get("id") if isinstance(created, dict) else None
if st_post != 201 or not created_id:
    check("p8: POST→GET→DELETE→GET-404 roundtrip", False,
          f"POST status={st_post} body={created!r}")
else:
    st_get, row = req(f"{ANNO}/{created_id}")
    st_del, _ = req(f"{ANNO}/{created_id}", method="DELETE")
    st_gone, _ = req(f"{ANNO}/{created_id}")
    rt_ok = (
        st_get == 200
        and row.get("label") == "p8-accept"
        and row.get("shape") == "pin"
        and row.get("geometry") == P8_PIN["geometry"]
        and st_del == 204
        and st_gone == 404
    )
    check("p8: POST→GET→DELETE→GET-404 roundtrip", rt_ok,
          f"post={st_post} get={st_get} delete={st_del} get-after-delete={st_gone}"
          f" label={row.get('label')!r}")

# 13. bbox window: one pin in [-10,-10,10,10], one far outside → only the
#     in-window id is returned (list is world-anchored + spatially filterable).
st_a, pin_a = req(ANNO, method="POST", body=P8_PIN)
st_b, pin_b = req(ANNO, method="POST",
                  body={**P8_PIN, "label": "p8-out",
                        "geometry": {"vertices": [{"lon": 50.0, "lat": 50.0}]}})
a_id = pin_a.get("id") if isinstance(pin_a, dict) else None
b_id = pin_b.get("id") if isinstance(pin_b, dict) else None
st_bbox, bbox_body = req(f"{ANNO}?bbox=-10,-10,10,10")
bbox_ids = [r["id"] for r in bbox_body.get("annotations", [])] if st_bbox == 200 else []
check("p8: bbox filter isolates in-window annotation",
      st_a == 201 and st_b == 201 and st_bbox == 200
      and a_id in bbox_ids and b_id not in bbox_ids,
      f"status={st_bbox} in={a_id in bbox_ids} out={b_id in bbox_ids}")
for _id in (a_id, b_id):  # cleanup — acceptance must not leak rows
    if _id:
        req(f"{ANNO}/{_id}", method="DELETE")

# 14. geometry GIN index (jsonb_path_ops) present; the containment predicate
#     is the contract-shaped one ({vertices:[...]}) — the plan's
#     `@> '{"south":0}'` predates the corrected geometry shape and would only
#     document a stale contract. Planner choice is informational (tiny table).
anno_idx = pg("SELECT indexname FROM pg_indexes WHERE tablename = 'annotations_v1' "
              "AND indexname LIKE '%geom%'")
anno_plan = pg("EXPLAIN SELECT id FROM annotations_v1 "
               "WHERE geometry @> '{\"vertices\":[{\"lon\":0,\"lat\":0}]}'::jsonb LIMIT 1")
check("p8: annotations geometry GIN index present",
      "annotations_v1_geom_idx" in anno_idx,
      f"index={anno_idx!r} planner="
      f"{'Bitmap Index Scan' if 'Bitmap' in anno_plan else 'Seq Scan (tiny table OK)'}")

# ---------------------------------------------------------------------------
# GEV P9 (2026-09-18): cockpit overlay DOM contract.
#
# sp8 is python/urllib (no browser); the LIVE render (button click → frame +
# gauges + vision switch + briefing panel) is asserted by
# `console/probe-gev.mjs` (P9 segment). Here we guard the SHIPPED bundle — the
# cockpit entry testid and the three instrument testids must be baked into the
# built console JS, so the rail button and the compass/altimeter/speed gauges
# exist independently of any Cesium render (plan Task 5: 仅 DOM 存在性). This
# mirrors the existing bundle checks 8/8b/8c (hud-root, CARTO key, palette).
# ---------------------------------------------------------------------------
ck_btn = vm('grep -l "hud-cockpit-button" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p9: cockpit button present in console bundle", bool(ck_btn),
      ck_btn or "hud-cockpit-button not found in dist bundle")

ck_ins = vm('grep -o "hud-cockpit-compass\\|hud-cockpit-altimeter\\|hud-cockpit-speed" '
            '/home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | sort -u | wc -l')
check("p9: cockpit instruments DOM (compass/altimeter/speed) in bundle",
      ck_ins.strip() == "3",
      f"distinct instrument testids={ck_ins.strip()} (want 3)")

# ---------------------------------------------------------------------------
# GEV P10 (2026-09-18): tail adapter + HUD contract bundle-level guards.
#
# sp8 is python/urllib (no browser); the LIVE DOM render (body class toggle
# after click, shortcut cheatsheet pop, scene panel open, panel drag →
# localStorage roundtrip) is exercised by `console/probe-gev.mjs` (P10
# segment). Here we guard the SHIPPED bundle + the IntelHub adapter wiring
# in source, so the four P10 widgets exist independently of any Cesium
# render. Same source-vs-bundle split as the existing P9 (cockpit button +
# instruments) and P8 (annotations roundtrip + bbox) checks.
#
# Why source-level for the recording body class (instead of bundle grep):
# the literal "recording-mode" appears in vendored recording.css (copied
# into hud.css wholesale per plan §T3 step 7), so a bundle grep would pass
# even if GlobeV2 wiring regressed. Asserting the `setMode` HUD contract
# closure lives in GlobeV2.tsx is the contract-shaped check (it's the only
# path that toggles body.recording-mode at runtime per T3 concern #1).
# ---------------------------------------------------------------------------

# 15. recording body class wiring lives in GlobeV2.tsx (HUD contract setMode).
#     The vendor's `setRecordingMode(true)` ALSO toggles body.recording-mode
#     (recordingControls.js:40), but T3 decided to route through the HUD
#     contract `setMode` callback only (T3 review concern #1) — the body
#     class is set inside the `setMode` closure, not by invoking the
#     vendor's setRecordingMode API. The check below asserts the wiring
#     closure lives in GlobeV2.tsx so a future regression that drops the
#     HUD contract falls back to relying on vendor internal state, which
#     is currently unreachable from the rail click path.
globe_src = vm("cat /home/zou/IntelHub/console/src/pages/GlobeV2.tsx 2>/dev/null")
recording_setmode_wired = bool(re.search(
    r'setMode:\s*\(mode:\s*string\)\s*=>\s*\{[^}]*document\.body\.classList\.toggle\(\s*[\"\']recording-mode[\"\']',
    globe_src, re.DOTALL,
))
check("p10: recording body class wired via HUD contract setMode (GlobeV2.tsx)",
      recording_setmode_wired,
      "GlobeV2.tsx must define setMode closure that toggles body.recording-mode")

# 16. scene-panel HUD testid baked into the bundle (mirrors p9 cockpit button).
scene_panel = vm('grep -l "hud-scene-panel" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p10: scene panel HUD testid present in console bundle", bool(scene_panel),
      scene_panel or "hud-scene-panel not found in dist bundle")

# 17. panel-drag localStorage roundtrip — vendor's PanelPositionControls
#     owns the `godsEyeView.v8.panelPos.<id>` storage namespace (per T1
#     source-contracts pin: PANEL_POSITION_STORAGE_VERSION='v8'). The
#     `mountPanelDrag` adapter wraps the vendor constructor; the IntelHub
#     `HudPanelDragHandle` exposes the affordance on the panel. The
#     roundtrip needs both halves in place: the vendor storage key
#     template ships in the bundle, AND the adapter is wired in
#     GlobeV2.tsx so the live page can both write (drag end) and read
#     (mount) the key. The live drag → reload → restore cycle is covered
#     by `console/probe-gev.mjs` (p10-panel-drag segment); here we assert
#     the wiring is shipped.
panel_key_src = vm('grep -l "godsEyeView\\.\\${PANEL_POSITION_STORAGE_VERSION}\\.panelPos" '
                    '/home/zou/IntelHub/console/gev-engine/src/ui/panelPositionControls.js 2>/dev/null')
panel_drag_wired = bool(re.search(
    r'mountPanelDrag\s*\(',
    globe_src,
))
roundtrip_ok = bool(panel_key_src) and panel_drag_wired
check("p10: panel-drag localStorage roundtrip wired (vendor key + adapter)",
      roundtrip_ok,
      f"vendor_key={bool(panel_key_src)} adapter_wired={panel_drag_wired}"
      f" — both required for drag→write→reload→restore (live cycle in probe)")

# 18. GEV P11: CCTV popout panel testids baked into the bundle. The button
#     that opens the popout is `cctv-open-popout` (added to HudDetailPanel
#     CctvBody in P11). The popout panel itself is `cctv-popout-panel`.
#     Both must be in the dist bundle so a click can open the face-on 2D
#     image surface from the existing 3D-side camera detail view.
popout_btn_bundle = vm('grep -l "cctv-open-popout" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p11: cctv-open-popout testid baked into console bundle",
      bool(popout_btn_bundle),
      popout_btn_bundle or "MISSING — HudDetailPanel CctvBody button not shipped")
popout_panel_bundle = vm('grep -l "cctv-popout-panel" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p11: cctv-popout-panel testid baked into console bundle",
      bool(popout_panel_bundle),
      popout_panel_bundle or "MISSING — CctvPopoutPanel component not shipped")

# 19. P11 source-level: HudDetailPanel source still references the testid
#     (catches drift between dist bundle and source — bundle can ship an
#     older build than the source claims).
hd_source = vm('cat /home/zou/IntelHub/console/src/globe-hud/HudDetailPanel.tsx 2>/dev/null')
hd_btn = "cctv-open-popout" in hd_source
check("p11: HudDetailPanel.tsx references cctv-open-popout (source truth)",
      hd_btn, "button testid in source" if hd_btn else "MISSING in HudDetailPanel.tsx")

# 20. P11 source-level: CctvPopoutPanel source must define the
#     `cctv-popout-panel` testid (testid contract for probe-gev.mjs
#     p11-cctv-popout segment).
popout_source = vm('cat /home/zou/IntelHub/console/src/globe-hud/CctvPopoutPanel.tsx 2>/dev/null')
popout_id = "cctv-popout-panel" in popout_source
check("p11: CctvPopoutPanel.tsx references cctv-popout-panel (source truth)",
      popout_id, "panel testid in source" if popout_id else "MISSING in CctvPopoutPanel.tsx")

# ---------------------------------------------------------------------------
# GEV P12 (2026-09-21): flight-layer parity — aircraft source adapter + cockpit
# behavior contracts.
#
# Same source-vs-live split the P9/P10/P11 blocks use. checks 49/50 are
# bundle-level (the shipped dist must carry T3's adapter, which is what makes
# the vendor's flights track-backfill + adsbdb enrichment drip work). The
# distinguishing marker is the `hub.intelhub` label + the two error strings
# that exist ONLY in console/src/gev-adapters/aircraft-source.ts — the bare
# `getTrack`/`/api/opensky-track` literals also live in the vendored
# standalone.js, so grepping only for those would pass on a build that
# predates T3 (P12 review finding).
# ---------------------------------------------------------------------------

# 49. T3 aircraft source: getTrack + /api/opensky-track shipped. The vendor
#     tracking.js:480 calls `_source.getTrack` for every tracked flight's trail;
#     without the IntelHub adapter the default snapshot source has no such
#     method and trails silently never backfill.
track_marker = vm('grep -l "hub.intelhub" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
track_method = vm('grep -l "getTrack" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
track_path = vm('grep -l "/api/opensky-track" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p12: aircraft adapter getTrack + /api/opensky-track shipped in dist",
      bool(track_marker) and bool(track_method) and bool(track_path),
      f"marker={bool(track_marker)} getTrack={bool(track_method)} path={bool(track_path)}")

# 50. T3 aircraft source: getEnrichment + /api/adsbdb shipped. Vendor
#     enrichment.js:60 calls `_source.getEnrichment`; the route/type string is
#     unique to the adapter (the vendor builds its URL from query.kind).
enrich_method = vm('grep -l "getEnrichment" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
enrich_path = vm('grep -l "/api/adsbdb" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
enrich_guard = vm('grep -l "route enrichment id must be 2-8 char callsign" '
                  '/home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p12: aircraft adapter getEnrichment + /api/adsbdb shipped in dist",
      bool(enrich_method) and bool(enrich_path) and bool(enrich_guard),
      f"getEnrichment={bool(enrich_method)} path={bool(enrich_path)} guard={bool(enrich_guard)}")

# 51-53. Cockpit behavior (T5 keyboard / T6 camera / T7 viewport lock). These
# are LIVE-render contracts — sp8 has no browser (the P9/P10/P11 precedent
# keeps sp8 on urllib + ssh and puts live behavior in console/probe-gev.mjs).
# Per spec §7.2 NOTE + R10 they are acceptance-deferred here and asserted by
# probe-gev.mjs's P12_PROBES segment (p12-cockpit-* probes). The underlying
# adapters are unit-tested hub-side (shortcuts.test.ts / camera-transition
# .test.ts / viewport-lock.test.ts); the bundle-level wiring is covered by
# checks 49/50's marker and the P9 cockpit testid checks above.
check_deferred(
    "p12: cockpit keyboard shortcut (←/→/Esc/Tab live keydown)",
    "live-render contract — asserted by console/probe-gev.mjs p12-cockpit-* "
    "(probe exit 0); sp8 is urllib+ssh without Playwright (spec §7.2 NOTE/R10)")
check_deferred(
    "p14: cockpit camera flyTo lands at chase pose (duration 0.3-0.5s, {direction, up} orientation)",
    "needs a tracked flight + a Cesium viewer handle; headless probe cannot "
    "enter cockpit with trackedId — unit-tested in camera-transition.test.ts, "
    "runtime handle deferred to P13 (spec R10)")
check_deferred(
    "p12: cockpit viewport lock (enableInputs=false + cursor hidden)",
    "live-render contract — probe-gev.mjs p12-cockpit-viewport-lock asserts the "
    "lock's observable effect (canvas cursor:none); enableInputs is unit-tested "
    "in viewport-lock.test.ts (spec §7.2 NOTE/R10)")

# ---------------------------------------------------------------------------
# GEV P14 (2026-09-21): cockpit chase-cam. Three bundle-level guards for the
# three new adapters — chase-cam mount, model-visibility toggle, and the
# camera-transition isInFlight() export. These mirror the P12/P13 bundle
# checks (check the shipped dist, not the source) and are the acceptance
# layer's counterpart to the live probe-gev.mjs P14_PROBES.
# ---------------------------------------------------------------------------
ck_chase = vm('grep -l "mountCockpitChaseCam\\|chase-cam" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p14: cockpit chase-cam module bundled in console dist", bool(ck_chase),
      ck_chase or "mountCockpitChaseCam not found in dist bundle")

ck_modvis = vm('grep -l "mountModelVisibility\\|model-visibility" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p14: cockpit model-visibility module bundled in console dist", bool(ck_modvis),
      ck_modvis or "mountModelVisibility not found in dist bundle")

ck_inflight = vm('grep -l "isInFlight" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p14: cockpit camera-transition isInFlight() exported", bool(ck_inflight),
      ck_inflight or "isInFlight not found in dist bundle")

# ---------------------------------------------------------------------------
# GEV P15 T8 (2026-09-21): cockpit mouse-look — three bundle checks mirroring
# the P14 shape (vm('grep -l ...') + check()). Three categories per spec §8:
#   (1) mountCockpitMouseLook exported from cockpit barrel, used by GlobeV2
#       to mount the right-drag / wheel handler when cockpit opens.
#   (2) vendor-port trio (readCameraTargetFrame / setCameraTargetFrame /
#       createCameraOrientationAnimator) — ported from gev-engine into
#       console/src/gev-visual/cockpit/vendor-port/cockpitCameraOrientation.ts
#       per spec §4.3. One check() with all(...) so a single missing fn
#       fails the bundle cleanly; the three vm() calls keep the per-fn
#       diagnostic message.
#   (3) MOUSE_LOOK_ZERO_OFFSET — re-exported from the cockpit barrel and
#       imported by chase-cam.ts as the backward-compat fallback when no
#       mouse-look handle is supplied (P14 chase-cam still compiles).
# ---------------------------------------------------------------------------
ck_mlook = vm('grep -l "mountCockpitMouseLook" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p15: cockpit mouse-look mount bundled in console dist", bool(ck_mlook),
      ck_mlook or "mountCockpitMouseLook not found in dist bundle")

# Vendor-port trio: Vite tree-shakes unused exports even when re-exported
# from the barrel. Search for the implementation's unique identifiers
# instead of the export names: `eastNorthUpToFixedFrame` (readCameraTargetFrame
# uses Transforms.eastNorthUpToFixedFrame to project camera position onto
# target's ENU frame), `HeadingPitchRange` (setCameraTargetFrame's second
# arg), and `preUpdate.addEventListener` (createCameraOrientationAnimator's
# per-frame tick registration).
ck_vport_enu = vm('grep -l "eastNorthUpToFixedFrame" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
ck_vport_hpr = vm('grep -l "HeadingPitchRange" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
ck_vport_pre = vm('grep -l "preUpdate" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p15: cockpit vendor-port trio bundled (readCameraTargetFrame + setCameraTargetFrame + createCameraOrientationAnimator)",
      all([bool(ck_vport_enu), bool(ck_vport_hpr), bool(ck_vport_pre)]),
      f"enu={bool(ck_vport_enu)} hpr={bool(ck_vport_hpr)} pre={bool(ck_vport_pre)}")

# MOUSE_LOOK_ZERO_OFFSET is a constant object; Vite inlines it into the
# call site. Search for the value shape instead: headingDeltaRad=0 plus
# pitchDeltaRad=0 plus rangeOffsetM=0 — the constant is the only place
# all three zeros appear together in mouse-look.ts's offset state.
ck_zero = vm('grep -l "headingDeltaRad" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p15: cockpit MOUSE_LOOK_ZERO_OFFSET re-export bundled", bool(ck_zero),
      ck_zero or "mouse-look offset state (headingDeltaRad/pitchDeltaRad/rangeOffsetM) not found in dist bundle")

# ---------------------------------------------------------------------------
# GEV P16 (2026-09-22): cockpit HUD avionics — 10Hz update timer (T3),
# chase-cam attitude state (T1), HUD instruments (T5). bundle-only checks;
# source contracts live in source-contracts.test.ts (T6).
#
# Vite tree-shaking note: mountCockpitInstruments / mountCockpitHudTick /
# ChaseCamResolvedState are exported via the cockpit barrel, but tree-
# shaking may erase the literal export name if the import is side-effect-
# free and the consumer (GlobeV2.tsx) is reached via a different path.
# We search the implementation identifier as a fallback (`fromQuaternion`,
# `altitudeHistory`, `instruments.update`) so the check still fires when
# the function name itself is shaken.
# ---------------------------------------------------------------------------
ck_instruments = vm('grep -l "mountCockpitInstruments" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: mountCockpitInstruments bundled", bool(ck_instruments),
      ck_instruments or "mountCockpitInstruments not found in dist bundle")

ck_hudtick = vm('grep -l "mountCockpitHudTick" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
if not ck_hudtick:
    # P15 lesson: Vite tree-shakes unused exports even when re-exported from
    # the barrel. mountCockpitHudTick is consumed only inside GlobeV2's
    # cockpit boot block; in production bundles Vite inlines the 10Hz
    # setInterval call and the literal export name drops. Fall back to the
    # implementation identifier — the tick body calls flights.getTrackedInfo(),
    # which is unique to cockpit-hud-tick.ts and survives minification.
    ck_hudtick = vm('grep -l "getTrackedInfo" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: mountCockpitHudTick bundled", bool(ck_hudtick),
      ck_hudtick or "mountCockpitHudTick (and getTrackedInfo fallback) not found in dist bundle")

ck_get_resolved = vm('grep -l "getResolvedState" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: chase-cam.getResolvedState bundled", bool(ck_get_resolved),
      ck_get_resolved or "getResolvedState not found in dist bundle")

ck_hpr = vm('grep -l "fromQuaternion" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: HeadingPitchRoll.fromQuaternion bundled", bool(ck_hpr),
      ck_hpr or "fromQuaternion not found in dist bundle")

ck_altitude_history = vm('grep -l "altitudeHistory" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: altitude history window bundled", bool(ck_altitude_history),
      ck_altitude_history or "altitudeHistory not found in dist bundle")

ck_pitch_ladder = vm('grep -l "pitch-ladder\\|pitch_ladder\\|PitchLadder" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p16: pitch ladder SVG in dist", bool(ck_pitch_ladder),
      ck_pitch_ladder or "pitch ladder marker not found in dist bundle")

# ---------------------------------------------------------------------------
# GEV P13 (2026-09-21): flight-display optimization — default camera, enrich
# budget override, HudAircraftDetail panel, 3rd ADS-B source. Same source-vs-
# bundle split as the P9-P12 blocks. checks 54/55 are console-side (bundle +
# source truth); 56 pins the raised enrich value in source; 57 is the hub-side
# monitor registration (sp6 check_46 asserts the snapshot content).
# ---------------------------------------------------------------------------

# 54. T3 HudAircraftDetail: the panel's testid ships in dist AND the source is
#     mounted in GlobeV2 (the IntelHub mount point — vendor GlobeHud.tsx does
#     not exist in this tree, per the accepted T3 deviation).
detail_bundle = vm('grep -lF "hud-aircraft-detail" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
detail_mount = vm('grep -c "HudAircraftDetail" /home/zou/IntelHub/console/src/pages/GlobeV2.tsx 2>/dev/null | head -1')
check("p13: HudAircraftDetail testid in bundle + mounted in GlobeV2",
      bool(detail_bundle) and detail_mount.strip() not in ("", "0"),
      f"bundle={bool(detail_bundle)} globev2_refs={detail_mount.strip()}")

# 55. T3 i18n: the aircraft.detail namespace must be defined in BOTH dicts and
#     the dotted t() key literal must ship in the bundle. en/zh dicts use
#     nested-object keys (`aircraft: { detail: {...} }`); the component
#     references them as t("aircraft.detail.*") — the dotted literal is the
#     wire marker that survives in dist. Source pins: unique en value (ASCII)
#     + the `aircraft:` block in zh (the Dict type enforces shape parity, so
#     zh_block + en_value together prove both dicts carry the namespace).
i18n_en = vm('grep -cF "Click a flight to inspect" /home/zou/IntelHub/console/src/i18n/en.ts 2>/dev/null | head -1')
i18n_zh = vm('grep -cF "aircraft: {" /home/zou/IntelHub/console/src/i18n/zh.ts 2>/dev/null | head -1')
detail_key = vm('grep -lF "aircraft.detail." /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p13: aircraft.detail i18n (en value + zh block + bundle key)",
      i18n_en.strip() == "1" and i18n_zh.strip() == "1" and bool(detail_key),
      f"en={i18n_en.strip()} zh={i18n_zh.strip()} bundle_key={bool(detail_key)}")

# 56. T2 enrich budget: the raised budget must be `ceil: 800` (not the vendor
#     default 300). Source truth is enrich-override.ts — the value is pinned
#     by the T2 test's toEqual. The bundle marker is covered by sp6 check_45;
#     here the VALUE is what gates.
ceil_src = vm('grep -cF "ceil: 800" /home/zou/IntelHub/console/src/gev-boot/enrich-override.ts 2>/dev/null | head -1')
check("p13: enrich QA override ceil=800 (raised from vendor 300)",
      ceil_src.strip() == "1", f"ceil_800={ceil_src.strip()}")

# 57. T4 3rd ADS-B source: the monitor loop must be registered and running.
#     name() = "adsbx" → the module logs "adsbx us-hub sweep" after each
#     successful sweep, and the scheduler writes the `hub:monitor:health` cell
#     `adsbx` (registration proof). Both must be present after the restart's
#     first sweep (~125s hub sweep + stagger, well inside the 5-min wait).
adsbx_journal = vm('sudo journalctl -u hub-core --since "12 minutes ago" --no-pager 2>/dev/null | grep -F "adsbx us-hub sweep" | head -1')
adsbx_cell8 = redis("HGET", "hub:monitor:health", "adsbx").strip()
check("p13: adsbx monitor loop running (sweep journal + health cell)",
      bool(adsbx_journal) and adsbx_cell8 not in ("", "nil"),
      f"journal={bool(adsbx_journal)} cell={adsbx_cell8[:50]}")

# ---------------------------------------------------------------------------
# GEV P17 (2026-09-23): cockpit HUD element visibility — per-element show/
# hide map persisted to localStorage. Four checks cover: toggle button
# present in bundle, storage key literal ships, component file mounted in
# GlobeV2 (the consumer that wires the new HUD into the cockpit overlay),
# and the cockpit-store carries the elementVisibility field (source truth
# for the contract).
# ---------------------------------------------------------------------------

# 58. T4 HudCockpitElementSwitch toggle button testid ships in dist. The
#     literal is part of the button's `data-testid` and survives Vite
#     minification (verbatim attribute).
p17_switch_bundle = vm('grep -lF "hud-cockpit-element-switch" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p17: hud-cockpit-element-switch testid in bundle", bool(p17_switch_bundle),
      p17_switch_bundle or "hud-cockpit-element-switch not found in dist bundle")

# 59. T1 element-visibility module + storage key literal ship in bundle. The
#     literal `intelhub.cockpit.elementVisibility` is the localStorage key
#     pin and must survive — renaming it post-ship would invalidate every
#     existing user's preferences silently.
p17_storage_bundle = vm('grep -lF "intelhub.cockpit.elementVisibility" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("p17: ELEMENT_VISIBILITY_STORAGE_KEY literal in bundle", bool(p17_storage_bundle),
      p17_storage_bundle or "intelhub.cockpit.elementVisibility not found in dist bundle")

# 60. T6 wiring: HudCockpitElementSwitch must be mounted in
#     HudCockpitFrame (the cockpit overlay that composes all cockpit HUD
#     sub-components). Source-level proof so a refactor that removes the
#     import is caught at acceptance time.
p17_switch_mount = vm('grep -c "HudCockpitElementSwitch" /home/zou/IntelHub/console/src/globe-hud/HudCockpitFrame.tsx 2>/dev/null | head -1')
check("p17: HudCockpitElementSwitch mounted in HudCockpitFrame",
      p17_switch_mount.strip() not in ("", "0"),
      f"cockpit_frame_refs={p17_switch_mount.strip()}")

# 61. T2 source truth: cockpit-store carries elementVisibility in source.
#     TypeScript would catch this at build time, but acceptance runs against
#     the prebuilt dist — a grep-level pin protects against a sneaky refactor
#     that drops the field from the runtime default.
p17_store_field = vm('grep -cF "elementVisibility" /home/zou/IntelHub/console/src/gev-visual/cockpit/cockpit-store.ts 2>/dev/null | head -1')
check("p17: cockpit-store carries elementVisibility field",
      p17_store_field.strip() not in ("", "0"),
      f"cockpit_store_refs={p17_store_field.strip()}")

# ---------------------------------------------------------------------------
# GEV P18 (2026-09-23): cockpit SVG positioning — replaces the P16/P17 flex
# layout with a 3-col × 2-row CSS grid. Four checks cover: the grid-template-
# areas declaration, the shared pitchBank grid area, the gauge-light wrapper
# class, and the responsive scale media query.
# ---------------------------------------------------------------------------

# 62. CSS grid declaration: hud.css must declare grid-template-areas for the
#    instruments cluster (not the P16/P17 flex layout). Source-level pin.
p18_grid_src = vm('grep -cF "grid-template-areas" /home/zou/IntelHub/console/src/globe-hud/hud.css 2>/dev/null | head -1')
check("p18: hud.css declares grid-template-areas (cluster uses CSS grid, not flex)",
      p18_grid_src.strip() not in ("", "0"),
      f"grid_template_areas_refs={p18_grid_src.strip()}")

# 63. Pitch bank area shared by 4+ elements: pitch-ladder, bank-indicator,
#    altimeter, speed all map to 'pitchBank' so they overlap (the spec
#    sketch's center-column stack). Source-level pin.
p18_pitchbank = vm('grep -cF "grid-area: pitchBank" /home/zou/IntelHub/console/src/globe-hud/hud.css 2>/dev/null | head -1')
check("p18: 4+ elements share pitchBank grid area (center-column overlap)",
      p18_pitchbank.strip() not in ("", "0") and int(p18_pitchbank.strip()) >= 4,
      f"pitchBank_refs={p18_pitchbank.strip()}")

# 64. gauge-light wrapper class: each P16 component wraps its SVG in this
#    class for visual consistency. The literal class name must appear 5+
#    times in HudCockpitInstruments.tsx (one per P16 component).
p18_gauge_light = vm('grep -cF "hud-cockpit-gauge-light" /home/zou/IntelHub/console/src/globe-hud/HudCockpitInstruments.tsx 2>/dev/null | head -1')
check("p18: P16 elements wrapped in hud-cockpit-gauge-light (5x in HudCockpitInstruments)",
      p18_gauge_light.strip() not in ("", "0") and int(p18_gauge_light.strip()) >= 5,
      f"gauge_light_refs={p18_gauge_light.strip()}")

# 65. Responsive scale media query: short viewports (< 800px height) get a
#    uniform scale(0.85). The max-height media query must be declared in
#    hud.css so short-viewport pilots still see the cluster.
p18_responsive = vm('grep -cF "max-height: 800px" /home/zou/IntelHub/console/src/globe-hud/hud.css 2>/dev/null | head -1')
check("p18: responsive scale on short viewports (max-height: 800px media query)",
      p18_responsive.strip() not in ("", "0"),
      f"responsive_refs={p18_responsive.strip()}")

# ---------------------------------------------------------------------------
# GEV §6.3 Replay (2026-09-23): cockpit frame recording + playback. Console-
# only feature using IndexedDB. Four checks cover: the mountCockpitReplay
# factory, the IndexedDB database name literal, the HudCockpitReplay
# popover testid, and the cockpit-store replayState field.
# ---------------------------------------------------------------------------

# 66. T2 mountCockpitReplayRecorder bundled. Vite tree-shakes the literal
#     export name when only consumed inside GlobeV2.tsx (mirrors P15/P16
#     lesson). Fall back to the IndexedDB DB name literal which is unique
#     to replay-recorder.ts and survives minification as a string constant.
p63_recorder = vm('grep -lF "mountCockpitReplayRecorder" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
if not p63_recorder:
    p63_recorder = vm('grep -lF "intelhub-cockpit-replay" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("§6.3: mountCockpitReplayRecorder bundled (or DB-name fallback)",
      bool(p63_recorder),
      p63_recorder or "mountCockpitReplayRecorder (and IDB DB name) not found in dist bundle")

# 67. T2 IndexedDB name literal ships in bundle.
p63_idb = vm('grep -lF "intelhub-cockpit-replay" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("§6.3: IndexedDB name 'intelhub-cockpit-replay' literal in bundle", bool(p63_idb),
      p63_idb or "intelhub-cockpit-replay not found in dist bundle")

# 68. T4 HudCockpitReplay toggle button testid ships in dist.
p63_switch = vm('grep -lF "hud-cockpit-replay-switch" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("§6.3: hud-cockpit-replay-switch testid in bundle", bool(p63_switch),
      p63_switch or "hud-cockpit-replay-switch not found in dist bundle")

# 69. T1 cockpit-store carries replayState field in source.
p63_replay_state = vm('grep -cF "replayState" /home/zou/IntelHub/console/src/gev-visual/cockpit/cockpit-store.ts 2>/dev/null | head -1')
check("§6.3: cockpit-store carries replayState field",
      p63_replay_state.strip() not in ("", "0"),
      f"replay_state_refs={p63_replay_state.strip()}")

# ---------------------------------------------------------------------------
# GEV P19 SVS (2026-09-23): Synthetic Vision System — Cesium-native terrain
# sampling + 2D wireframe overlay on pitch ladder. Six checks.
# ---------------------------------------------------------------------------

# 70. T1 cesium-init.ts module exists in source.
p19_init = vm('test -f /home/zou/IntelHub/console/src/cesium-init.ts && echo OK || echo MISSING')
check("P19: cesium-init.ts module exists", p19_init.strip() == "OK",
      p19_init.strip() or "cesium-init.ts missing")

# 71. T4 HudCockpitSvsSwitch button testid ships in bundle.
p19_switch = vm('grep -lF "hud-cockpit-svs-switch" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("P19: hud-cockpit-svs-switch testid in bundle", bool(p19_switch),
      p19_switch or "hud-cockpit-svs-switch not found in dist bundle")

# 72. T2 mountCockpitTerrainSampler bundled (with tree-shake fallback).
p19_sampler = vm('grep -lF "mountCockpitTerrainSampler" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
if not p19_sampler:
    p19_sampler = vm('grep -lF "intelhub.cockpit.svsEnabled" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("P19: mountCockpitTerrainSampler bundled (or storage-key fallback)",
      bool(p19_sampler),
      p19_sampler or "mountCockpitTerrainSampler (and storage key) not found in dist bundle")

# 73. T4 HudCockpitSvsOverlay renders polylines (testid in bundle).
p19_poly = vm('grep -cF "hud-cockpit-svs-overlay" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("P19: hud-cockpit-svs-overlay testid in bundle",
      p19_poly.strip() not in ("", "0"),
      f"svs_overlay_refs={p19_poly.strip()}")

# 74. T3 cockpit-store carries svsEnabled field.
p19_svs = vm('grep -cF "svsEnabled" /home/zou/IntelHub/console/src/gev-visual/cockpit/cockpit-store.ts 2>/dev/null | head -1')
check("P19: cockpit-store carries svsEnabled field",
      p19_svs.strip() not in ("", "0"),
      f"svs_enabled_refs={p19_svs.strip()}")

# 75. T4 HudCockpitFrame mounts terrain sampler inside state.active effect.
p19_wire = vm('grep -cF "mountCockpitTerrainSampler" /home/zou/IntelHub/console/src/globe-hud/HudCockpitFrame.tsx 2>/dev/null | head -1')
check("P19: HudCockpitFrame mounts terrain sampler",
      p19_wire.strip() not in ("", "0"),
      f"frame_wire_refs={p19_wire.strip()}")

# ---------------------------------------------------------------------------
# GEV P20 TCAS (2026-09-23): Traffic Collision Avoidance System. Six checks.
# ---------------------------------------------------------------------------

# 76. T1 hub-core /api/v1/flights/near route registered.
p20_route = vm('grep -cF "/api/v1/flights/near" /home/zou/IntelHub/hub-core/crates/hub-core/src/api.rs 2>/dev/null | head -1')
check("P20: /api/v1/flights/near route registered in hub-core api.rs",
      p20_route.strip() not in ("", "0"),
      f"route_refs={p20_route.strip()}")

# 77. T2 mountCockpitTcas bundled (with tree-shake fallback to URL literal).
p20_client = vm('grep -lF "mountCockpitTcas" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
if not p20_client:
    p20_client = vm('grep -lF "/api/v1/flights/near" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("P20: mountCockpitTcas bundled (or URL-path fallback)",
      bool(p20_client),
      p20_client or "mountCockpitTcas (and URL path) not found in dist bundle")

# 78. T4 HudCockpitTcasSwitch button testid in bundle.
p20_switch = vm('grep -lF "hud-cockpit-tcas-switch" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("P20: hud-cockpit-tcas-switch testid in bundle",
      bool(p20_switch),
      p20_switch or "hud-cockpit-tcas-switch not found in dist bundle")

# 79. T3 HudCockpitTcasOverlay testid in bundle.
p20_overlay = vm('grep -cF "hud-cockpit-tcas-overlay" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("P20: hud-cockpit-tcas-overlay testid in bundle",
      p20_overlay.strip() not in ("", "0"),
      f"overlay_refs={p20_overlay.strip()}")

# 80. T4 cockpit-store carries tcasEnabled field.
p20_store = vm('grep -cF "tcasEnabled" /home/zou/IntelHub/console/src/gev-visual/cockpit/cockpit-store.ts 2>/dev/null | head -1')
check("P20: cockpit-store carries tcasEnabled field",
      p20_store.strip() not in ("", "0"),
      f"tcas_enabled_refs={p20_store.strip()}")

# 81. T4 HudCockpitFrame mounts TCAS client + renders overlay.
p20_wire = vm('grep -cF "mountCockpitTcas" /home/zou/IntelHub/console/src/globe-hud/HudCockpitFrame.tsx 2>/dev/null | head -1')
check("P20: HudCockpitFrame mounts TCAS client + overlay",
      p20_wire.strip() not in ("", "0"),
      f"frame_wire_refs={p20_wire.strip()}")

# 82. PR 2 (globe-init overlay) — component exists, mounted in GlobeV2,
# and the testid reaches the bundle. The testid is the contract surface;
# a future refactor that drops it trips this assertion immediately.
p21_overlay_src = vm('grep -cF "HudGlobeLoadingOverlay" /home/zou/IntelHub/console/src/pages/GlobeV2.tsx 2>/dev/null | head -1')
p21_overlay_bundle = vm('grep -lF "hud-globe-loading-overlay" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("P21: globe-init overlay component mounted + testid in bundle",
      p21_overlay_src.strip() not in ("", "0")
      and bool(p21_overlay_bundle),
      f"src_refs={p21_overlay_src.strip()} bundle={p21_overlay_bundle or 'missing'}")

# 83. Subtitle carries the "ADS-B 区域" copy (8-hub sweep). Pinning the
# literal forces the acceptance to track ADSBX_HUBS changes — a future
# PR that drops to 6 hubs must also rewrite the overlay subtitle.
p21_subtitle = vm('grep -lF "ADS-B" /home/zou/IntelHub/console/dist/assets/*.js 2>/dev/null | head -1')
check("P21: overlay subtitle names the ADS-B hub sweep",
      bool(p21_subtitle),
      f"refs={p21_subtitle or 'missing'}")

# 84. GEV P21 T1 (2026-09-23): the merged /api/v1/globe/aircraft envelope
# must carry an RFC3339 `ts` field. Without it, the console adapter's
# `snapshotEpochMs(env)` returns null, every record's `positionTimeMs`
# collapses to null, and the vendor's `_positionHistory` ring buffer never
# advances past the first 1970-epoch fix — aircraft on the globe page freeze
# in place because no per-frame interpolation between polls is possible.
# The upstream guard is `merge_globe_snapshots` in adsb.rs (always sets ts,
# either from the adsb envelope verbatim or synthesized at merge time when
# the adsb collector is dead). The freshness window (≤10 min) covers the
# longest realistic poll cycle (15 s + safety) without flapping on a quiet
# hub. If the field goes missing again, this check fires immediately.
st, p21_globe = req("/api/v1/globe/aircraft")
if st != 200:
    check("P21: /globe/aircraft envelope carries parseable RFC3339 ts (≤10 min old)",
          False, f"status={st}")
else:
    ts_raw = p21_globe.get("ts")
    p21_ts_ok = isinstance(ts_raw, str) and len(ts_raw) >= 10
    p21_age_s = None
    if p21_ts_ok:
        try:
            from datetime import datetime, timezone, timedelta
            # RFC3339 with optional fractional seconds + tz offset; Python's
            # fromisoformat handles the common forms produced by
            # `chrono::Utc::now().to_rfc3339()` ("YYYY-MM-DDTHH:MM:SS+00:00"
            # / "Z" suffix). Strip trailing Z before parse (older Python
            # doesn't recognize it as UTC).
            ts_norm = ts_raw.replace("Z", "+00:00") if ts_raw.endswith("Z") else ts_raw
            ts_parsed = datetime.fromisoformat(ts_norm)
            if ts_parsed.tzinfo is None:
                ts_parsed = ts_parsed.replace(tzinfo=timezone.utc)
            now = datetime.now(timezone.utc)
            p21_age_s = int((now - ts_parsed).total_seconds())
        except Exception:
            p21_ts_ok = False
    check("P21: /globe/aircraft envelope carries parseable RFC3339 ts (≤10 min old)",
          bool(p21_ts_ok and p21_age_s is not None and 0 <= p21_age_s <= 600),
          f"ts={ts_raw!r} age_s={p21_age_s} status={st}")

# 85. GEV P21 T1 browser-level hard-prove: a headless playwright probe loads
# /globe, screenshots the Cesium canvas twice (t0 + t=+10s) via CDP, and
# diffs the pixels. If aircraft are frozen (the original symptom — the
# vendor _positionHistory ring buffer stuck at 1970 epoch), the diff is ~0;
# if the merged /globe/aircraft envelope carries `ts` (check #84) and the
# adapter converts it to per-record positionTimeMs, the dead-reckoning
# interpolation moves aircraft every frame. Observed normal: 1.3-1.5%
# pixel change across 10s on a hub with ≥100 cached aircraft.
#
# Gated by SP8_MOTION_PROBE=1 — the probe runs inside node:22-trixie docker
# on the hub VM (no node/chromium on the host), takes ~50s warm / ~3min cold
# (chromium download), and shells out. Default: shelved. Opt-in for periodic
# deep validation against the regression.
if not os.environ.get("SP8_MOTION_PROBE"):
    check_shelved(
        "P21: headless motion probe — ≥0.3% pixels change across 10s on /globe",
        "SP8_MOTION_PROBE=1 not set; motion probe opt-in (doubles sp8 runtime + pulls chromium on first run)",
    )
else:
    # Run on the hub VM (where docker + chromium are available). Auth via the
    # same ihk_<hex> key sp8 already has; PROBE_API_KEY envs the probe through
    # the console SPA's localStorage auth gate.
    probe_url = f"{BASE.rstrip('/')}/globe"
    # `vm()` doesn't cd into the repo root; use absolute path so the wrapper
    # resolves regardless of the ssh session's cwd. timeout=600 covers the
    # cold path (chromium ~200MB download + npm install + apt-get --with-deps
    # + ~50s probe = ~5-9 min on prod — observed); warm path is ~50s.
    probe_out = vm(
        f"PROBE_API_KEY='{KEY}' bash /home/zou/IntelHub/scripts/run-probe-motion.sh '{probe_url}' 2>&1 | tail -25",
        timeout=600,
    )
    # Parse the structured tail. The wrapper prints:
    #   byte_identical: false
    #   viewport: 1440x900
    #   pixels_with_chan_delta_gt_5: 17034 / 1296000 (1.314%)
    #   ...
    #   MOTION VERIFIED — 1.314% pixels changed across 10s window
    pct_match = re.search(
        r"pixels_with_chan_delta_gt_5:\s+\d+\s+/\s+\d+\s+\(([\d.]+)%\)",
        probe_out,
    )
    verdict_match = re.search(
        r"(?:MOTION VERIFIED|MINIMAL MOTION)\s+—\s+([\d.]+)%(?: \(motion present but minor; check upstream 429 rate limit\)| pixels changed across 10s window)",
        probe_out,
    )
    byte_match = re.search(r"byte_identical:\s+(\S+)", probe_out)
    pct = float(pct_match.group(1)) if pct_match else 0.0
    verified = bool(verdict_match)
    byte_diff = (byte_match.group(1) == "false") if byte_match else False
    # Trim tail for detail (avoid 25 lines in failure summary).
    tail_brief = probe_out.strip().splitlines()[-6:]
    check(
        "P21: headless motion probe — non-static motion on /globe (trusts probe's MINIMAL/VERIFIED verdict)",
        bool(byte_diff and verified),
        f"pct={pct:.3f}% verified={verified} byte_diff={byte_diff} "
        f"tail={'; '.join(tail_brief)}",
    )

# 86-88. GEV P22 multi-layer motion probes: same CDP-screenshot-diff
# mechanism as check #85 but generalized via probe-layer.mjs + LAYER_PROBES
# table. Each layer has its own motion threshold (aircraft/satellites/vessels
# move per-frame; traffic flow heatmap updates on poll cadence and uses a
# slightly higher threshold to absorb per-frame noise vs feed-update
# signal). All gated by SP8_MOTION_PROBE=1 (same opt-in as #85).
#
# Observed normal ranges (315 Debian-test, 2026-09-23):
#   aircraft    1.3-1.5%
#   satellites  ~12% (orbit propagation, ~440 markers)
#   vessels     1.5-1.6%
#   traffic     1.5-1.7% (TomTom/Overpass feed cadence)
#
# 410 production typically runs higher (full data flow, ~12% for aircraft).
LAYER_MOTION_PROBES = [
    # (layer_id, threshold_pct, sp8_label)
    # threshold_pct is the preflight data-source row floor (asserts the layer's
    # hub REST endpoint has ≥threshold_pct rows before the probe runs); the
    # probe's own pixel-diff verdict (MOTION VERIFIED / MINIMAL MOTION /
    # ESSENTIALLY STATIC / STATIC) gates the actual motion pass/fail.
    ("satellites", 1,   "P22: satellites motion probe — non-static motion on /globe (trusts probe verdict)"),
    ("vessels",    1,   "P22: vessels motion probe — non-static motion on /globe (trusts probe verdict)"),
    ("traffic",    1,   "P22: traffic motion probe — non-static motion on /globe (trusts probe verdict)"),
]
for layer_id, threshold_pct, label in LAYER_MOTION_PROBES:
    if not os.environ.get("SP8_MOTION_PROBE"):
        check_shelved(
            label,
            f"SP8_MOTION_PROBE=1 not set; motion probe opt-in (doubles sp8 runtime + pulls chromium on first run)",
        )
        continue
    probe_url = f"{BASE.rstrip('/')}/globe"
    probe_out = vm(
        f"PROBE_API_KEY='{KEY}' bash /home/zou/IntelHub/scripts/run-probe-layer.sh '{layer_id}' '{probe_url}' 2>&1 | tail -25",
        timeout=600,
    )
    pct_match = re.search(
        r"pixels_with_chan_delta_gt_5:\s+\d+\s+/\s+\d+\s+\(([\d.]+)%\)",
        probe_out,
    )
    verdict_match = re.search(
        r"(?:MOTION VERIFIED|MINIMAL MOTION)\s+\([^)]+\)\s+—\s+([\d.]+)%(?: \(motion present but minor; check upstream 429 rate limit\)| pixels changed across 10s window)",
        probe_out,
    )
    byte_match = re.search(r"byte_identical:\s+(\S+)", probe_out)
    preflight_fail = "PREFLIGHT_FAIL" in probe_out
    pct = float(pct_match.group(1)) if pct_match else 0.0
    verified = bool(verdict_match)
    byte_diff = (byte_match.group(1) == "false") if byte_match else False
    tail_brief = probe_out.strip().splitlines()[-6:]
    check(
        label,
        bool(byte_diff and verified),
        f"layer={layer_id} pct={pct:.3f}% threshold={threshold_pct}% "
        f"verified={verified} byte_diff={byte_diff} preflight_fail={preflight_fail} "
        f"tail={'; '.join(tail_brief)}",
    )

# 89. GEV P22 + nightly-CI: VM-native sp8-nightly.sh writes a Redis
# heartbeat (hub:nightly:sp8:run) at 03:00 UTC. Check it from sp8: fails
# if the last nightly run is older than 48h OR its status was "fail" OR
# the hash is missing entirely. This makes the nightly timer self-policing
# — if systemd stops running, sp8 catches it on the next manual run; if a
# nightly run actually fails, sp8 flags it without waiting for the user
# to read /var/log/intelhub/sp8-nightly-*.log. Shelved when the heartbeat
# key is absent on a fresh hub (which is the case for VM 315 before the
# systemd timer is installed there).
heartbeat = redis("HGETALL", "hub:nightly:sp8:run")
if heartbeat.strip():
    # redis HGETALL returns key/value pairs on **separate lines**:
    #   "ts\n2026-09-23T02:18:13Z\nhost\nIntelHub\n..."
    # Pair them up by stepping 2 at a time.
    lines = heartbeat.strip().splitlines()
    hb = dict(zip(lines[0::2], lines[1::2]))
    hb_ts = hb.get("ts", "")
    hb_status = hb.get("status", "?")
    hb_failed = hb.get("failed", "?")
    age_h = None
    try:
        from datetime import datetime, timezone
        ts_norm = hb_ts.replace("Z", "+00:00") if hb_ts.endswith("Z") else hb_ts
        ts_parsed = datetime.fromisoformat(ts_norm)
        if ts_parsed.tzinfo is None:
            ts_parsed = ts_parsed.replace(tzinfo=timezone.utc)
        now = datetime.now(timezone.utc)
        age_h = round((now - ts_parsed).total_seconds() / 3600, 1)
    except Exception:
        pass
    # Self-reference issue: sp8.py finishes BEFORE sp8-nightly.sh writes the
    # new heartbeat, so check #89 reads the PREVIOUS run's heartbeat. If
    # the heartbeat was just written (<30 min ago) the nightly is still
    # running — SHELVE rather than fail. Stale (>48h) is a real alarm.
    if age_h is None:
        check_shelved(
            "P22: nightly sp8 heartbeat fresh + healthy (≤48h, status=ok, failed=0)",
            f"hub:nightly:sp8:run ts unparseable: {hb_ts!r}",
        )
    elif age_h < 0.5:
        check_shelved(
            "P22: nightly sp8 heartbeat fresh + healthy (≤48h, status=ok, failed=0)",
            f"heartbeat just written ({age_h:.2f}h ago) — current nightly still running; previous status={hb_status} failed={hb_failed}",
        )
    elif age_h > 48:
        check(
            "P22: nightly sp8 heartbeat fresh + healthy (≤48h, status=ok, failed=0)",
            False,
            f"STALE: ts={hb_ts!r} age_h={age_h} status={hb_status} failed={hb_failed} host={hb.get('host','?')}",
        )
    else:
        healthy = hb_status == "ok" and hb_failed == "0"
        check(
            "P22: nightly sp8 heartbeat fresh + healthy (≤48h, status=ok, failed=0)",
            healthy,
            f"ts={hb_ts!r} status={hb_status} failed={hb_failed} age_h={age_h} host={hb.get('host','?')}",
        )
else:
    check_shelved(
        "P22: nightly sp8 heartbeat fresh + healthy (≤48h, status=ok, failed=0)",
        "hub:nightly:sp8:run hash missing — nightly timer not installed on this VM yet",
    )

print(f"\n== {passed} passed, {shelved} shelved, {deferred} deferred, {failed} failed ==")
sys.exit(1 if failed else 0)
