# SP9→Neo4j Mirror — graph_change_log Replay Worker

**Date:** 2026-09-14
**Status:** Implemented in `fix/graph-v2-mirror` worktree
**Author:** pi

## Background

`graph_v2` (SP9 compiler path) writes canonical state to PG + an audit row to
`graph_change_log`, but never mirrored anything to Neo4j. The graph page and
`/api/v1/graph/*` reads all hit Neo4j, so v2-written data was invisible to the
graph UI. The previous fix (commit `a76aa76`) healed v1's missing `entity_id`
write but the v2 gap was noted as a follow-up.

This change plugs the v2 mirror so any future v2 intent — `assert_entity`,
`assert_relationship`, `link_evidence_to_claim`, `mark_finding_about_entity`,
entity resolution merges, and contradiction writes — reaches Neo4j without
the v2 compiler needing to know about the graph.

## Design

### Shape translation

`graph_change_log` op kinds don't map 1:1 to v1 graph op shapes (different
semantics, especially for `claim`). The replay worker translates what is
mirrorable and silently skips what isn't:

| change_log row                                  | → graph_sync_queue op             | Notes |
|-------------------------------------------------|-----------------------------------|-------|
| `insert`/`update` + `entity`                    | `create_entity`                   | `entity_id`, `kind`, `name` from `target_id` + `after.kind` + `after.name`; `aliases:[]` |
| `insert`/`update` + `relationship`              | `create_relationship`             | `from_kind`/`from_name` from PG lookup of `after.subject`; `to_kind`/`to_name` from PG lookup of `after.object`; `rel_type=after.predicate`; `from_id`/`to_id` passed so both endpoint nodes get `entity_id` backfilled on MERGE |
| `merge` + `entity`                              | `create_entity` on `target_id` (dropped node's canonical row), then attach a `:MERGED_INTO` edge to the kept node | 2 ops, kept in same enqueue batch |
| `contradict` + `contradiction`                  | (skipped)                         | Neo4j has no `:Contradiction` node yet — spec'd separately |
| any `claim` row                                 | (skipped)                         | v1's `create_claim` semantically attaches a claim to a (kind,name) entity; v2's `link_evidence_to_claim` is claim↔document and `mark_finding_about_entity` is finding↔entity. Different shapes; mirrored via document/finding node labels if/when those exist |
| `temporal_close`                                | (skipped)                         | Administrative; Neo4j edge has no valid_until semantics yet |

For relationship translation, the worker needs the `from_kind`/`from_name` /
`to_kind`/`to_name` — change_log only stores uuids. The worker does a single
`SELECT entity_id, kind, name FROM entities WHERE entity_id = ANY($1)` for
the batch and uses the map.

### Idempotency / progress tracking

A new `mirrored_at timestamptz` column on `graph_change_log` is the worker's
progress marker. The worker:
1. `SELECT change_id, op, target_kind, target_id, after FROM graph_change_log
    WHERE mirrored_at IS NULL ORDER BY change_id LIMIT 50` — pulls a batch
2. Translates each row to one or more v1 op shapes
3. Bulk-inserts into `graph_sync_queue` (`op_id = uuid`, `status = 'PENDING'`)
4. `UPDATE graph_change_log SET mirrored_at = now() WHERE change_id = ANY($1)`
   — only marks the log row once the ops are durably in the queue (the
   downstream `graphw::replay_once` worker is what actually writes to Neo4j)
5. Failures: any translation/DB error is logged and the row stays
   `mirrored_at = NULL` for the next tick. No retry counter — a row stays
   unmirrored until the underlying problem (Neo4j down, bad data) is fixed.
   This is intentional: v1's `graph_sync_queue` already has its own
   attempts/error machinery; double-bookkeeping is worse than one.

### Cadence

15s tick matching `graphw::run_replay`. Both workers are spawned in
`server.rs` alongside the existing replay worker; they don't coordinate.
The downstream `graph_sync_queue` is the single Neo4j-write choke point, so
ordering between upstream sources doesn't matter (both MERGE).

### Why not translate inside v2 compiler calls?

The compiler was the alternative, but the v2 path is the audit log — every
intent that mutates canonical state has to write a change_log row anyway
(spec §13), so the worker is "free": no extra writes from the hot path.
The compiler also has 7 intent kinds vs. 1 translation table — separating
audit from mirror is cleaner and lets us evolve translation without
touching the compiler.

## Files

- `hub-core/migrations/0012_change_log_mirror.sql` — `mirrored_at` column
- `hub-core/crates/hub-core/src/graphw.rs` — `run_change_log_mirror` + `translate_change_log`
- `hub-core/crates/hub-core/src/server.rs` — spawn the new worker
- `scripts/backfill-neo4j-graph-mirror.py` — direct enqueue from PG tables
  for the 80 entities + 42 relationships that pre-date this worker (they
  pre-date it because they were created before any v2 intent ever ran, and
  the v1 mirror path also missed them — they're from the original sp2b
  seeder that loaded entities into PG without writing to Neo4j)
- `scripts/accept-sp10.py` — assertion 31: PG-minus-Neo4j (kind,name) diff = 0

## Out of scope (deferred)

- `:Contradiction` Neo4j node label + `contradict` op handler
- `:Claim` ↔ `:Document` edge mirror (would need a Document node label +
  document-to-uuid bridge)
- `:Finding` mirror (would need Finding node label)
- `:MERGED_INTO` edges for `merge` op (splitting a dropped node from a kept
  node is a graph-modeling decision, not a mirror concern)
