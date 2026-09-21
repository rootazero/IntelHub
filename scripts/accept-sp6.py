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


def req_bytes(path, timeout=20):
    """Raw-bytes GET. Unlike `req_text`, this does NOT UTF-8-decode the
    body — JPEG / PNG / GIF magic bytes are invalid UTF-8 and the decoder
    would replace them with U+FFFD, breaking the frame proxy's magic-byte
    guard. Returns (status, bytes)."""
    r = urllib.request.Request(BASE + path, headers={"Authorization": f"Bearer {KEY}"})
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except Exception as e:
        return 0, str(e).encode()[:200]


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
# Count floor calibrated 2026-09-17 (T18 410 deploy): snapshot = trailing
# 600s unique aircraft, but tick latency varies with upstream state (measured
# 60-75s/tick under load → a 13-tick cycle spans ~15min > 600s retention), so
# the count oscillates with cycle phase: trough 79-86 at squawk7700, peak 429.
# count>100 was a coin flip; >50 still catches a dead collector/empty upstream
# while tolerating slow-tick phases. Freshness carries the liveness signal.
check("globe: aircraft snapshot fresh (count>50)",
      int(snap.get("count", 0)) > 50,
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


def req_text_raw(method, path, body=None, extra_headers=None, timeout=30):
    """Method/body-general raw-text call (GEV P3: POST /overpass). Same
    (status, text) contract as req_text; error bodies truncated."""
    headers = {"Authorization": f"Bearer {KEY}"}
    if extra_headers:
        headers.update(extra_headers)
    data = body.encode("utf-8") if isinstance(body, str) else body
    r = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")[:200]
    except Exception as e:
        return 0, str(e)[:200]


# T17 实测 (315 live, 3 consecutive sweeps): real CelesTrak constellation
# sizes are stations~20 / visual~156 / gps-ops~32 / glo-ops~28 / galileo~32 /
# geo~567 — four of six groups are PHYSICALLY incapable of >50 (GPS
# constellation is 31 sats). Floors calibrated to reality with margin: low
# enough to never false-fail on a healthy upstream, high enough to catch a
# group that failed to fetch (0 rows) or silently shrank.
GEV_TLE_FLOORS = {"stations": 10, "visual": 100, "gps-ops": 25,
                  "glo-ops": 20, "galileo": 20, "geo": 400}
sat_items = body_s.get("items", []) if isinstance(body_s, dict) else []
by_cat = {}
for _it in sat_items:
    if isinstance(_it, dict):
        _c = _it.get("category", "?")
        by_cat[_c] = by_cat.get(_c, 0) + 1
low_cat = [g for g, floor in GEV_TLE_FLOORS.items() if by_cat.get(g, 0) < floor]
check("gev: satellites per-category floor (six groups calibrated, total >600)",
      not low_cat and isinstance(body_s, dict) and body_s.get("count", 0) > 600,
      f"per-cat={ {g: by_cat.get(g, 0) for g in GEV_TLE_FLOORS} } total={body_s.get('count') if isinstance(body_s, dict) else '?'}")

# T13: merged flights snapshot envelope. `coverage` carries the +opensky
# suffix exactly when an opensky OAuth snapshot is being merged — assert the
# base prefix (startswith), never exact equality.
cov = str(body_a.get("coverage", "")) if isinstance(body_a, dict) else ""
check("gev: flights snapshot envelope (count>50, coverage tolerant)",
      st_a == 200 and isinstance(body_a, dict) and int(body_a.get("count", 0)) > 50 and cov.startswith("hotspots+mil+squawk"),
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

# ---- GEV P3: vessels/installations/traffic/cctv endpoints (2026-09-17) ----

# T2: vessels ais-live. Without AISSTREAM_API_KEY the collector is not
# registered (env-gated) and the endpoint must STILL answer 200 with
# status:"missing-key" (contract §5: the vendor chip is driven by the
# status field, never by HTTP 503) — that shape is a hard check. With a
# key, rows must flow (count>0) — anything else means the WS collector
# or Redis snapshot regressed.
ais_id = secret("HUB_AISSTREAM_API_KEY") or secret("AISSTREAM_API_KEY")
st_v, body_v = req("/api/v1/gev/ais-live")
check("gev: ais-live envelope 200 + contract status field",
      st_v == 200 and isinstance(body_v, dict) and isinstance(body_v.get("rows"), list)
      and body_v.get("status") in ("missing-key", "connecting", "live", "stale", "reconnecting", "down", "auth-failed"),
      f"http={st_v} status={body_v.get('status') if isinstance(body_v, dict) else '?'!r}")
if not ais_id:
    check_shelved("gev: ais-live rows flowing",
                  "AISSTREAM_API_KEY absent in secrets.env — WS collector shelved-by-design")
else:
    check("gev: ais-live rows flowing",
          st_v == 200 and isinstance(body_v, dict) and len(body_v.get("rows", [])) > 0,
          f"rows={len(body_v.get('rows', [])) if isinstance(body_v, dict) else '?'} status={body_v.get('status') if isinstance(body_v, dict) else '?'}")

# T4: installations — T3 harvests OSM military=* into PG on a 24h cadence
# (first-round runs at startup when the table is empty). rows>0 in PG AND
# the bbox endpoint serving a 1° box around a known-dense region
# (Ramstein AB, Germany: 49.4N 7.6E) with the contract envelope.
n_mi = int(pg1("SELECT count(*) FROM military_installations") or "0")
st_i, body_i = req("/api/v1/gev/installations?south=49.0&west=7.0&north=49.8&east=8.0")
els_i = body_i.get("elements", []) if isinstance(body_i, dict) else []
check("gev: installations catalog rows>0 + bbox endpoint envelope",
      n_mi > 0 and st_i == 200 and isinstance(els_i, list)
      and isinstance(body_i, dict) and "retrievedAt" in body_i and "saturated" in body_i,
      f"pg_rows={n_mi} http={st_i} elements={len(els_i) if isinstance(els_i, list) else '?'} status={body_i.get('status') if isinstance(body_i, dict) else '?'}")
# bbox validation: the inverted/oversized contract branches must 400.
st_i400, _ = req("/api/v1/gev/installations?south=49.8&west=7.0&north=49.0&east=8.0")
check("gev: installations bbox validation 400s on inverted box",
      st_i400 == 400, f"http={st_i400}")

# T5: overpass proxy — a tiny bbox query round-trips (hard check: keyless
# upstream; failure = proxy or egress regression, stays visible). Body
# whitelist behavior is unit-tested hub-side; here the live path matters.
import urllib.parse as _up
_ql = '[out:json][timeout:25];(way["highway"~"^(motorway)$"](10.0,106.0,10.5,106.5););out geom qt;'
st_o, raw_o = req_text_raw("POST", "/api/v1/gev/overpass",
                           "data=" + _up.quote(_ql),
                           {"Content-Type": "application/x-www-form-urlencoded"}, timeout=40)
try:
    body_o = json.loads(raw_o) if raw_o else None
except Exception:
    body_o = None
check("gev: overpass proxy round-trip (200 + elements array)",
      st_o == 200 and isinstance(body_o, dict) and isinstance(body_o.get("elements"), list),
      f"http={st_o} body={raw_o[:120]}")

# T6: tomtom status always answers {hasKey:bool}; the flow tile proxy is
# shelved without TOMTOM_API_KEY (503 by design), hard with one.
st_ts, body_ts = req("/api/v1/gev/tomtom/status")
check("gev: tomtom status hasKey boolean",
      st_ts == 200 and isinstance(body_ts, dict) and isinstance(body_ts.get("hasKey"), bool),
      f"http={st_ts} body={body_ts}")
tt_key = secret("HUB_TOMTOM_API_KEY") or secret("TOMTOM_API_KEY")
if not tt_key:
    check_shelved("gev: tomtom flow tile proxied",
                  "TOMTOM_API_KEY absent in secrets.env — live flow shelved-by-design (engine falls back to simulated mode)")
else:
    st_tf, raw_tf = req_text("/api/v1/gev/tomtom/flow/12/3306/1685")
    check("gev: tomtom flow tile proxied",
          st_tf == 200 and len(raw_tf) > 0, f"http={st_tf} bytes={len(raw_tf)}")

# T8-T11: cctv catalog — static loader (259 rows) + live providers (TfL
# ~890 / Ontario ~944 / 511NY ~2933) → sources>0 hard check. The frame
# proxy spot-check walks up to 3 catalog cameras before failing (single
# town-hall host outages must not fail the suite, but a fully broken
# proxy must).
st_cs, body_cs = req("/api/v1/gev/cctv/sources")
srcs = body_cs.get("sources", []) if isinstance(body_cs, dict) else []
check("gev: cctv catalog sources>0 (static+live providers)",
      st_cs == 200 and len(srcs) > 200,
      f"http={st_cs} sources={len(srcs)}")
_frame_ok = False
_frame_note = "no camera with url in catalog"
# Magic-byte signature per image family. The frame proxy's
# content-sniff guard must return real image bytes — not JSON, not HTML.
# Without this assertion, an upstream regression like TxDOT returning
# {snippet:"..."} JSON or NSW returning HTML 404 pages would slip past
# a 200-only check (the pre-fix black-screen bug, 2026-09-20).
_image_magic = (
    (b"\xff\xd8\xff", "jpeg"),
    (b"\x89PNG\r\n\x1a\n", "png"),
    (b"GIF87a", "gif87a"),
    (b"GIF89a", "gif89a"),
    (b"RIFF", "riff"),  # WEBP (RIFF header — full magic RIFF....WEBP checked after)
)
def _looks_like_image(b: bytes) -> bool:
    if not b:
        return False
    for sig, name in _image_magic:
        if b.startswith(sig):
            if name == "riff":
                return len(b) >= 12 and b[8:12] == b"WEBP"
            return True
    return False
for _cam in [c for c in srcs if isinstance(c, dict) and c.get("url")][:3]:
    _id = _cam.get("id")
    _st_f, _raw_f = req_bytes(f"/api/v1/gev/cctv/frame/{_up.quote(str(_id), safe='')}")
    if _st_f == 200 and _looks_like_image(_raw_f):
        _frame_ok = True
        _frame_note = f"id={_id} bytes={len(_raw_f)} magic=ok"
        break
    _frame_note = f"id={_id} http={_st_f} bytes={len(_raw_f) if _raw_f else 0} image_magic={_looks_like_image(_raw_f or b'')}"
check("gev: cctv frame proxy spot check (200 image with valid magic bytes)",
      _frame_ok, _frame_note)

# ---- GEV P11 follow-up: txdot ITS JSON-envelope guard (2026-09-20) ----
# TxDOT upstream returns `application/json` envelopes that the proxy must
# decode. If the guard regresses, every TxDOT camera returns JSON wrapped
# as `image/jpeg` and the browser sees a black screen (the original bug).
_txdot_check = None
for _cam in [c for c in srcs if isinstance(c, dict) and c.get("sourceKind") == "txdot-its"][:3]:
    _id = _cam.get("id")
    _st_t, _raw_t = req_bytes(f"/api/v1/gev/cctv/frame/{_up.quote(str(_id), safe='')}")
    _txdot_check = (_id, _st_t, len(_raw_t), _looks_like_image(_raw_t))
    if _st_t == 200 and _looks_like_image(_raw_t):
        break
check("gev: txdot JSON-envelope cameras decode to image bytes (no JSON leak)",
      _txdot_check is not None and _txdot_check[1] == 200 and _txdot_check[3],
      f"id={_txdot_check[0] if _txdot_check else 'none'} http={_txdot_check[1] if _txdot_check else '?'} bytes={_txdot_check[2] if _txdot_check else 0} image_magic={_txdot_check[3] if _txdot_check else '?'}")

# ---- GEV P11: per-provider CCTV floor (2026-09-19) ----
# After P11 port, IntelHub ingests from 8 new live providers + 4 static
# catalogs. Each new provider has a documented floor in spec §7.1; if any
# floor drops to zero it's a regression that suggests an upstream feed or
# a parse change broke. Static catalogs (austin/shinjuku/tallinn/warendorf)
# are tiny on purpose and not floor-checked.
#
# Per-provider floors (spec §7.1, calibrated to real constellation size):
#   caltrans≥100 (12 districts, ~1k+ actual)
#   drivebc≥500 (all BC highways)
#   fintraffic≥500 (entire Finland)
#   txdot≥50   (2 districts out of 25 — floor for partial ingest)
#   nsw≥100    (Sydney metropolitan)
#   calgary≥100 (City of Calgary)
#   austin≥100 (Socrata Traffic Cameras, ~450 actual)
#   tarktee=0  (DATEX2 upstream 500 — documented outage, not a floor)
_provider_floors = {
    "caltrans": 100, "drivebc": 500, "fintraffic": 500,
    "txdot": 50, "nsw": 100, "calgary": 100, "austin": 100,
}
_provider_counts = {}
for _p in srcs:
    if isinstance(_p, dict):
        _provider_counts[_p.get("provider", "")] = _provider_counts.get(_p.get("provider", ""), 0) + 1
for _pname, _floor in _provider_floors.items():
    _pcount = _provider_counts.get(_pname, 0)
    check(f"gev: cctv per-provider floor {">"+_pname+">"}>={_floor}",
          _pcount >= _floor, f"rows={_pcount}")

# ---- GEV P9: cockpit weather + summary brief endpoints (2026-09-18) ----

# T2: cockpit weather brief. NOAA (2-step grid) with an Open-Meteo fallback;
# both upstreams down → 503 {error, sources_tried} (contract §3.3). The 7
# metric fields are ALWAYS present (null when a source omits one), so the
# shape check is unconditional; only the values are upstream-dependent.
# NOAA 403s anonymous/default-UA clients — `secrets::noaa_user_agent()`
# resolves HUB_NOAA_UA → NOAA_USER_AGENT → "IntelHub/dev"; Open-Meteo is
# keyless and should still answer 200. EITHER source is a PASS. Both down is
# environmental degradation (overpass precedent, plan Task 5) → SHELVE, not
# FAIL. Timeout is generous: NOAA 2-step (12s × 2) + Open-Meteo (12s).
_weather_fields = ["temperature_c", "wind_speed_kts", "wind_direction_deg",
                   "precipitation_mm", "cloud_cover_pct", "visibility_m",
                   "pressure_hpa"]
st_w, body_w = req("/api/v1/gev/weather?lat=40.0&lon=-74.0", timeout=50)
if st_w == 200 and isinstance(body_w, dict):
    _src_w = body_w.get("source")
    _missing_w = [f for f in _weather_fields if f not in body_w]
    check("gev: weather endpoint 200 + 7-metric contract (P9)",
          _src_w in ("noaa", "open-meteo") and not _missing_w
          and isinstance(body_w.get("fetched_at"), str) and bool(body_w["fetched_at"]),
          f"http={st_w} source={_src_w} missing={_missing_w or 'none'}")
elif st_w == 503:
    check_shelved("gev: weather endpoint 200 + 7-metric contract (P9)",
                  "both NOAA and Open-Meteo unreachable from VM egress (http=503 "
                  f"{body_w.get('sources_tried') if isinstance(body_w, dict) else ''})")
else:
    check("gev: weather endpoint 200 + 7-metric contract (P9)", False,
          f"http={st_w} body={str(body_w)[:120]}")

# T2: cockpit summary brief. The plan assumed acled/reliefweb/gdelt upstreams
# already exist in hub-core — they do NOT (verified T2), so the endpoint is a
# stub-by-design: 200 + bullets:[] + sources:["cache"] + next_refresh_after
# (spec §3.3 空载防御). The stub shape IS the contract, so this is a hard
# check — an empty body / 500 / non-empty invented bullets is a regression.
st_s, body_s = req("/api/v1/gev/summary?entity_id=test")
check("gev: summary endpoint stub contract (P9)",
      st_s == 200 and isinstance(body_s, dict)
      and body_s.get("entity_id") == "test"
      and body_s.get("bullets") == []
      and body_s.get("sources") == ["cache"]
      and isinstance(body_s.get("generated_at"), str) and bool(body_s["generated_at"])
      and isinstance(body_s.get("next_refresh_after"), str)
      and bool(body_s["next_refresh_after"]),
      f"http={st_s} body={str(body_s)[:160]}")

# ---- GEV P12: flight-layer enrichment + track backfill proxies (2026-09-21) ----
# T1/T2 backend parity for the console aircraft source. Both routes are
# keyless-or-gated proxies mounted from api.rs:
#   GET /api/adsbdb/route/{callsign}   (keyless, adsbdb.com CC0)
#   GET /api/opensky-track?icao24=hex  (OAuth-gated; 503 without creds)
# Unmatched routes on a stale build fall through to the SPA index.html
# (200 + text/html), so both checks assert the JSON contract — a route
# regression is caught as a shape failure, never as a false 200 pass.

# T1: adsbdb route proxy. `{found:false}` for an unknown callsign is legal
# (the upstream simply has no schedule); a missing route/shape is a failure.
st_ab, raw_ab = req_text("/api/adsbdb/route/UAL123")
try:
    body_ab = json.loads(raw_ab) if raw_ab else None
except Exception:
    body_ab = None
_ab_bad = []
if not isinstance(body_ab, dict) or "found" not in body_ab:
    _ab_bad.append("missing 'found' key (route not mounted or non-JSON body)")
elif body_ab.get("found"):
    for _k in ("airline", "origin", "destination"):
        if _k not in body_ab:
            _ab_bad.append(f"missing {_k}")
    for _port_name in ("origin", "destination"):
        _port = body_ab.get(_port_name) or {}
        if not isinstance(_port, dict):
            _ab_bad.append(f"{_port_name} not an object")
            continue
        for _k in ("code", "name", "lat", "lon"):
            if _k not in _port:
                _ab_bad.append(f"{_port_name}.{_k} missing")
            elif _k in ("lat", "lon") and _port[_k] is not None \
                    and not isinstance(_port[_k], (int, float)):
                _ab_bad.append(f"{_port_name}.{_k} non-numeric")
check("gev: adsbdb route proxy shape (found bool + port fields)",
      st_ab == 200 and not _ab_bad,
      f"http={st_ab} found={body_ab.get('found') if isinstance(body_ab, dict) else '?'} "
      f"bad={_ab_bad or 'none'} body={raw_ab[:100]!r}")

# T2: OpenSky track backfill proxy. Spec §7.1 wants 200 + `records[]`; without
# OPENSKY_CLIENT_ID/SECRET the route is INTENTIONALLY 503 with a JSON error
# (gev_tracks.rs step 3) — that is still proof the route is mounted, so both
# branches pass. `4ca9b1` is a static hex used by earlier GEV tracks work.
st_tr, raw_tr = req_text("/api/opensky-track?icao24=4ca9b1")
try:
    body_tr = json.loads(raw_tr) if raw_tr else None
except Exception:
    body_tr = None
_tr_bad = []
if st_tr == 200 and isinstance(body_tr, dict):
    _recs = body_tr.get("records")
    if not isinstance(_recs, list):
        _tr_bad.append("'records' missing or not a list")
    else:
        for _rec in _recs[:3]:
            if not isinstance(_rec, dict):
                _tr_bad.append("record not an object")
                continue
            for _k in ("observedAtMs", "latitude", "longitude"):
                if _k not in _rec:
                    _tr_bad.append(f"record missing {_k}")
            if isinstance(_rec.get("observedAtMs"), int) and _rec["observedAtMs"] <= 0:
                _tr_bad.append("observedAtMs <= 0")
            if isinstance(_rec.get("latitude"), (int, float)) \
                    and not (-90 <= _rec["latitude"] <= 90):
                _tr_bad.append("latitude out of [-90,90]")
            if isinstance(_rec.get("longitude"), (int, float)) \
                    and not (-180 <= _rec["longitude"] <= 180):
                _tr_bad.append("longitude out of [-180,180]")
elif st_tr == 503 and isinstance(body_tr, dict) and body_tr.get("error"):
    # Shelved-by-design: OAuth creds absent. Route mounted + documented error.
    pass
else:
    _tr_bad.append("unexpected status/shape (route not mounted or non-JSON body)")
check("gev: opensky-track endpoint contract (200 records[] or 503 missing-creds)",
      not _tr_bad,
      f"http={st_tr} bad={_tr_bad or 'none'} body={raw_tr[:120]!r}")

print(f"\n== {passed} passed, {shelved} shelved, {failed} failed ==")
sys.exit(1 if failed else 0)
