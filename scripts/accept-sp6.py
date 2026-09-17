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
# company registry, paid-only token shelved-by-design).
# Bridge 2.1 (2026-09-17): Wikidata (free, keyless entity enrichment
# counterpart to OpenCorp — same OSINT Framework gap, different upstream).
# Bridge 3 (2026-09-17): CourtListener (Public Records → Court Filings,
# free + keyless) + Leaksify (Email/Breach, free + keyless).
# Bridge 4 (2026-09-17): cisa-kev/nvd/osv/opensanctions (already in
# registry, now surfaced in bridge JSON) + tor_exit/ipsum (new,
# free+keyless threat-IP intel: Tor exit-node daily bulk list +
# stamparm's community blocklist aggregator).
# Bridge 5 (2026-09-17): crtsh (cert transparency, keyless) +
# openphish (phishing URL catalog, keyless — replaces retired
# PhishTank public feed) + shodan_internetdb (per-IP enrichment,
# keyless). All three fill distinct OSINT Framework gaps.
# Bridge 6 (2026-09-17): spamhaus_drop (authoritative netblock
# blocklist) + blocklist_de (German fail2ban community per-
# attack-type IP blocklists). ThreatMiner originally proposed
# but blocked from datacenter egress — substituted with
# blocklist_de (per-IP blocklist with fail2ban provenance).
# RDAP originally planned but Verisign RDAP rejects reqwest
# <-> Verisign interop from datacenter egress (HTTP 400);
# tracked separately.
# Bridge 7 (2026-09-17): nominatim (OSM geocoding for
# threat-actor HQ anchoring) + ripestat (RIPE abuse-contact
# lookup for incident response) + wayback (Internet Archive
# URL archive snapshots for phishing forensics). All three
# free keyless, no registration required.
# Bridge 8 (2026-09-17): aws_ip_ranges (AWS public IP-range
# feed for cloud-IP attribution) + gcp_ip_ranges (GCP public
# IP-range feed) + ripe_as_overview (RIPE stat per-ASN holder
# lookup for AS-topology attribution). All three free keyless,
# no registration required.
# Bridge 9 (2026-09-17): ipapi_co (rich IP metadata:
# city/country/lat/lon/ASN/org) + ip_api_com (IP geolocation
# + ASN/ISP, redundant with ipapi_co for cross-validation) +
# ripe_prefix_overview (RIPE stat per-prefix BGP info: prefix,
# AS path, RPKI status). All three free keyless, no
# registration required.
# Bridge 10 (2026-09-17): misp_dynamic_dns (MISP sentinel
# list of dynamic-DNS domains for OSINT false-positive
# suppression) + urlhaus (abuse.ch plain-text malware URL
# feed, ~50K active URLs) + firehol_level1 (FireHOL curated
# IP blocklist, ~4718 CIDR entries aggregated from ~30
# sources). All three free keyless, no registration required.
# Bridge 11 (2026-09-17): misp_rfc5735 (MISP RFC 5735
# Special-Use IPv4 sentinel — 15 CIDRs) + misp_rfc6761 (MISP
# RFC 6761 Special-Use TLD sentinel — 25 entries) +
# tor_exit_details (Tor exit-addresses feed with fingerprint
# + Published + LastStatus + ExitAddress timestamps,
# complementary to existing tor_exit which uses the bare-IP
# torbulkexitlist). All three free keyless.
# Bridge 12 (2026-09-17): romainmarcoux_malicious_ip
# (40K most-malicious IPs aggregator from
# github.com/romainmarcoux/malicious-ip, metadata-pattern
# with count-tagged external_id) + ihr_hegemony (IIJ Lab
# Internet Health Report REST API tracking AS customer-cone
# reach for internet-topology shift detection — default
# watch Cloudflare 13335 + Akamai 20940, env override
# HUB_IHR_HEGEMONY_ASNS) + misp_second_level_tlds (MISP
# sentinel of 10,315 Mozilla-PSL 2nd-level TLDs for OSINT
# false-positive suppression on hostname indicators). All
# three free keyless.
#
# Re-query cells and states here: the OSINT bridge collectors sit at the
# tail of the source stagger (idx > 26 ⇒ ~80-93s before first sweep), so
# the cells captured at script start pre-date their health cells.
osint_bridge = ["etherscan", "defillama", "otx", "urlscan", "gfw",
                "overpass", "ahmia", "opencorp", "wikidata",
                "courtlistener", "leaksify",
                "cisa-kev", "nvd", "osv", "opensanctions",
                "tor_exit", "ipsum",
                "crtsh", "openphish", "shodan_internetdb",
                "spamhaus_drop", "blocklist_de",
                "nominatim", "ripestat", "wayback",
                "aws_ip_ranges", "gcp_ip_ranges", "ripe_as_overview",
                "ipapi_co", "ip_api_com", "ripe_prefix_overview",
                "misp_dynamic_dns", "urlhaus", "firehol_level1",
                "misp_rfc5735", "misp_rfc6761", "tor_exit_details",
                "romainmarcoux_malicious_ip", "ihr_hegemony",
                "misp_second_level_tlds"]
