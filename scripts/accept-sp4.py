#!/usr/bin/env python3
"""SP4 acceptance (SP6 rewrite) — native monitor signal layer + observability
stack + Radar. The Crucix container is gone; the signal layer now lives inside
hub-core (spec 2026-09-10-intelhub-sp6-native-monitor-design.md).

Usage: accept-sp4.py <agent-api-key> [base-url]
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
        data=json.dumps(body).encode() if body else None,
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


def raw(url, timeout=10):
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read() or b"{}")
    except Exception:
        return 0, {}


def raw_text(url, timeout=10):
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, ""
    except Exception:
        return 0, ""


def vm_json(cmd):
    out = vm(cmd)
    try:
        return json.loads(out) if out else {}
    except Exception:
        return {}


def vm(cmd):
    return subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "IntelHub", cmd],
        capture_output=True, text=True, timeout=60,
    ).stdout.strip()


def pg(sql):
    return vm("DBURL=$(grep '^DATABASE_URL=' /home/zou/IntelHub/core/hub.env | cut -d= -f2-); "
              f"docker exec intelhub-postgres psql \"$DBURL\" -tA -c \"{sql}\"")


print("== SP4 acceptance (SP6 native-monitor rewrite) ==")

# 1. observability containers healthy (crucix intentionally absent)
ps = vm("docker ps --format '{{.Names}} {{.Status}}'")
up = [l.split()[0] for l in ps.splitlines() if "(healthy)" in l or "Up" in l]
for c in ["intelhub-prometheus", "intelhub-node-exporter", "intelhub-cadvisor", "intelhub-grafana"]:
    check(f"container {c} running", c in up, c)
check("crucix container removed (entropy)", "intelhub-crucix" not in ps, "no crucix in docker ps")

# 2. monitor source health (console overview reads the redis health cells)
code, ov = req("/api/v1/overview")
mon = ov.get("radar", {}).get("monitor", {})
check("overview monitor block up", mon.get("up") is True, f"ok={mon.get('sources_ok')}/{mon.get('sources_total')}")
sources = {s["name"]: s["state"] for s in mon.get("sources", [])}
check("monitor sources registered >= 8", len(sources) >= 8, ",".join(sorted(sources)))
keyless = {"usgs", "noaa", "gdelt", "rss", "opensky", "radiation", "kiwisdr"}
ok_keyless = [n for n in keyless if sources.get(n) == "ok"]
check("keyless sources ok >= 4", len(ok_keyless) >= 4, f"ok={sorted(ok_keyless)}")

# 3. geo_events ingested + radar endpoint
code, radar = req("/api/v1/radar/events?limit=500")
check("radar endpoint 200 + events present", code == 200 and radar.get("count", 0) > 0, f"count={radar.get('count')}")
kinds = {e["kind"] for e in radar.get("items", [])}
check("radar kinds diverse (>=2)", len(kinds) >= 2, ",".join(sorted(kinds)))
check("USGS quake layer present (new vs crucix)", "quake" in kinds, ",".join(sorted(kinds)))

# 4. radar filters
code, f1 = req("/api/v1/radar/events?kind=news&limit=500")
check("radar kind=news filter", code == 200 and all(e["kind"] == "news" for e in f1.get("items", [])), f"count={f1.get('count')}")
code, f2 = req("/api/v1/radar/events?severity=info&limit=500")
check("radar severity=info filter", code == 200 and all(e["severity"] == "info" for e in f2.get("items", [])), f"count={f2.get('count')}")

# 5. health probes: monitor + observability components
code, health = req("/api/v1/health")
comps = health.get("components", {})
check("health includes monitor/prometheus/grafana up",
      all(comps.get(c, {}).get("status") == "up" for c in ["monitor", "prometheus", "grafana"]),
      str({c: comps.get(c, {}).get("status") for c in ["monitor", "prometheus", "grafana"]}))

# 6. metrics summary real values
code, m = req("/api/v1/metrics/summary")
host = m.get("host") or {}
check("metrics/summary telemetry ok", m.get("telemetry") == "ok", f"cpu={host.get('cpu_pct')} ram={host.get('ram_pct')}")
check("metrics values sane", 0 <= (host.get("cpu_pct") or -1) <= 100 and 0 < (host.get("ram_pct") or -1) <= 100,
      f"cpu={host.get('cpu_pct')} ram={host.get('ram_pct')} disk={host.get('disk_pct')}")

# 7. prometheus targets all up
tg = vm_json("curl -s -m 5 http://172.30.3.20:9090/api/v1/targets")
targets = {t["labels"]["job"]: t["health"] for t in tg.get("data", {}).get("activeTargets", [])}
check("prometheus targets all up", len(targets) >= 3 and all(v == "up" for v in targets.values()), str(targets))

# 8. grafana datasource + dashboard
ds = vm_json("curl -s -m 5 -u admin:$(grep '^GRAFANA_ADMIN_PASSWORD=' /home/zou/IntelHub/compose/.env | cut -d= -f2) http://172.30.3.22:3000/api/datasources")
check("grafana datasource Prometheus provisioned", isinstance(ds, list) and any(d.get("type") == "prometheus" for d in ds), f"datasources={len(ds) if isinstance(ds, list) else 0}")
dash = vm_json("curl -s -m 5 -u admin:$(grep '^GRAFANA_ADMIN_PASSWORD=' /home/zou/IntelHub/compose/.env | cut -d= -f2) http://172.30.3.22:3000/api/dashboards/uid/intelhub-overview")
check("grafana IntelHub Overview dashboard provisioned", "dashboard" in dash, dash.get("dashboard", {}).get("title", ""))

# 9. console: radar page + offline geojson asset
code, body = raw_text(BASE + "/radar")
check("console /radar SPA route 200", code == 200 and "root" in body, str(code))
code, geo = raw("http://10.10.10.41:8800/world-110m.geo.json")
check("offline basemap geojson served", code == 200 and geo.get("type") == "FeatureCollection", f"features={len(geo.get('features', []))}")

# 10. monitor data layer: chokepoint seed + source rename + fresh rows
n = pg("SELECT count(*) FROM geo_events WHERE source='monitor:chokepoint'")
check("chokepoint reference layer seeded (9)", n.strip() == "9", n)
n = pg("SELECT count(*) FROM geo_events WHERE source LIKE 'crucix:%'")
check("historical crucix:* sources renamed", n.strip() == "0", n)
# Geo event inflow is bursty (quiet hours are normal); collector liveness is
# authoritatively covered by the redis health cells (SP6 checks). >=2 in 2h.
n = pg("SELECT count(DISTINCT source) FROM geo_events WHERE source LIKE 'monitor:%' AND ingested_at > now() - interval '2 hours'")
check("monitor sources producing rows (>=2 in 2h)", n.strip().isdigit() and int(n.strip()) >= 2, n)

# 11. resilience: hub stays green even if individual sources error
# (error isolation is structural — a failing source only flips its own cell)
bad = [n for n, s in sources.items() if s != "ok"]
code, h2 = req("/api/v1/health")
check("hub healthy regardless of per-source errors", code == 200, f"erroring sources: {bad}")

print(f"\n== {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
