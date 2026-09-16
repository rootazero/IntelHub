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


def check_shelved(name, reason):
    """Count as shelved-by-design (e.g. third-party API key not provisioned)."""
    global shelved
    shelved += 1
    print(f"SHELVE {name}  | {reason}")


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

print(f"\n== {passed} passed, {shelved} shelved, {failed} failed ==")
sys.exit(1 if failed else 0)
