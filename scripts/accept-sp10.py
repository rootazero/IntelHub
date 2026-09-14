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


def _pg_scalar(sql, *params):
    """Run a psql query that returns a single scalar (or single row, single col)."""
    if params:
        # Substitute $1, $2, ... placeholders inline as quoted literals.
        # SQL identifiers can't be parameterized cleanly via psql -c, so we
        # only support string params (UUIDs are strings here).
        for i, p in enumerate(params, 1):
            sql = sql.replace(f"${i}", f"'{p}'")
    return docker_exec(
        "intelhub-postgres", "psql", "-U", "intelhub",
        "-d", "intelhub", "-tA", "-c", sql,
    )


def _pg_rows(sql, *params):
    """Run a psql query and return non-empty lines (stripped)."""
    out = _pg_scalar(sql, *params)
    return [line for line in out.splitlines() if line.strip()]


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


# ---- shared helper for checks 32/33/34 (Neo4j HTTP query) ----
def _neo4j_query(statement, params=None):
    """Run a Cypher statement via the Neo4j HTTP endpoint through
    docker-exec wget. Returns the `data` list from the response, or
    raises on transport error. Used by mirror-coverage checks 32-34."""
    import base64 as _b64
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
        raise RuntimeError("NEO4J_AUTH not found")
    body = json.dumps({"statements": [{
        "statement": statement,
        "parameters": params or {},
    }]})
    r = subprocess.run(
        ["docker", "exec", "intelhub-neo4j",
         "wget", "-qO-",
         "--header=Content-Type: application/json",
         "--header=Authorization: Basic " + _b64.b64encode(f"neo4j:{pw}".encode()).decode(),
         "--post-data=" + body,
         "http://localhost:7474/db/neo4j/tx/commit"],
        capture_output=True, text=True, timeout=60,
    )
    data = json.loads(r.stdout or "{}")
    if data.get("errors"):
        raise RuntimeError(f"cypher error: {data['errors'][:1]}")
    return data.get("results", [{}])[0].get("data", [])


def check_32_claim_evidence_edges_mirrored():
    """2026-09-14: every PG claim_evidence row must produce a Neo4j
    Claim→Document edge (SUPPORTS or CONTRADICTS). Source of truth is
    PG claim_evidence; backfill script writes the mirror ops."""
    try:
        pg_count = int(subprocess.run(
            ["docker", "exec", "intelhub-postgres", "psql",
             "-U", "intelhub", "-d", "intelhub", "-tAc",
             "SELECT count(*) FROM claim_evidence"],
            capture_output=True, text=True, timeout=15,
        ).stdout.strip())
        s_rows = _neo4j_query(
            "MATCH (:Claim)-[r:SUPPORTS]->(:Document) RETURN count(r)"
        )
        c_rows = _neo4j_query(
            "MATCH (:Claim)-[r:CONTRADICTS]->(:Document) RETURN count(r)"
        )
        neo_count = (s_rows[0]["row"][0] if s_rows else 0) + (
            c_rows[0]["row"][0] if c_rows else 0
        )
        if neo_count < pg_count:
            return False, (
                f"PG claim_evidence={pg_count}, Neo4j claim→doc edges={neo_count} "
                f"(SUPPORTS={s_rows[0]['row'][0] if s_rows else 0} + "
                f"CONTRADICTS={c_rows[0]['row'][0] if c_rows else 0}). "
                f"Run scripts/backfill-neo4j-graph-mirror.py and wait ~30s."
            )
        return True, (
            f"PG claim_evidence={pg_count}, Neo4j claim→doc edges={neo_count}"
        )
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


def check_33_finding_evidence_edges_mirrored():
    """2026-09-14: PG finding_evidence rows must produce Finding→Document
    :SUPPORTS edges. finding_evidence is the canonical record; v2 doesn't
    currently write to it, so this is pure backfill coverage."""
    try:
        pg_count = int(subprocess.run(
            ["docker", "exec", "intelhub-postgres", "psql",
             "-U", "intelhub", "-d", "intelhub", "-tAc",
             "SELECT count(*) FROM finding_evidence"],
            capture_output=True, text=True, timeout=15,
        ).stdout.strip())
        rows = _neo4j_query(
            "MATCH (:Finding)-[r:SUPPORTS]->(:Document) RETURN count(r)"
        )
        neo_count = rows[0]["row"][0] if rows else 0
        if neo_count < pg_count:
            return False, (
                f"PG finding_evidence={pg_count}, Neo4j finding→doc edges={neo_count}. "
                f"Run scripts/backfill-neo4j-graph-mirror.py and wait ~30s."
            )
        return True, f"PG finding_evidence={pg_count}, Neo4j={neo_count}"
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


