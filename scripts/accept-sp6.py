#!/usr/bin/env python3
"""SP6 acceptance — native monitor (hub-core built-in signal collectors).

Covers what SP4/SP5 rewrites don't: collector health cells, USGS quake layer,
secret migration, crucix excision, restart recovery, retention hygiene.

Usage: accept-sp6.py <agent-api-key> [base-url]
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


def vm(cmd):
    return subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "IntelHub", cmd],
        capture_output=True, text=True, timeout=60,
    ).stdout.strip()


PSQL = 'DBURL=$(grep "^DATABASE_URL=" /home/zou/IntelHub/core/hub.env | cut -d= -f2-); U=$(echo $DBURL | sed -E "s|.*://([^:]+):.*|\\1|"); docker exec intelhub-postgres psql -U "$U" -d intelhub -t -A -c'
REDIS = 'docker exec intelhub-redis redis-cli --no-auth-warning -a $(grep "^REDIS_PASSWORD=" /home/zou/IntelHub/compose/.env | cut -d= -f2)'


def pg1(sql):
    out = vm(f'{PSQL} "{sql}"')
    return out.splitlines()[-1].strip() if out else ""


print("== SP6 acceptance: native monitor ==")

# 1. hub exposes monitor component (binary is SP6+)
code, health = req("/api/v1/health")
check("health components.monitor present", "monitor" in health.get("components", {}),
      ",".join(health.get("components", {}).keys()))

# 2. collector health cells (redis)
cells = vm(f"{REDIS} HKEYS hub:monitor:health").split()
check("collector health cells >= 8", len(cells) >= 8, ",".join(sorted(cells)))

# 3. keyless collectors ok
states = {}
for c in cells:
    v = vm(f"{REDIS} HGET hub:monitor:health {c}")
    states[c] = '"state":"ok"' in v.replace(" ", "")
keyless_ok = [c for c in ["usgs", "noaa", "gdelt", "rss", "opensky", "radiation"] if states.get(c)]
check("keyless collectors ok >= 4", len(keyless_ok) >= 4, ",".join(keyless_ok))

# 4. USGS quake layer flowing (new vs crucix)
n = pg1("SELECT count(*) FROM geo_events WHERE source='monitor:usgs' AND occurred_at > now() - interval '2 days'")
check("USGS quake events flowing", n.isdigit() and int(n) > 0, f"usgs={n}")

# 5. key migration: FIRMS working with secrets.env key
n = pg1("SELECT count(*) FROM geo_events WHERE source='monitor:firms'")
check("FIRMS events present (key migrated)", n.isdigit() and int(n) > 0, f"firms={n}")

# 6. ACLED collector visible on the health board (user decision 2026-09: keep
# the error displayed so the pending account tier isn't forgotten; hourly
# retries self-heal on approval). Any state is fine — presence is the check.
v = vm(f"{REDIS} HGET hub:monitor:health acled")
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

# 9. secrets migrated to hub secrets.env
sec = vm("grep -cE '^(FIRMS_MAP_KEY|ACLED_EMAIL)=.+' /home/zou/IntelHub/core/secrets.env")
check("secrets.env holds FIRMS+ACLED keys", sec.strip() == "2", sec)

# 10. restart recovery: bounce hub-core → collectors resume
vm("sudo systemctl restart hub-core")
time.sleep(45)  # startup + staggered first sweeps (fast sources: usgs/noaa 5min cadence, first run immediate)
code, h2 = req("/api/v1/health")
fresh = vm(f"{REDIS} HGET hub:monitor:health usgs")
check("hub-core restart → monitor collector fresh", code == 200 and '"state":"ok"' in fresh.replace(" ", ""), fresh[:100])

# 11. radar serves monitor events end-to-end
code, radar = req("/api/v1/radar/events?limit=100")
srcs = {e.get("source", "") for e in radar.get("items", [])}
check("radar events carry monitor: sources", any(s.startswith("monitor:") for s in srcs), ",".join(sorted(srcs))[:120])

print(f"\n== {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
