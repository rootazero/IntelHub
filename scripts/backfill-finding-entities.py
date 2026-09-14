#!/usr/bin/env python3
"""
Backfill PG `finding_entities` table for VM 410.

This script runs on the VM (where the hub-core data lives). It performs TWO
backfill passes:

  1. Audit-replay pass — replay every `(insert|update, claim)` change_log
     row whose `after` JSON has entity_id + relation:"about". These are the
     mirror paths that already produced :ABOUT edges in Neo4j; the PG
     `finding_entities` row is the canonical twin.

  2. Heuristic recovery pass — for each finding that still has zero
     `finding_entities` rows, infer entities from the observation graph:
     for every (entity, document) pair where the entity is observed by
     any document that's in the finding's evidence set, count coverage.
     Insert entities whose coverage ≥ 50% of the finding's evidence docs,
     at confidence 0.6 (distinguishable from explicit mark_finding calls
     which default to confidence 1.0).

Idempotent — re-running is a no-op once finding_entities is fully populated.
After both passes, the mirror worker picks up the change_log audit row
emitted for the heuristic insert (step 4 below) and creates the matching
Neo4j :ABOUT edge.

Run from the VM:
    python3 /home/zou/IntelHub/scripts/backfill-finding-entities.py
"""

from __future__ import annotations
import os
import sys
import json
import subprocess

PG_USER = os.environ.get("PGUSER", "intelhub")
PG_DB = os.environ.get("PGDATABASE", "intelhub")

DOCKER_PSQL = ["docker", "exec", "intelhub-postgres", "psql", "-U", PG_USER, "-d", PG_DB, "-tA"]


def psql(sql: str) -> str:
    """Run SQL via docker exec psql, return stdout as a single string."""
    r = subprocess.run(
        DOCKER_PSQL + ["-c", sql],
        capture_output=True, text=True, check=True,
    )
    return r.stdout


def psql_rows(sql: str) -> list[str]:
    """Run SQL, return non-empty lines."""
    return [line for line in psql(sql).splitlines() if line.strip()]


def count(sql: str) -> int:
    out = psql(sql).strip()
    return int(out) if out.isdigit() else 0