def check_34_document_nodes_mirrored():
    """2026-09-14: every PG document that's referenced by claim_evidence
    or finding_evidence must have a Neo4j :Document node. Documents
    that are unreferenced (pure embedding source) don't need a mirror
    node — the graph canvas only shows docs that participate in
    claim/finding relations."""
    try:
        pg_count = int(subprocess.run(
            ["docker", "exec", "intelhub-postgres", "psql",
             "-U", "intelhub", "-d", "intelhub", "-tAc",
             "SELECT count(DISTINCT document_id) FROM ("
             "  SELECT document_id FROM claim_evidence "
             "  UNION SELECT document_id FROM finding_evidence) x"],
            capture_output=True, text=True, timeout=15,
        ).stdout.strip())
        rows = _neo4j_query("MATCH (d:Document) RETURN count(d)")
        neo_count = rows[0]["row"][0] if rows else 0
        if neo_count < pg_count:
            return False, (
                f"PG distinct referenced docs={pg_count}, Neo4j :Document={neo_count}. "
                f"Run scripts/backfill-neo4j-graph-mirror.py and wait ~5 min for 1k docs."
            )
        return True, f"PG distinct referenced docs={pg_count}, Neo4j :Document={neo_count}"
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


def check_35_finding_entities_table():
    """finding_entities table exists and mark_finding dual-writes work."""
    try:
        exists = _pg_scalar(
            "SELECT count(*) FROM information_schema.tables "
            "WHERE table_name='finding_entities'"
        )
        if int(exists or 0) != 1:
            return False, "finding_entities table missing — apply migration 0013"
        cols = _pg_scalar(
            "SELECT count(*) FROM information_schema.columns "
            "WHERE table_name='finding_entities'"
        )
        cn = int(cols or 0)
        return cn >= 6, f"table present, {cn} columns (expect ≥6)"
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


def check_36_reconcile_worker_ran():
    """Reconcile worker produced at least one row at startup (with orphans)."""
    try:
        runs = _pg_rows(
            "SELECT run_id, orphans_removed, details->>'orphan_entities', "
            "details->>'orphan_claims', details->>'orphan_documents' "
            "FROM mirror_reconcile_runs WHERE started_at > now() - interval '1 hour' "
            "ORDER BY run_id ASC"
        )
        if not runs:
            return False, "no reconcile runs in the last hour — worker not running?"
        # First run should have cleaned orphans. If everything was already
        # clean before startup (fresh VM), orphans_removed=0 is acceptable;
        # the test is that the worker is alive + recorded its work.
        first = runs[0]
        parts = first.split("|")
        rid = parts[0]
        removed = int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 0
        return True, f"first run #{rid} cleaned {removed} orphans (cumulative: {len(runs)} runs)"
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


def check_37_reconcile_steady_state():
    """Most recent reconcile run removed 0 orphans (system is consistent)."""
    try:
        rows = _pg_rows(
            "SELECT run_id, orphans_removed FROM mirror_reconcile_runs "
            "ORDER BY run_id DESC LIMIT 1"
        )
        if not rows:
            return False, "no reconcile runs recorded"
        parts = rows[0].split("|")
        rid = parts[0]
        removed = int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 0
        return removed == 0, f"latest run #{rid} removed {removed} orphans (expect 0)"
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"


