#!/usr/bin/env python3
"""
Backfill `graph_change_log` audit rows for pre-phase-3 claims.

Phase 3 added a change_log emit at the v1 `create_claim` write site
(graphw.rs:378). The 36 claims on VM 410 that pre-date that emit have
no audit row, so the mirror worker never gets a chance to write their
Neo4j :Claim + :ABOUT edges. (The v1 path's `graph_write` call *did*
fire when the claim was first created, so the actual Neo4j :Claim node
may already exist — but if the Neo4j mirror ever goes cold (e.g. a
fresh Neo4j container from a backup) there's no audit trail to
recover from.)

This script is a one-shot, idempotent healer:
  1. Find claims that have no `(insert|update, claim, $1, ...)` audit row
  2. Insert one audit row per claim shaped like the new create_claim
     emit: {text, status, entities: [...]}
  3. Mirror worker drains within ~15s, producing the v1 create_claim
     op shape, which the replay worker materialises in Neo4j.

The `entities` array is populated from `claim_entities` joined to
`entities` (kind + name). Claims without claim_entities get an empty
array — the mirror will still produce the :Claim node, just no :ABOUT
edges (consistent with the historical write path).

Run on the VM:
    python3 /home/zou/IntelHub/scripts/backfill-claim-audit.py
"""

from __future__ import annotations
import os
import subprocess

PG_USER = os.environ.get("PGUSER", "intelhub")
PG_DB = os.environ.get("PGDATABASE", "intelhub")

DOCKER_PSQL = ["docker", "exec", "intelhub-postgres", "psql", "-U", PG_USER, "-d", PG_DB, "-tA"]


def psql(sql: str) -> str:
    return subprocess.run(
        DOCKER_PSQL + ["-c", sql],
        capture_output=True, text=True, check=True,
    ).stdout.strip()


def count(sql: str) -> int:
    out = psql(sql).strip()
    return int(out) if out.isdigit() else 0


def main() -> int:
    print("=" * 70)
    print("graph_change_log audit-row backfill (claims)")
    print("=" * 70)
    print(f"PG user={PG_USER} db={PG_DB}")
    print()

    # --- Pre-flight: which claims need backfill? ---
    orphan_count = count(
        """
        SELECT count(*) FROM claims c
        WHERE NOT EXISTS (
            SELECT 1 FROM graph_change_log
            WHERE target_kind = 'claim' AND target_id = c.claim_id::text
        )
        """
    )
    print(f"claims without a (claim) audit row: {orphan_count}")
    if orphan_count == 0:
        print("nothing to backfill — all claims already audited")
        return 0

    # --- Run the backfill ---
    # We use a single INSERT ... SELECT to make this atomic + idempotent
    # in one statement. The (target_kind, target_id) pair isn't currently
    # UNIQUE, so we add a WHERE NOT EXISTS to prevent re-adding rows on
    # rerun.
    inserted = psql(
        """
        WITH to_audit AS (
            SELECT c.claim_id, c.text,
                   COALESCE(
                       (
                           SELECT jsonb_agg(jsonb_build_object('kind', e.kind, 'name', e.name))
                           FROM claim_entities ce
                           JOIN entities e ON e.entity_id = ce.entity_id
                           WHERE ce.claim_id = c.claim_id
                       ),
                       '[]'::jsonb
                   ) AS entities_json
            FROM claims c
            WHERE NOT EXISTS (
                SELECT 1 FROM graph_change_log
                WHERE target_kind = 'claim' AND target_id = c.claim_id::text
            )
        ),
        ins AS (
            INSERT INTO graph_change_log
                (op, target_kind, target_id, before, after, changed_by)
            SELECT
                'insert',
                'claim',
                claim_id::text,
                NULL,
                jsonb_build_object(
                    'text', text,
                    'status', 'unverified',
                    'entities', entities_json
                ),
                'backfill-claim-audit'
            FROM to_audit
            WHERE NOT EXISTS (
                SELECT 1 FROM graph_change_log
                WHERE target_kind = 'claim'
                  AND target_id = to_audit.claim_id::text
            )
            RETURNING change_id
        )
        SELECT count(*) FROM ins
        """
    ).strip()
    n = int(inserted) if inserted and inserted.isdigit() else 0
    print(f"audit rows inserted: {n}")

    # --- Post-state: how many claims still have no audit? ---
    remaining = count(
        """
        SELECT count(*) FROM claims c
        WHERE NOT EXISTS (
            SELECT 1 FROM graph_change_log
            WHERE target_kind = 'claim' AND target_id = c.claim_id::text
        )
        """
    )
    print(f"claims still without audit row: {remaining}")
    print()
    print("Mirror worker drains these within ~15s and produces v1")
    print("create_claim ops; replay worker materialises :Claim + :ABOUT")
    print("edges in Neo4j. No further action needed.")
    print()
    print("=" * 70)
    print("DONE — claim audit-row backfill complete")
    print("=" * 70)
    return 0


if __name__ == "__main__":
    import sys
    sys.exit(main())
