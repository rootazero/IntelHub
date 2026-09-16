#!/usr/bin/env python3
"""SP6 acceptance — native monitor (hub-core built-in signal collectors).

Covers what SP4/SP5 rewrites don't: collector health cells, USGS quake layer,
secret migration, crucix excision, restart recovery, retention hygiene.

Usage: accept-sp6.py <agent-api-key> [base-url]
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


def secret(name):
    """Read a single env var value from hub secrets.env. Returns "" if missing."""
    out = vm(f"grep '^{name}=' /home/zou/IntelHub/core/secrets.env 2>/dev/null | cut -d= -f2-").strip()
    return out


def req(path, key=KEY, timeout=15):
    r = urllib.request.Request(BASE + path, headers={"Authorization": f"Bearer {key}"})
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"{}")
        except Exception:
            return e.code, {}


def pg1(sql):
    out = pg(sql)
    return out.splitlines()[-1].strip() if out else ""


print("== SP6 acceptance: native monitor ==")

# 1. hub exposes monitor component (binary is SP6+)
code, health = req("/api/v1/health")
check("health components.monitor present", "monitor" in health.get("components", {}),
      ",".join(health.get("components", {}).keys()))

# 2. collector health cells (redis)
cells = redis("HKEYS", "hub:monitor:health").split()
check("collector health cells >= 8", len(cells) >= 8, ",".join(sorted(cells)))

# 2a. populate per-collector state map once (used by sections 2b, 2c, and 3).
# Captured after the first sweep completes; new collectors get their health
# cell written on first fetch() success regardless of fetched count.
states = {}
for c in cells:
    v = redis("HGET", "hub:monitor:health", c)
    states[c] = '"state":"ok"' in v.replace(" ", "")

# 2b. SP8 batch-A sources visible on the health board (any state — they just
# started; gdelt-style upstream penalties must not fail acceptance)
newA = [c for c in ["reliefweb", "who", "cisa-kev", "gscpi"] if c in cells]
check("SP8-A collectors present (reliefweb/who/cisa-kev/gscpi)", len(newA) == 4, ",".join(newA))
newB = [c for c in ["ofac", "usaspending", "epa-radnet"] if c in cells]
check("SP8-B collectors present (ofac/usaspending/epa-radnet)", len(newB) == 3, ",".join(newB))
check("BLS collector visible (degraded by design)", "bls" in cells, "")
check("SP8-C social collectors present (bluesky/telegram-watch/x)",
      all(c in cells for c in ["bluesky", "telegram-watch", "x"]),
      ",".join(c for c in ["bluesky", "telegram-watch", "x"] if c in cells))

# 2c. OSINT Framework bridge (https://osintframework.com gap analysis 2026-09):
# Etherscan (blockchain large-tx), DefiLlama (DeFi TVL anomaly), OTX (community
# threat-intel), urlscan (live URL scans), GFW (vessel events). All five must be
# present on the health board; the GFW one may be shelved when GFW_API_TOKEN
# is missing (free signup, not deployed yet).
#
# Bridge 2 (2026-09-16): Overpass (OSM POI watcher, keyless), Ahmia (tor
# hidden-service search, keyless HTML scrape), OpenCorporates (global
# company registry, free token shelved-by-design).
#
# Re-query cells and states here: the OSINT bridge collectors sit at the
# tail of the source stagger (idx > 26 ⇒ ~80-93s before first sweep), so
# the cells captured at script start pre-date their health cells.
osint_bridge = ["etherscan", "defillama", "otx", "urlscan", "gfw",
                "overpass", "ahmia", "opencorp"]
cells = redis("HKEYS", "hub:monitor:health").split()
for c in cells:
    v = redis("HGET", "hub:monitor:health", c)
    states[c] = '"state":"ok"' in v.replace(" ", "")
present = [c for c in osint_bridge if c in cells]
check("OSINT Framework bridge collectors present (5/5)",
      len(present) == 5, f"present={','.join(present)},missing={','.join(set(osint_bridge)-set(present))}")
# etherscan / defillama / otx / urlscan run keyless (free public endpoints),
# so they should reach "ok" within a few cycles. GFW may stay in any state if
# its token is missing — that's a known shelf, not a regression.
osint_keyless = [c for c in ["etherscan", "defillama", "otx", "urlscan"] if states.get(c)]
check("OSINT Framework keyless collectors ok >= 2", len(osint_keyless) >= 2, ",".join(osint_keyless))

# 2d. Structured /api/v1/health.osint_bridge (added 2026-09-16). The console
# Monitor page reads the same field, so this check guards both the data
# shape and the structured fetch. Hardcoded list — adding a new bridge
# collector is a deliberate Source-registry change, not a config flag.
ob = health.get("osint_bridge", {})
ob_collectors = ob.get("collectors", [])
ob_names = {c.get("name") for c in ob_collectors if isinstance(c, dict)}
check("OSINT bridge /api/v1/health surfaces 8 collectors (structured)",
      len(ob_collectors) == 8 and ob_names == set(osint_bridge),
      f"got={sorted(ob_names)}")
# Per-collector cadence is exposed so the console can render "next sweep
# ETA" without a separate config call. Verify the shape carries it.
ob_with_cadence = [c for c in ob_collectors if isinstance(c, dict)
                   and isinstance(c.get("cadence_hours"), int) and c["cadence_hours"] > 0]
check("OSINT bridge collectors carry cadence_hours (8/8)",
      len(ob_with_cadence) == 8, f"missing-cadence={set(osint_bridge)-{c['name'] for c in ob_with_cadence}}")
# Structured `ok` count — at least 4 of 5 must be in state=ok. GFW is
# allowed to be absent/shelved by design (see gfw.rs docstring for the
# self-heal trigger conditions).
ob_ok = [c for c in ob_collectors if c.get("state") == "ok"]
# Bridge 2 adds 2 keyless (overpass, ahmia) + 1 key-gated (opencorp).
# Keyless should land ok; opencorp stays absent or "ok/0 new" without token.
# GFW may shelve by design. So minimum ok = 6/8.
check("OSINT bridge structured ok >= 6/8 (GFW+OpenCorp may shelve)", len(ob_ok) >= 6,
      ",".join(f"{c['name']}={c.get('state')}" for c in ob_collectors))

# 3. keyless collectors ok
keyless_ok = [c for c in ["usgs", "noaa", "gdelt", "rss", "opensky", "radiation"] if states.get(c)]
check("keyless collectors ok >= 4", len(keyless_ok) >= 4, ",".join(keyless_ok))

# 4. USGS quake layer flowing (new vs crucix)
n = pg1("SELECT count(*) FROM geo_events WHERE source='monitor:usgs' AND occurred_at > now() - interval '2 days'")
check("USGS quake events flowing", n.isdigit() and int(n) > 0, f"usgs={n}")

# 5. key migration: FIRMS working with secrets.env key
firms_key = secret("FIRMS_MAP_KEY")
if not firms_key:
    check_shelved("FIRMS events present (key migrated)", "FIRMS_MAP_KEY not configured in secrets.env — collector shelved-by-design")
else:
    n = pg1("SELECT count(*) FROM geo_events WHERE source='monitor:firms'")
    check("FIRMS events present (key migrated)", n.isdigit() and int(n) > 0, f"firms={n}")

# 6. ACLED collector visible on the health board (user decision 2026-09: keep
# the error displayed so the pending account tier isn't forgotten; hourly
# retries self-heal on approval). Any state is fine — presence is the check.
acled_email = secret("ACLED_EMAIL")
if not acled_email:
    check_shelved("ACLED collector ran (health cell present)", "ACLED_EMAIL not configured in secrets.env — collector shelved-by-design")
else:
    v = redis("HGET", "hub:monitor:health", "acled")
    check("ACLED collector ran (health cell present)", len(v) > 0, v[:100])

# 7. chokepoint static layer + retention hygiene
n = pg1("SELECT count(*) FROM geo_events WHERE source='monitor:chokepoint'")
check("chokepoint reference layer = 9", n == "9", n)
n = pg1("SELECT count(*) FROM geo_events WHERE source LIKE 'crucix:%'")
check("zero crucix:* rows (rename applied)", n == "0", n)

# 8. crucix excision
ps = vm("docker ps -a --format '{{.Names}}'")
check("crucix container gone", "intelhub-crucix" not in ps, "")
imgs = vm("docker images --format '{{.Repository}}:{{.Tag}}'")
check("crucix image gone", not any("crucix" in i for i in imgs.splitlines()), "")
cfg = vm("bash /home/zou/IntelHub/scripts/hub-compose.sh config 2>/dev/null | grep -c crucix")
check("compose config has zero crucix refs", cfg.strip() == "0", cfg)

# 9. secrets migrated to hub secrets.env — field presence, not value content
sec = vm("grep -cE '^(FIRMS_MAP_KEY|ACLED_EMAIL)=' /home/zou/IntelHub/core/secrets.env")
check("secrets.env has FIRMS+ACLED key fields", sec.strip() == "2", sec)

# 10. restart recovery: bounce hub-core → collectors resume
vm("sudo systemctl restart hub-core")
time.sleep(45)  # startup + staggered first sweeps (fast sources: usgs/noaa 5min cadence, first run immediate)
code, h2 = req("/api/v1/health")
fresh = redis("HGET", "hub:monitor:health", "usgs")
check("hub-core restart → monitor collector fresh", code == 200 and '"state":"ok"' in fresh.replace(" ", ""), fresh[:100])

# 11. radar serves monitor events end-to-end
code, radar = req("/api/v1/radar/events?limit=100")
srcs = {e.get("source", "") for e in radar.get("items", [])}
check("radar events carry monitor: sources", any(s.startswith("monitor:") for s in srcs), ",".join(sorted(srcs))[:120])

print(f"\n== {passed} passed, {shelved} shelved, {failed} failed ==")
sys.exit(1 if failed else 0)
