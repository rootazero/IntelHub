#!/usr/bin/env python3
"""Backfill graph_sync_queue with v1 mirror ops for every PG entity and
relationship that has no Neo4j mirror.

Why: even before graph_v2 (SP9) ever wrote a row to graph_change_log, 80
PG entities had no Neo4j mirror at all (the original sp2b seeder loaded
entities into PG without going through v1 create_entity). The
change_log replay worker (graphw::run_change_log_mirror) only handles
v2 going forward — the 80 pre-existing orphans are not in change_log, so
this script heals them by walking the canonical PG tables directly and
enqueueing v1 mirror ops into graph_sync_queue (where the existing
graphw::run_replay worker drains them to Neo4j).

The v1 ops are MERGE-based, so this is idempotent. Safe to re-run.
Re-running on a fully-mirrored DB is a no-op (the existing mirror nodes
get MERGEd to the same state).

Run on the VM:
    python3 scripts/backfill-neo4j-graph-mirror.py
"""
import json
import subprocess
import sys


def psql(sql):
    r = subprocess.run(
        ["docker", "exec", "intelhub-postgres", "psql",
         "-U", "intelhub", "-d", "intelhub", "-tA", "-c", sql],
        capture_output=True, text=True, timeout=120,
    )
    if r.returncode != 0:
        raise SystemExit(f"psql failed: {r.stderr[:400]}")
    return r.stdout.strip()


def enqueue_v1_op(op):
    """Insert one v1 op into graph_sync_queue as PENDING (jsonb) — the
    existing run_replay worker will pick it up and call try_graph_write."""
    r = subprocess.run(
        ["docker", "exec", "intelhub-postgres", "psql",
         "-U", "intelhub", "-d", "intelhub", "-tA", "-c",
         f"INSERT INTO graph_sync_queue (op_id, op) VALUES (gen_random_uuid(), $${json.dumps(op)}$$::jsonb)"],
        capture_output=True, text=True, timeout=30,
    )
    if r.returncode != 0:
        raise SystemExit(f"enqueue failed: {r.stderr[:300]}")


def main():
    # 1. Pull every active entity (skip merged).
    raw = psql(
        "SELECT json_agg(json_build_object('eid', entity_id::text, 'kind', kind, 'name', name)) "
        "FROM entities WHERE merged_into IS NULL"
    )
    entities = json.loads(raw or "[]") or []
    print(f"PG entities: {len(entities)}")

    # 2. Pull every relationship with its endpoint names (join once).
    rels_raw = psql(
        "SELECT json_agg(json_build_object("
        "  'rid', r.relationship_id::text,"
        "  'from_id', r.from_entity::text,"
        "  'to_id',   r.to_entity::text,"
        "  'from_kind', sf.kind, 'from_name', sf.name,"
        "  'to_kind',   st.kind, 'to_name',   st.name,"
        "  'rel_type',  r.rel_type"
        ")) FROM relationships r "
        "JOIN entities sf ON sf.entity_id = r.from_entity "
        "JOIN entities st ON st.entity_id = r.to_entity"
    )
    rels = json.loads(rels_raw or "[]") or []
    print(f"PG relationships: {len(rels)}")

    # 3. Enqueue entity mirrors.
    n_ent = 0
    for e in entities:
        enqueue_v1_op({
            "type": "create_entity",
            "entity_id": e["eid"],
            "kind": e["kind"],
            "name": e["name"],
            "aliases": [],
            "attributes": {},
        })
        n_ent += 1
    print(f"enqueued {n_ent} create_entity ops")

    # 4. Enqueue relationship mirrors.
    n_rel = 0
    for r in rels:
        enqueue_v1_op({
            "type": "create_relationship",
            "relationship_id": r["rid"],
            "from_id": r["from_id"],
            "to_id": r["to_id"],
            "from_kind": r["from_kind"], "from_name": r["from_name"],
            "to_kind": r["to_kind"],     "to_name":   r["to_name"],
            "rel_type": r["rel_type"],
            "attributes": {},
        })
        n_rel += 1
    print(f"enqueued {n_rel} create_relationship ops")
    print("queue drained by graphw::run_replay at 15s cadence; check after ~20s")


if __name__ == "__main__":
    main()