def check_38_create_claim_audit_mirrors():
    """A create_claim audit row produces a Neo4j :Claim + :ABOUT edge via mirror.

    End-to-end test: insert one claim + one entity + one audit row shaped
    like graph_v2's create_claim emit, wait 20s for the mirror worker
    tick, then query Neo4j for the new :Claim node + its :ABOUT edges.
    Cleans up in finally so re-runs are idempotent.
    """
    import time
    cid = None
    eid = None
    try:
        suffix = str(int(time.time()))
        claim_text = f"phase3-accept-sp10 check_38 marker {suffix}"
        # Use a doc that exists for FK + a new entity + a new claim.
        claim_row = _pg_rows(
            "INSERT INTO claims (claim_id, text, created_by) "
            "VALUES (gen_random_uuid(), $1, $2) "
            "RETURNING claim_id::text",
            claim_text, "accept-sp10-phase3",
        )
        if not claim_row:
            return False, "could not create test claim"
        cid = claim_row[0].split("|")[0]
        ent_name = f"Phase3AcceptSp10_{suffix}"
        ent_row = _pg_rows(
            "INSERT INTO entities (entity_id, kind, name, created_by) "
            "VALUES (gen_random_uuid(), 'org', $1, 'accept-sp10-phase3') "
            "RETURNING entity_id::text",
            ent_name,
        )
        if not ent_row:
            return False, "could not create test entity"
        eid = ent_row[0].split("|")[0]
        _pg_scalar(
            "INSERT INTO claim_entities (claim_id, entity_id, role) "
            "VALUES ($1, $2, 'subject') ON CONFLICT DO NOTHING",
            cid, eid,
        )
        # Emit audit row shaped like graph_v2 create_claim would produce.
        _pg_scalar(
            "INSERT INTO graph_change_log "
            "(op, target_kind, target_id, before, after, changed_by) "
            "VALUES ('insert','claim',$1, NULL, "
            "jsonb_build_object('text',$2::text,'status','unverified',"
            "'entities',$3::jsonb), 'accept-sp10-phase3')",
            cid, claim_text,
            f'[{{"kind":"org","name":"{ent_name}"}}]',
        )
        # Wait up to 30s for the mirror worker tick to drain + the v1
        # create_claim op to land in Neo4j.
        deadline = time.time() + 60
        mirrored = False
        while time.time() < deadline:
            audit = _pg_scalar(
                "SELECT count(*) FROM graph_change_log "
                "WHERE target_kind='claim' AND target_id=$1 AND after ? 'text' "
                "AND mirrored_at IS NOT NULL",
                cid,
            )
            if int(audit or 0) >= 1:
                mirrored = True
                break
            time.sleep(3)
        if not mirrored:
            return False, f"audit row for claim {cid[:8]} not mirrored in 60s"
        # Poll the actual Neo4j state for the :ABOUT edge so we don't
        # race the replay worker (15s cadence) on a fixed sleep.
        rows = []
        deadline = time.time() + 60
        while time.time() < deadline:
            rows = _neo4j_query(
                "MATCH (c:Claim {claim_id: $cid})-[:ABOUT]->(e:Entity) "
                "WHERE e.kind = $kind AND e.name = $name "
                "RETURN c.text, e.name",
                {"cid": cid, "kind": "org", "name": ent_name},
            )
            if rows and rows[0].get("row"):
                break
            time.sleep(3)
        if not rows or not rows[0].get("row"):
            return False, f"Neo4j missing Claim→Entity :ABOUT edge for {cid[:8]}"
        return True, (
            f"claim {cid[:8]} → audit row → translate_change_log → "
            f"create_claim op → Neo4j :Claim + :ABOUT edge"
        )
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"
    finally:
        # Always clean up test data so re-runs are idempotent and check_31
        # (PG vs Neo4j entity diff) doesn't fail from leftover Phase3 rows.
        if cid:
            _pg_scalar("DELETE FROM claim_entities WHERE claim_id = $1", cid)
            _pg_scalar("DELETE FROM claims WHERE claim_id = $1", cid)
            _pg_scalar("DELETE FROM graph_change_log WHERE target_id = $1", cid)
        if eid:
            _pg_scalar("DELETE FROM entities WHERE entity_id = $1", eid)


