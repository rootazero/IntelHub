#!/usr/bin/env python3
"""Backfill Neo4j Entity.entity_id from the PG canonical store.

Why: graphw.rs historically MERGEd Neo4j Entity nodes by (kind, name) and
SET only aliases/attributes — the PG uuid never made it onto the node.
Read paths (get_neighbors, graph canvas) match on entity_id, so mirrored
data was unreachable by uuid. The write-path fix sets entity_id going
forward; this script heals the existing mirror once.

How: pull every (kind, name, entity_id) from PG entities, then UNWIND in
batches against Neo4j via the transactional HTTP endpoint (parameters are
JSON-native — no shell-quoting hazards from names containing quotes).
Sets e.entity_id only where the node exists and the property is NULL.
Idempotent — safe to re-run.

Run on the VM (needs docker access to intelhub-postgres; Neo4j HTTP on 7474):
    python3 scripts/backfill-neo4j-entity-ids.py
"""
import base64
import json
import subprocess
import sys

BATCH = 500


def sh(*args, timeout=120):
    r = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0:
        raise SystemExit(f"command failed: {args[:3]}…\n{r.stderr[:400]}")
    return r.stdout


def neo4j_password():
    out = sh("docker", "inspect", "intelhub-neo4j",
             "--format", "{{range .Config.Env}}{{println .}}{{end}}")
    for line in out.splitlines():
        if line.startswith("NEO4J_AUTH=neo4j/"):
            return line.split("=", 1)[1].split("/", 1)[1]
    raise SystemExit("NEO4J_AUTH not found in intelhub-neo4j env")


def cypher_http(pw, statement, parameters=None, timeout=60):
    """Run one statement via Neo4j's transactional HTTP endpoint.

    Neo4j publishes NO host ports (internal intelhub-data network only), so
    we exec wget inside the container (argv-passed body — no shell quoting
    hazards for names containing quotes/backslashes).
    """
    body = json.dumps({
        "statements": [{"statement": statement, "parameters": parameters or {}}]
    })
    out = subprocess.run(
        ["docker", "exec", "intelhub-neo4j",
         "wget", "-qO-",
         "--header=Content-Type: application/json",
         "--header=Accept: application/json",
         "--header=Authorization: Basic " + base64.b64encode(f"neo4j:{pw}".encode()).decode(),
         "--post-data=" + body,
         "http://localhost:7474/db/neo4j/tx/commit"],
        capture_output=True, text=True, timeout=timeout,
    )
    if out.returncode != 0:
        raise SystemExit(f"neo4j http call failed: {out.stderr[:300]}")
    data = json.loads(out.stdout or "{}")
    if data.get("errors"):
        raise SystemExit(f"cypher error: {data['errors'][:2]}")
    results = data.get("results") or []
    if not results:
        return []
    return [row["row"] for row in results[0].get("data", [])]


def main():
    pw = neo4j_password()

    # 1. PG canonical entities → rows
    raw = sh("docker", "exec", "intelhub-postgres",
             "psql", "-U", "intelhub", "-d", "intelhub", "-tA",
             "-c", "SELECT json_agg(json_build_object('kind',kind,'name',name,'eid',entity_id::text)) FROM entities WHERE merged_into IS NULL")
    rows = json.loads(raw.strip() or "[]") or []
    print(f"PG entities: {len(rows)}")

    # 2. Pre-count Neo4j NULL-id nodes
    before = cypher_http(pw, "MATCH (e:Entity) WHERE e.entity_id IS NULL RETURN count(e)")
    print(f"Neo4j nodes with entity_id NULL (before): {before[0][0] if before else '?'}")

    # 3. Batched UNWIND backfill
    updated_total = 0
    for i in range(0, len(rows), BATCH):
        batch = rows[i:i + BATCH]
        out = cypher_http(pw, (
            "UNWIND $rows AS row "
            "MATCH (e:Entity {kind: row.kind, name: row.name}) "
            "WHERE e.entity_id IS NULL "
            "SET e.entity_id = row.eid "
            "RETURN count(e) AS updated"
        ), parameters={"rows": batch})
        n = out[0][0] if out else 0
        updated_total += n
        print(f"  batch {i // BATCH + 1}: +{n}")

    # 4. Post-count
    after = cypher_http(pw, "MATCH (e:Entity) WHERE e.entity_id IS NULL RETURN count(e)")
    remaining = after[0][0] if after else "?"
    print(f"backfilled: {updated_total}")
    print(f"Neo4j nodes with entity_id NULL (after): {remaining}")
    sys.exit(0 if remaining == 0 else 1)


if __name__ == "__main__":
    main()