cells = redis("HKEYS", "hub:monitor:health").split()
for c in cells:
    v = redis("HGET", "hub:monitor:health", c)
    states[c] = '"state":"ok"' in v.replace(" ", "")
present = [c for c in osint_bridge if c in cells]
check("OSINT Framework bridge collectors present (40/40)",
      len(present) == 40, f"present={','.join(present)},missing={','.join(set(osint_bridge)-set(present))}")
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
check("OSINT bridge /api/v1/health surfaces 40 collectors (structured)",
      len(ob_collectors) == 40 and ob_names == set(osint_bridge),
      f"got={sorted(ob_names)}")
# Per-collector cadence is exposed so the console can render "next sweep
# ETA" without a separate config call. Verify the shape carries it.
ob_with_cadence = [c for c in ob_collectors if isinstance(c, dict)
                   and isinstance(c.get("cadence_hours"), int) and c["cadence_hours"] > 0]
check("OSINT bridge collectors carry cadence_hours (40/40)",
      len(ob_with_cadence) == 40, f"missing-cadence={set(osint_bridge)-{c['name'] for c in ob_with_cadence}}")
# Structured `ok` count — at least 4 of 5 must be in state=ok. GFW is
# allowed to be absent/shelved by design (see gfw.rs docstring for the
# self-heal trigger conditions).
ob_ok = [c for c in ob_collectors if c.get("state") == "ok"]
# Bridge 2 + 2.1 + 3 + 4: 5 + 1 + 2 + 6 = 14 keyless/cheap collectors
# (overpass/ahmia/wikidata/courtlistener/leaksify/cisa-kev/nvd/osv/
# tor_exit/ipsum) + 3 may-shelve (gfw/opencorp/opensanctions — last
# one is "ok/0-new" without API key, still counts as state=ok).
# So minimum ok = 37/40 (allow 3 to fail or be absent).
check("OSINT bridge structured ok >= 37/40 (GFW+OpenCorp+OpenSanctions may shelve)", len(ob_ok) >= 37,
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

# ---- Globe P1: adsb + celestrak collectors ----
# Runs after the §10 restart (hub-core bounce + 45s drain), so it validates
# the collectors' post-restart first sweep, not just leftover state.
# adsb.lol: cumulative Redis snapshot (TTL 300s, expiry = death detector);
# the envelope went through a Task-5 redesign that dropped `regions_ok` in
# favour of `last_tick` (work-queue cursor name) + `coverage`/`cycle_secs`.
cell_adsb = redis("HGET", "hub:monitor:health", "adsb").strip()
cell_cel = redis("HGET", "hub:monitor:health", "celestrak").strip()
check("globe: adsb+celestrak health cells present",
      cell_adsb not in ("", "nil") and cell_cel not in ("", "nil"),
      f"adsb={cell_adsb[:40]} cel={cell_cel[:40]}")

snap_raw = redis("GET", "hub:globe:aircraft")
try:
    snap = json.loads(snap_raw or "{}")
except Exception:
    snap = {}
# valid-JSON-non-dict (e.g. a bare string/array) must not crash .get() below
if not isinstance(snap, dict):
    snap = {}
check("globe: aircraft snapshot fresh (count>100)",
      int(snap.get("count", 0)) > 100,
      f"count={snap.get('count', 0)} last_tick={snap.get('last_tick')}")

n_sat = pg1("SELECT count(*) FROM satellites")
fresh = pg1("SELECT count(*) FROM satellites WHERE fetched_at > now() - interval '12 hours'")
# Ruling 2 (GEV P2 T5 carry-forward): the old flat n_sat>400 threshold is
# toothless under the six-group catalog (T5 实测: geo alone ~567, all six
# groups ~830). Total floor raised to >600 here; the per-category floor is
# asserted on the live endpoint below.
check("globe: satellites catalog loaded",
      n_sat.isdigit() and int(n_sat) > 600,
      f"rows={n_sat} fresh12h={fresh}")

st_a, body_a = req("/api/v1/globe/aircraft")
st_s, body_s = req("/api/v1/globe/satellites")
check("globe: /api/v1/globe/* endpoints 200",
      st_a == 200 and "aircraft" in body_a and st_s == 200 and body_s.get("count", 0) > 600,
      f"aircraft_http={st_a} satellites_http={st_s} sat_count={body_s.get('count')}")

# ---- GEV P2: T4/T5/T13 engine source endpoints (2026-09-17) ----


def req_text(path, timeout=30):
    """Raw-text GET for TLE/plain endpoints (req() JSON-parses bodies and
    would crash on TLE text). Returns (status, text); error bodies are
    truncated to 200 chars so failure details stay readable."""
    r = urllib.request.Request(BASE + path, headers={"Authorization": f"Bearer {KEY}"})
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")[:200]
    except Exception as e:
        return 0, str(e)[:200]


# T5 实测 (Ruling 2): per-category floor on the live /globe/satellites
# response — the six PG-backed GEV TLE groups must each carry >50 sats.
GEV_TLE_GROUPS = ["stations", "visual", "gps-ops", "glo-ops", "galileo", "geo"]
sat_items = body_s.get("items", []) if isinstance(body_s, dict) else []
by_cat = {}
for _it in sat_items:
    if isinstance(_it, dict):
        _c = _it.get("category", "?")
        by_cat[_c] = by_cat.get(_c, 0) + 1
low_cat = [g for g in GEV_TLE_GROUPS if by_cat.get(g, 0) <= 50]
check("gev: satellites per-category floor (six groups each >50, total >600)",
      not low_cat and isinstance(body_s, dict) and body_s.get("count", 0) > 600,
      f"per-cat={ {g: by_cat.get(g, 0) for g in GEV_TLE_GROUPS} } total={body_s.get('count') if isinstance(body_s, dict) else '?'}")

# T13: merged flights snapshot envelope. `coverage` carries the +opensky
# suffix exactly when an opensky OAuth snapshot is being merged — assert the
# base prefix (startswith), never exact equality.
cov = str(body_a.get("coverage", "")) if isinstance(body_a, dict) else ""
check("gev: flights snapshot envelope (count>100, coverage tolerant)",
      st_a == 200 and int(body_a.get("count", 0)) > 100 and cov.startswith("hotspots+mil+squawk"),
      f"http={st_a} count={body_a.get('count') if isinstance(body_a, dict) else '?'} coverage={cov!r}")

# T4: earthquake layer rows (JSON array body — every row M2.5+ by the
# upstream 2.5_day feed contract). 24h global M2.5+ is practically never
# empty; 0 rows means the USGS collector or the endpoint regressed, so this
# is a hard check. Raw fetch + defensive parse: unmatched routes on a stale
# build fall through to the SPA index.html (200 + HTML), which must fail
# cleanly here, not crash json.loads. Failure detail prints body[:200].
st_q, raw_q = req_text("/api/v1/gev/earthquakes")
try:
    body_q = json.loads(raw_q) if raw_q else []
except Exception:
    body_q = None
n_q = len(body_q) if isinstance(body_q, list) else 0
check("gev: earthquakes endpoint rows>0",
      st_q == 200 and n_q > 0,
      f"http={st_q} rows={n_q} body={raw_q[:200]}")

# T5: celestrak readGroup — six PG-backed groups served as plain-text TLE
# (name\nline1\nline2 blocks, byte-compatible with celestrak gp.php).
# "\n1 " is the line-1 marker of the first TLE block. The not-HTML guard
# rejects the SPA index.html fallback (200 + HTML) that unmatched routes
# serve on builds predating this endpoint.
st_t, text_t = req_text("/api/v1/gev/celestrak/stations")
check("gev: celestrak readGroup stations TLE text",
      st_t == 200 and not text_t.lstrip().startswith("<") and "\n1 " in text_t and len(text_t.splitlines()) > 30,
      f"http={st_t} lines={len(text_t.splitlines())} bytes={len(text_t)}")

# T5: starlink is a thin celestrak.org proxy cached in Redis (upstream
# failure = 502 by design). That is a genuine failure to surface, NOT a
# shelf — 315 verified celestrak.org reachable from the VM egress.
st_x, text_x = req_text("/api/v1/gev/celestrak/starlink")
check("gev: starlink proxied TLE (200 + STARLINK)",
      st_x == 200 and not text_x.lstrip().startswith("<") and "STARLINK" in text_x,
      f"http={st_x} bytes={len(text_x)} head={text_x[:80]!r}")

# T13: opensky OAuth gate. Creds absent -> the collector keeps the keyless
# 900s hotspot loop and the OAuth full-vector layer is shelved-by-design;
# creds present -> the opensky health cell must read ok (the full-vector
# channel never fails the keyless tick, so an unhealthy cell here means a
# real regression). Both HUB_-prefixed and bare env names are accepted by
# oauth_creds() (HUB_* wins); secrets.env is the source of truth.
os_id = secret("HUB_OPENSKY_CLIENT_ID") or secret("OPENSKY_CLIENT_ID")
if not os_id:
    check_shelved("opensky OAuth configured",
                  "HUB_OPENSKY_CLIENT_ID/OPENSKY_CLIENT_ID absent in secrets.env — OAuth full-vector layer shelved-by-design")
else:
    v_os = redis("HGET", "hub:monitor:health", "opensky")
    check("opensky OAuth configured (health cell ok)",
          '"state":"ok"' in v_os.replace(" ", ""), v_os[:120])

print(f"\n== {passed} passed, {shelved} shelved, {failed} failed ==")
sys.exit(1 if failed else 0)