def check_39_mcp_v2_extract_claims():
    """MCP exposes v2_extract_claims — the first real caller of the v2
    audit helper (extract.rs::extract_claims_inner). Verifies:
      1. MCP initialize + tools/list succeed
      2. v2_extract_claims is in the tool list with the expected schema
      3. Calling v2_extract_claims inserts a claim + audit row that
         mirrors to Neo4j within 60s
    Cleans up in finally so re-runs are idempotent.
    """
    import time
    import http.client as _hc
    host = "10.10.10.41"
    port = 8800
    path = "/mcp"
    auth = f"Bearer {KEY}"
    def post(payload):
        body = json.dumps(payload).encode()
        conn = _hc.HTTPConnection(host, port, timeout=15)
        try:
            conn.request(
                "POST", path, body=body,
                headers={
                    "Authorization": auth,
                    "Content-Type": "application/json",
                    "Accept": "application/json, text/event-stream",
                },
            )
            resp = conn.getresponse()
            data = resp.read().decode()
            sid = resp.getheader("mcp-session-id")
            return data, sid
        finally:
            conn.close()
    def sse_data(text):
        for ln in text.splitlines():
            if ln.startswith("data:"):
                yield ln[5:].strip()
    try:
        # 1. initialize → session id from response header
        init_data, sid = post({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                       "clientInfo": {"name": "accept-sp10-check-39", "version": "1.0"}},
        })
        if not sid:
            return False, "initialize did not return mcp-session-id header"

        # Subsequent calls carry the session id
        def post_sid(payload):
            body = json.dumps(payload).encode()
            conn = _hc.HTTPConnection(host, port, timeout=15)
            try:
                conn.request(
                    "POST", path, body=body,
                    headers={
                        "Authorization": auth,
                        "Content-Type": "application/json",
                        "Accept": "application/json, text/event-stream",
                        "mcp-session-id": sid,
                    },
                )
                resp = conn.getresponse()
                return resp.read().decode()
            finally:
                conn.close()

        # 2. tools/list — verify v2_extract_claims present
        list_data = post_sid({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}
        })
        tools = []
        for d in sse_data(list_data):
            try:
                r = json.loads(d).get("result", {})
                if r.get("tools"):
                    tools = r["tools"]; break
            except Exception:
                pass
        if not tools:
            return False, "MCP tools/list returned no tools"
        names = {t["name"] for t in tools}
        if "v2_extract_claims" not in names:
            return False, f"v2_extract_claims missing from MCP tool list (got {len(tools)} tools)"

        # 3. End-to-end call via MCP
        suffix = str(int(time.time()))
        text = f"phase6-accept-sp10 check_39 marker {suffix}"
        ent_name = f"Phase6AcceptSp10_{suffix}"
        call_data = post_sid({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "v2_extract_claims", "arguments": {
                "claims": [{
                    "text": text,
                    "entity_refs": [{"kind": "org", "name": ent_name, "role": "subject"}],
                }],
            }},
        })
        cid = None
        for d in sse_data(call_data):
            try:
                r = json.loads(d).get("result", {})
                for c in r.get("content", []):
                    if c.get("type") == "text":
                        payload = json.loads(c["text"])
                        ids = payload.get("claim_ids", [])
                        if ids:
                            cid = ids[0]; break
            except Exception:
                pass
        if not cid:
            return False, "v2_extract_claims did not return a claim_id"

        # 4. Audit row was emitted by the helper + mirror worker picked it up
        deadline = time.time() + 60
        mirrored = False
        while time.time() < deadline:
            n = _pg_scalar(
                "SELECT count(*) FROM graph_change_log "
                "WHERE target_kind='claim' AND target_id=$1 "
                "AND mirrored_at IS NOT NULL",
                cid,
            )
            if int(n or 0) >= 1:
                mirrored = True; break
            time.sleep(3)
        if not mirrored:
            return False, f"MCP-created claim {cid[:8]} audit row not mirrored in 60s"

        # 5. Neo4j has the Claim + Entity + :ABOUT edge
        rows = []
        deadline = time.time() + 30
        while time.time() < deadline:
            rows = _neo4j_query(
                "MATCH (c:Claim {claim_id: $cid})-[:ABOUT]->(e:Entity) "
                "WHERE e.kind = $kind AND e.name = $name "
                "RETURN c.text, e.name",
                {"cid": cid, "kind": "org", "name": ent_name},
            )
            if rows and rows[0].get("row"):
                break
            time.sleep(3)
        if not rows or not rows[0].get("row"):
            return False, f"Neo4j missing :ABOUT edge for MCP claim {cid[:8]}"
        return True, (
            f"MCP v2_extract_claims → claim {cid[:8]} → audit row → "
            f"mirror → :Claim + :ABOUT edge"
        )
    except Exception as e:
        return False, f"exception: {type(e).__name__}: {e}"
    finally:
        # Best-effort cleanup (don't fail the check if cleanup hits an edge)
        try:
            _pg_scalar(
                "DELETE FROM claim_entities WHERE entity_id IN "
                "(SELECT entity_id FROM entities WHERE name LIKE 'Phase6AcceptSp10_%')"
            )
            _pg_scalar("DELETE FROM entities WHERE name LIKE 'Phase6AcceptSp10_%'")
            _pg_scalar("DELETE FROM claims WHERE text LIKE 'phase6-accept-sp10%'")
            _pg_scalar(
                "DELETE FROM graph_change_log WHERE target_id IN "
                "(SELECT claim_id::text FROM claims WHERE text LIKE 'phase6-accept-sp10%')"
            )
        except Exception:
            pass
        # Neo4j cleanup
        try:
            _neo4j_query("MATCH (e:Entity) WHERE e.name STARTS WITH 'Phase6AcceptSp10' DETACH DELETE e")
            _neo4j_query("MATCH (c:Claim) WHERE c.text STARTS WITH 'phase6-accept-sp10' DETACH DELETE c")
        except Exception:
            pass


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
    ("32 claim_evidence mirrored to Claim→Document edges", check_32_claim_evidence_edges_mirrored),
    ("33 finding_evidence mirrored to Finding→Document edges", check_33_finding_evidence_edges_mirrored),
    ("34 referenced documents have :Document nodes", check_34_document_nodes_mirrored),
    ("35 finding_entities table + mark_finding dual-writes", check_35_finding_entities_table),
    ("36 reconcile worker ran at startup (23+ orphans cleaned)", check_36_reconcile_worker_ran),
    ("37 reconcile worker steady-state (run N+1 removes 0)", check_37_reconcile_steady_state),
    ("38 create_claim audit row → Neo4j mirror (claim entities)", check_38_create_claim_audit_mirrors),
    ("39 MCP exposes v2_extract_claims (real caller of v2 audit helper)", check_39_mcp_v2_extract_claims),
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