def main() -> int:
    print("=" * 70)
    print("finding_entities backfill")
    print("=" * 70)
    print(f"PG user={PG_USER} db={PG_DB}")
    print()

    # --- Preflight: confirm table exists ---
    exists = count(
        "SELECT count(*) FROM information_schema.tables "
        "WHERE table_name='finding_entities'"
    )
    if exists != 1:
        print("ERROR: finding_entities table missing — apply migration 0013 first.")
        return 2
    print("finding_entities table: present")

    # --- Pass 1: audit replay ---
    print()
    print("Pass 1: replay change_log audit rows")
    print("-" * 70)
    # change_log rows with entity_id in after JSON are mark_finding
    # audit rows. Skip the test-data pollution by filtering changed_by != 'test'.
    audit_count = count(
        "SELECT count(*) FROM graph_change_log "
        "WHERE target_kind='claim' AND after ? 'entity_id' "
        "AND after->>'relation'='about' AND changed_by <> 'test'"
    )
    print(f"candidate audit rows: {audit_count}")
    if audit_count > 0:
        # Bulk insert from change_log — confidence=1.0 (audit trail = explicit)
        psql(
            """
            INSERT INTO finding_entities (finding_id, entity_id, role, confidence, created_by)
            SELECT
                cl.target_id::uuid,
                (cl.after->>'entity_id')::uuid,
                COALESCE(cl.after->>'role', 'about'),
                1.0,
                cl.changed_by
            FROM graph_change_log cl
            WHERE cl.target_kind='claim'
              AND cl.after ? 'entity_id'
              AND cl.after->>'relation'='about'
              AND cl.changed_by <> 'test'
            ON CONFLICT (finding_id, entity_id, role) DO NOTHING
            """
        )
        replayed = count(
            "SELECT count(*) FROM finding_entities WHERE confidence = 1.0"
        )
        print(f"audit-replayed rows (confidence=1.0): {replayed}")
    else:
        print("no audit rows to replay")

    # --- Pass 2: heuristic recovery for findings still at 0 ---
    print()
    print("Pass 2: heuristic recovery (observations → finding_evidence coverage)")
    print("-" * 70)
    uncovered = count(
        """
        SELECT count(*) FROM findings f
        WHERE NOT EXISTS (
            SELECT 1 FROM finding_entities fe
            WHERE fe.finding_id = f.finding_id
        )
        """
    )
    print(f"findings with zero finding_entities: {uncovered}")

    # Count findings with ≥2 evidence docs (the only ones the heuristic
    # could possibly recover — single-doc findings give one observation
    # per entity = no coverage check possible).
    multi_doc_findings = count(
        """
        SELECT count(*) FROM (
            SELECT finding_id FROM finding_evidence
            GROUP BY finding_id HAVING count(*) >= 2
        ) t
        """
    )
    print(f"  of which have ≥2 evidence docs: {multi_doc_findings}")
    if uncovered == 0:
        print("nothing to recover — all findings have entity associations")
    elif multi_doc_findings == 0:
        print(
            "no multi-doc findings to evaluate — Pass 2 cannot recover "
            "anything from the current data shape. This is a one-time "
            "data loss from the pre-phase-3 design (findings reference "
            "documents only, observations join table was empty). Future "
            "findings will populate :ABOUT via the canonical mark_finding "
            "op. See docs/superpowers/specs/2026-09-14-intelhub-graph-v2-"
            "mirror-phase3-proposal.md for the design rationale."
        )
    else:
        # Run the heuristic anyway — if no rows come out, leave a clear
        # message so future operators can investigate the gap.
        inserted = psql(
            """
            WITH
            finding_docs AS (
                SELECT finding_id, count(*) AS doc_count
                FROM finding_evidence
                GROUP BY finding_id
                HAVING count(*) >= 2
            ),
            finding_doc_entities AS (
                SELECT fe.finding_id, o.entity_id,
                       count(DISTINCT fe.document_id) AS hits
                FROM finding_evidence fe
                JOIN observations o ON o.document_id = fe.document_id
                JOIN finding_docs fd ON fd.finding_id = fe.finding_id
                GROUP BY fe.finding_id, o.entity_id
            ),
            recoverable AS (
                SELECT fde.finding_id, fde.entity_id, fd.doc_count,
                       fde.hits::float / fd.doc_count AS coverage
                FROM finding_doc_entities fde
                JOIN finding_docs fd ON fd.finding_id = fde.finding_id
                WHERE fde.hits::float / fd.doc_count >= 0.5
            ),
            ins AS (
                INSERT INTO finding_entities
                    (finding_id, entity_id, role, confidence, created_by)
                SELECT finding_id, entity_id, 'about', 0.6, 'heuristic-backfill'
                FROM recoverable
                ON CONFLICT (finding_id, entity_id, role) DO NOTHING
                RETURNING finding_id, entity_id
            ),
            audit AS (
                INSERT INTO graph_change_log
                    (op, target_kind, target_id, before, after, changed_by)
                SELECT 'insert', 'claim', finding_id::text, NULL,
                       jsonb_build_object('entity_id', entity_id::text, 'relation', 'about'),
                       'heuristic-backfill'
                FROM ins
                RETURNING target_id
            )
            SELECT count(*) FROM audit
            """
        ).strip()
        n = int(inserted) if inserted and inserted.isdigit() else 0
        print(f"heuristic-recovered rows + audit emissions: {n}")
        if n == 0:
            print(
                "0 matches: no multi-doc finding had an entity observed by "
                "≥50% of its evidence docs. Pass 2 is a no-op on the "
                "current data; future mark_finding calls + future entity "
                "observations will populate :ABOUT edges through the "
                "canonical v2 op path."
            )
        else:
            print("audit rows queued; mirror worker drains them within ~15s")
    print()
    print("=" * 70)
    print("DONE — finding_entities backfill complete")
    print("=" * 70)
    return 0


if __name__ == "__main__":
    sys.exit(main())
