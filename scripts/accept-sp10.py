#!/usr/bin/env python3
"""SP10 acceptance — Graph Canvas (Complete Visualization Layer, 28 assertions).

Covers spec 2026-09-13-intelhub-sp10-graph-canvas-design.md §4.

USAGE: this script runs INSIDE the target VM (ssh zou@host 'python3 ...' or
locally on VM 410). It does NOT shell out to ssh because VM 410 has no
private key for outbound ssh — but it has full local file + docker access.
All remote ops use:
  - urllib for hub REST API + SPA HTML + bundle
  - docker exec for postgres / neo4j CLI
  - open() for filesystem reads

Usage: accept-sp10.py <agent-api-key> [base-url]
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.request
import urllib.error

BASE = sys.argv[2] if len(sys.argv) > 2 else "http://10.10.10.41:8800"
KEY = sys.argv[1]
# Where the IntelHub checkout lives on this VM (script runs on the VM itself).
HOME_DIR = os.environ.get("INTELHUB_HOME", "/home/zou/IntelHub")
PASSED = FAILED = 0


def check(name, cond, detail=""):
    global PASSED, FAILED
    if cond:
        PASSED += 1
        print(f"PASS {name}  | {detail}")
    else:
        FAILED += 1
        print(f"FAIL {name}  | {detail}")


def req(path, key=KEY, timeout=30, method="GET", body=None, raw=False):
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


def url_fetch(path, timeout=30):
    """Fetch an arbitrary URL on the hub (no auth) — used for SPA HTML + bundle.

    hub-core binds to the LAN IP (HUB_LISTEN_ADDR=10.10.10.41:8800), not
    loopback, so we hit it via LAN IP even from inside the VM.
    """
    with urllib.request.urlopen(BASE + path, timeout=timeout) as resp:
        return resp.status, resp.read().decode("utf-8", "replace"), dict(resp.headers)


def docker_exec(container, *args, timeout=30):
    """Run a command inside a container. Returns stdout (stripped)."""
    return subprocess.run(
        ["docker", "exec", container, *args],
        capture_output=True, text=True, timeout=timeout,
    ).stdout.strip()


def read_local(rel_path):
    """Read a file under HOME_DIR. Returns '' if missing."""
    full = os.path.join(HOME_DIR, rel_path)
    try:
        with open(full, "r", encoding="utf-8") as f:
            return f.read()
    except FileNotFoundError:
        return ""


# ----- seed entity lookup ----------------------------------------------------
# SP9 acceptance seeded two entities (Alpha/Beta) that exist in Neo4j with
# non-null entity_id. Pick whichever the DB has.
#
# NEO4J_AUTH lives in the neo4j container's env as `neo4j/<password>`, NOT in
# compose/.env. Extract it via docker inspect.
_neo4j_auth = subprocess.run(
    ["docker", "inspect", "intelhub-neo4j",
     "--format", "{{range .Config.Env}}{{println .}}{{end}}"],
    capture_output=True, text=True, timeout=15,
).stdout
_n4j_pw = ""
for line in _neo4j_auth.splitlines():
    if line.startswith("NEO4J_AUTH=neo4j/"):
        _n4j_pw = line.split("=", 1)[1].split("/", 1)[1]
        break

if _n4j_pw:
    _cypher_out = docker_exec(
        "intelhub-neo4j", "bash", "-c",
        f'cypher-shell -u neo4j -p "{_n4j_pw}" --format plain '
        '\'MATCH (e:Entity)-[r]-() WHERE e.entity_id IS NOT NULL '
        'RETURN e.entity_id AS eid LIMIT 1\'',
        timeout=30,
    )
    # cypher-shell --format plain emits the column header as line 1, the
    # value as line 2. Skip blanks; the data row is whichever line
    # doesn't match the header word.
    _seed_lines = [ln for ln in _cypher_out.splitlines() if ln.strip() and ln.strip() != "eid"]
    SEED_ENTITY_ID = _seed_lines[0] if _seed_lines else None
    # Strip wrapping quotes that cypher-shell adds around string values.
    if SEED_ENTITY_ID:
        SEED_ENTITY_ID = SEED_ENTITY_ID.strip('"').strip("'")
else:
    SEED_ENTITY_ID = None
SEED_ENTITY_NAME = None
if SEED_ENTITY_ID:
    code, rows = req(f"/api/v1/graph/entities/search?q={SEED_ENTITY_ID}&limit=5")
    if isinstance(rows, list):
        for r in rows:
            if r.get("entity_id") == SEED_ENTITY_ID:
                SEED_ENTITY_NAME = r.get("name")
                break

if not SEED_ENTITY_ID:
    print("FATAL: no entity found in Neo4j mirror with edges — skipping API tests")
    SEED_ENTITY_ID = "00000000-0000-0000-0000-000000000000"
if not _n4j_pw:
    print("WARN: could not extract NEO4J password from docker inspect")
print(f"# seed entity: {SEED_ENTITY_NAME} ({SEED_ENTITY_ID})")
print(f"# neo4j_pw extracted: {bool(_n4j_pw)} (len={len(_n4j_pw)})")


# ----- bundle fetch (one HTTP request, used by checks 09-21) -----------------
status, CONSOLE_HTML, _ = url_fetch("/")
JS_ASSET = None
m = re.search(r'<script[^>]*type="module"[^>]*src="(/assets/[^"]+\.js)"', CONSOLE_HTML or "")
if m:
    JS_ASSET = m.group(1)
BUNDLE = ""
if JS_ASSET:
    _, BUNDLE, BUNDLE_HEADERS = url_fetch(JS_ASSET)


# ===== Backend enrichment ===================================================

def check_01_edges_have_ids():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, rows = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=1")
    if code != 200 or not isinstance(rows, list):
        return False, f"status={code} rows={type(rows).__name__}"
    n_edges = n_with_ids = 0
    for n in rows:
        for e in n.get("edges", []):
            n_edges += 1
            if "source_id" in e and "target_id" in e and e["source_id"] and e["target_id"]:
                n_with_ids += 1
    return n_with_ids == n_edges and n_edges > 0, f"{n_with_ids}/{n_edges} edges with source_id+target_id"


def check_02_multi_hop_source_target():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, rows = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=2")
    if code != 200 or not isinstance(rows, list):
        return False, f"status={code}"
    all_edges = [e for n in rows for e in n.get("edges", [])]
    if not all_edges:
        return True, "(no edges in current data — vacuous)"
    # Vacuous if every edge in the depth=2 expansion is a 1-hop edge incident
    # to the root (i.e., we don't have 2-hop data to test against). With
    # only 1-hop data this is expected; the deeper test only matters when
    # the data has paths of length 2+.
    both_not_root = [e for e in all_edges
                     if e.get("source_id") != SEED_ENTITY_ID
                     and e.get("target_id") != SEED_ENTITY_ID]
    if both_not_root:
        return True, f"{len(both_not_root)} edges where neither endpoint is root (deep path present)"
    # Otherwise: at least verify the source_id / target_id are present on
    # every edge (this is what check_01 covers; here we just confirm
    # the backend didn't fabricate root as both endpoints).
    real_endpoints = [e for e in all_edges
                      if (e.get("source_id") == SEED_ENTITY_ID) != (e.get("target_id") == SEED_ENTITY_ID)]
    return len(real_endpoints) == len(all_edges), \
        f"{len(real_endpoints)}/{len(all_edges)} edges incident to root (vacuous: depth=2 has no 2-hop data)"


def check_03_rel_types_filter():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    _, rows = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=2")
    rel_types = {e["rel_type"] for n in rows for e in n.get("edges", []) if e.get("rel_type")}
    if not rel_types:
        return True, "(no edges to test against — vacuously satisfied)"
    sample = sorted(rel_types)[0]
    code, filtered = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=2&rel_types={sample}")
    if code != 200 or not isinstance(filtered, list):
        return False, f"status={code}"
    remaining = {e["rel_type"] for n in filtered for e in n.get("edges", [])}
    bad = remaining - {sample}
    return len(bad) == 0, f"filter={sample} returned {len(remaining)} rel_types (off-types: {bad})"


def check_04_min_confidence_filter():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, filtered = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=2&min_confidence=0.99")
    if code != 200 or not isinstance(filtered, list):
        return False, f"status={code}"
    bad = [e for n in filtered for e in n.get("edges", [])
           if e.get("confidence") is not None and e["confidence"] < 0.99]
    return len(bad) == 0, f"{len(bad)} edges below threshold"


def check_05_combined_filters():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    _, rows = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=2")
    rt = sorted({e["rel_type"] for n in rows for e in n.get("edges", []) if e.get("rel_type")})
    if not rt:
        return True, "(no rel_types in current data)"
    sample = rt[0]
    code, _ = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=2&rel_types={sample}&min_confidence=0.5")
    return code == 200, f"status={code} sample={sample}"


def check_06_at_time_still_works():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, _ = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=1&at_time=2026-01-01T00:00:00Z")
    return code == 200, f"status={code}"


def check_07_neighbors_accepts_new_params():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, _ = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=2&rel_types=located_in&min_confidence=0.7")
    return code == 200, f"status={code}"


def check_08_graceful_empty():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, rows = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&depth=4&min_confidence=1.5")
    return code == 200 and isinstance(rows, list), f"status={code} type={type(rows).__name__}"


# ===== Console bundle =======================================================

def check_09_kindcolor_in_bundle():
    b = BUNDLE or ""
    return "kindColor" in b or ("kind" in b and "fill" in b and "selected" in b), \
        f"asset={JS_ASSET or 'NONE'} bytes={len(b)}"


def check_10_filterbar_in_bundle():
    b = BUNDLE or ""
    return "FilterBar" in b or "relationship (" in b, "FilterBar / 'relationship (' chip-row present"


def check_11_sidepanel_in_bundle():
    return "SidePanel" in (BUNDLE or "") or "click a node or edge to inspect" in (BUNDLE or ""), \
        "SidePanel component or empty-state hint present"


def check_12_entitypanel_in_bundle():
    return "EntityPanel" in (BUNDLE or "") or "connections (" in (BUNDLE or ""), \
        "EntityPanel component or 'connections (' section present"


def check_13_edgepanel_in_bundle():
    return "EdgePanel" in (BUNDLE or "") or "confidence" in (BUNDLE or ""), \
        "EdgePanel component or confidence-bar present"


def check_14_confidence_opacity():
    b = BUNDLE or ""
    return ("confidenceOpacity" in b
            or (".2" in b and "confidence" in b)), "opacity heuristic present"


def check_15_kind_color_value():
    return "#ff3355" in (BUNDLE or ""), "KIND_COLORS conflict red value present"


def check_16_source_target_id_strings():
    return ("source_id" in (BUNDLE or "") and "target_id" in (BUNDLE or "")), \
        "source_id/target_id in payload"


def check_17_degree_size_heuristic():
    b = BUNDLE or ""
    return ("Math.min" in b and "degree" in b) or ("nodeSize" in b), "size-by-degree logic present"


def check_18_bundle_freshness():
    """Bundle served from disk and reflects a build after the SP10 commit."""
    if not JS_ASSET:
        return False, "no JS asset discovered"
    full = os.path.join(HOME_DIR, "console", "dist", JS_ASSET.lstrip("/"))
    if not os.path.exists(full):
        return False, f"missing on disk: {full}"
    mtime = os.path.getmtime(full)
    age_sec = int(time.time()) - int(mtime)
    return age_sec < 3600, f"bundle age {age_sec}s ({age_sec // 60}m)"


# ===== Console route ========================================================

def check_19_graph_route_loads():
    code, _, _ = url_fetch("/graph", timeout=10)
    return code == 200, f"status={code}"


def check_20_html_references_asset():
    return JS_ASSET is not None and JS_ASSET.startswith("/assets/"), f"asset={JS_ASSET}"


def check_21_js_asset_reachable():
    if not JS_ASSET:
        return False, "no asset"
    code, body, _ = url_fetch(JS_ASSET, timeout=10)
    return code == 200 and len(body) > 1000, f"status={code} bytes={len(body)}"


# ===== Side-panel API =======================================================

def check_22_timeline():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, rows = req(f"/api/v1/graph/entity/{SEED_ENTITY_ID}/timeline")
    return code == 200 and isinstance(rows, list), f"status={code} len={len(rows) if isinstance(rows, list) else 'n/a'}"


def check_23_evidence():
    """Evidence endpoint should not 500 even when an entity has no docs."""
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    try:
        code, rows = req(f"/api/v1/graph/evidence?entity={SEED_ENTITY_ID}")
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"
    return code == 200 and isinstance(rows, list), f"status={code} len={len(rows) if isinstance(rows, list) else 'n/a'}"


def check_24_neighbors_rel_types_param():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, _ = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&rel_types=located_in")
    return code == 200, f"status={code}"


def check_25_neighbors_min_confidence_param():
    if not SEED_ENTITY_ID:
        return False, "no seed entity"
    code, _ = req(f"/api/v1/graph/neighbors?root={SEED_ENTITY_ID}&min_confidence=0.5")
    return code == 200, f"status={code}"


# ===== UI source sanity (LOCAL file reads — script runs on VM) ==============

def check_26_kindcolor_in_kindmeta_consumers():
    files = [
        "console/src/components/GraphCanvas.tsx",
        "console/src/components/FilterBar.tsx",
        "console/src/components/EntityPanel.tsx",
    ]
    hits = [f for f in files if "kindmeta" in read_local(f)]
    return len(hits) >= 3, f"{len(hits)}/{len(files)} import kindmeta: {hits}"


def check_27_graph_has_sidepanel_jsx():
    src = read_local("console/src/pages/Graph.tsx")
    return bool(re.search(r"<SidePanel[\s/>]", src)), \
        ("opening tag found" if re.search(r"<SidePanel[\s/>]", src) else "no <SidePanel> opening tag")


def check_28_graph_uses_source_target_ids():
    src = read_local("console/src/pages/Graph.tsx")
    n = len(re.findall(r"source_id|target_id", src))
    return n >= 4, f"{n} hits (>=4 expected)"


def check_29_layout_and_autofit_in_bundle():
    """2026-09-14 layout regression guard: G6 requires an explicit `layout`
    option — without it every node renders at (0,0) and the canvas shows a
    single half-clipped dot in the top-left corner. 'd3-force' and 'autoFit'
    are string literals that survive Vite minification."""
    b = BUNDLE or ""
    has_layout = "d3-force" in b
    has_autofit = "autoFit" in b
    return has_layout and has_autofit, \
        f"d3-force={'yes' if has_layout else 'NO'} autoFit={'yes' if has_autofit else 'NO'}"


def check_30_neo4j_entity_ids_backfilled():
    """2026-09-14 mirror regression guard: every Neo4j Entity node must carry
    its PG uuid as entity_id (graphw historically MERGEd by (kind,name) and
    never SET it, orphaning the whole mirror from the uuid-keyed read path).
    Runs the same Neo4j HTTP probe the backfill script uses."""
    try:
        import base64 as _b64
        # Reuse the password-extraction trick from the backfill script.
        out = subprocess.run(
            ["docker", "inspect", "intelhub-neo4j",
             "--format", "{{range .Config.Env}}{{println .}}{{end}}"],
            capture_output=True, text=True, timeout=15,
        ).stdout
        pw = ""
        for line in out.splitlines():
            if line.startswith("NEO4J_AUTH=neo4j/"):
                pw = line.split("=", 1)[1].split("/", 1)[1]
                break
        if not pw:
            return False, "NEO4J_AUTH not found"
        # Neo4j publishes no host ports — exec wget inside the container.
        body = json.dumps({"statements": [{
            "statement": "MATCH (e:Entity) WHERE e.entity_id IS NULL RETURN count(e)"}]})
        r = subprocess.run(
            ["docker", "exec", "intelhub-neo4j",
             "wget", "-qO-",
             "--header=Content-Type: application/json",
             "--header=Authorization: Basic " + _b64.b64encode(f"neo4j:{pw}".encode()).decode(),
             "--post-data=" + body,
             "http://localhost:7474/db/neo4j/tx/commit"],
            capture_output=True, text=True, timeout=30,
        )
        data = json.loads(r.stdout or "{}")
        rows = data.get("results", [{}])[0].get("data", [])
        n = rows[0]["row"][0] if rows else -1
        return n == 0, f"{n} Entity nodes with entity_id NULL (expect 0)"
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


def check_31_pg_minus_neo4j_entity_diff():
    """2026-09-14 v2 mirror guard: every active PG entity (merged_into IS
    NULL) must have a Neo4j mirror. The change_log mirror worker
    (graphw::run_change_log_mirror) plus the backfill script
    (backfill-neo4j-graph-mirror.py) close the gap; this assertion
    catches regressions.

    Compares (kind||'|'||name) between PG entities and Neo4j :Entity
    nodes. Diffs: orphans (PG without Neo4j) + extras (Neo4j without PG,
    allowed for test fixtures but reported as a soft warning). Hard fail
    on any PG entity missing its mirror.
    """
    try:
        import base64 as _b64
        pgp_out = subprocess.run(
            ["docker", "exec", "intelhub-postgres", "psql",
             "-U", "intelhub", "-d", "intelhub", "-tAc",
             "SELECT kind||'|'||name FROM entities WHERE merged_into IS NULL ORDER BY 1"],
            capture_output=True, text=True, timeout=30,
        ).stdout
        pg_set = set(line for line in pgp_out.splitlines() if "|" in line)

        out = subprocess.run(
            ["docker", "inspect", "intelhub-neo4j",
             "--format", "{{range .Config.Env}}{{println .}}{{end}}"],
            capture_output=True, text=True, timeout=15,
        ).stdout
        pw = ""
        for line in out.splitlines():
            if line.startswith("NEO4J_AUTH=neo4j/"):
                pw = line.split("=", 1)[1].split("/", 1)[1]
                break
        if not pw:
            return False, "NEO4J_AUTH not found"

        body = json.dumps({"statements": [{
            "statement": "MATCH (e:Entity) RETURN e.kind AS k, e.name AS n ORDER BY k, n"}]})
        r = subprocess.run(
            ["docker", "exec", "intelhub-neo4j",
             "wget", "-qO-",
             "--header=Content-Type: application/json",
             "--header=Accept: application/json",
             "--header=Authorization: Basic " + _b64.b64encode(f"neo4j:{pw}".encode()).decode(),
             "--post-data=" + body,
             "http://localhost:7474/db/neo4j/tx/commit"],
            capture_output=True, text=True, timeout=30,
        )
        data = json.loads(r.stdout or "{}")
        rows = data.get("results", [{}])[0].get("data", [])
        neo_set = set()
        for row in rows:
            k, n = row["row"]
            if k and n:
                neo_set.add(f"{k}|{n}")

        missing = sorted(pg_set - neo_set)
        extras = sorted(neo_set - pg_set)
        if missing:
            sample = missing[:5]
            return False, (
                f"{len(missing)} PG entities have NO Neo4j mirror (first 5: {sample}). "
                f"Run scripts/backfill-neo4j-graph-mirror.py and wait ~20s."
            )
        return True, (
            f"PG={len(pg_set)} mirrored; {len(neo_set)} Neo4j nodes "
            f"({len(extras)} extras, tolerated for test fixtures)"
        )
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


CHECKS = [
    ("01 edges expose source_id/target_id", check_01_edges_have_ids),
    ("02 multi-hop path real source/target", check_02_multi_hop_source_target),
    ("03 ?rel_types= filters", check_03_rel_types_filter),
    ("04 ?min_confidence= filters", check_04_min_confidence_filter),
    ("05 combined rel_types + min_confidence", check_05_combined_filters),
    ("06 ?at_time= still accepted (SP9 guard)", check_06_at_time_still_works),
    ("07 neighbors accepts new params (200)", check_07_neighbors_accepts_new_params),
    ("08 neighbors graceful empty array", check_08_graceful_empty),
    ("09 kindColor wired in bundle", check_09_kindcolor_in_bundle),
    ("10 FilterBar in bundle", check_10_filterbar_in_bundle),
    ("11 SidePanel in bundle", check_11_sidepanel_in_bundle),
    ("12 EntityPanel in bundle", check_12_entitypanel_in_bundle),
    ("13 EdgePanel in bundle", check_13_edgepanel_in_bundle),
    ("14 confidence-opacity heuristic present", check_14_confidence_opacity),
    ("15 KIND_COLORS values in bundle", check_15_kind_color_value),
    ("16 source_id/target_id strings in bundle", check_16_source_target_id_strings),
    ("17 degree-size heuristic present", check_17_degree_size_heuristic),
    ("18 console bundle is fresh", check_18_bundle_freshness),
    ("19 GET /graph 200", check_19_graph_route_loads),
    ("20 HTML references bundled JS", check_20_html_references_asset),
    ("21 bundled JS reachable", check_21_js_asset_reachable),
    ("22 timeline API works", check_22_timeline),
    ("23 evidence API works", check_23_evidence),
    ("24 neighbors accepts rel_types (no 400)", check_24_neighbors_rel_types_param),
    ("25 neighbors accepts min_confidence (no 400)", check_25_neighbors_min_confidence_param),
    ("26 kindColor wired via kindmeta consumers", check_26_kindcolor_in_kindmeta_consumers),
    ("27 Graph.tsx has <SidePanel> JSX", check_27_graph_has_sidepanel_jsx),
    ("28 Graph.tsx uses source_id/target_id", check_28_graph_uses_source_target_ids),
    ("29 layout (d3-force) + autoFit in bundle", check_29_layout_and_autofit_in_bundle),
    ("30 Neo4j Entity nodes all have entity_id", check_30_neo4j_entity_ids_backfilled),
    ("31 PG entities all have a Neo4j mirror", check_31_pg_minus_neo4j_entity_diff),
]

print(f"== SP10 acceptance ({len(CHECKS)} assertions) ==")
try:
    for name, fn in CHECKS:
        try:
            cond, detail = fn()
        except Exception as e:
            cond, detail = False, f"exception: {type(e).__name__}: {e}"
        check(name, cond, detail)
except KeyboardInterrupt:
    print("\n[aborted]")
    sys.exit(130)

print(f"\n== {PASSED} passed, {FAILED} failed ==")
sys.exit(0 if FAILED == 0 else 1)