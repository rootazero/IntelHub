# Graph v2 Mirror Phase 3 — design proposal for 3 known gaps

**Date:** 2026-09-14 (preparation), `main @ 29f4e3d`
**Status:** proposal — awaiting user direction on priority + scope
**Background:** Phase 1 (`782dafa`) + Phase 2 (`29f4e3d`) shipped the change_log →
Neo4j mirror for entities, relationships, documents, findings, contradictions,
evidence edges, and MERGED_INTO. Three gaps were documented as out-of-scope.
This doc proposes concrete paths to close them.

## Current state of each gap (after re-investigation)

### Gap 1 — Finding→Entity :ABOUT pre-existing 0/83

**Root cause confirmed.** The 83 pre-existing PG `findings` rows never had a
`mark_finding_about_entity` v2 op applied to them — that op was added after
the seeders ran. The :ABOUT edge is **only** created via the change_log
audit row emitted at `compiler.rs:428`. Old findings have no audit row, no
PG source, and no way to reconstruct associations.

**Source-of-truth question.** Should findings canonically reference entities?
Currently `findings` table has:
- `investigation_id` (FK → investigations)
- `finding_evidence` (FK → documents via supports/contradicts relation)
- **No** entity FK, no `findings_entities` table

The :ABOUT edge is a v2-layer inference ("this finding is *about* this
entity because somebody called `mark_finding` on it"), not a canonical PG
fact. The mark_finding op also has a side-effect of copying entity
observations into finding_evidence, which already gives the finding a
document trail.

### Gap 2 — Delete propagation (1-way mirror, no reconciliation)

**Root cause confirmed.** I grepped all `hub-core/` source for `DELETE FROM
{entities, claims, findings, documents, relationships, claim_evidence,
finding_evidence}` and found **zero production delete operations**. The
mirror is currently 1-way because **nothing deletes**. So Gap 2 is dormant
until a delete path appears.

**Future trigger.** The two paths that could add deletes are:
1. A future API endpoint that lets users retract a claim/finding/entity
2. An admin tool to prune old/orphaned records (seeder, cleanup, etc.)

Both are not yet in scope. Adding a reconciliation worker *now* would be
proactive — defensible but speculative.

### Gap 3 — v2 create_claim does not write change_log

**Root cause confirmed and refined.** Only ONE site inserts into the
`claims` table: `graphw.rs:405` (the v1 MCP `create_claim` path). The v2
compiler (`graph_v2/compiler.rs`) has 6 dispatch branches; none of them
insert into `claims` directly. So the gap is more nuanced:
- v1 MCP `create_claim` → PG claims + v1 `graph_write` → Neo4j mirror ✓
- v2 has NO claim creation site at all
- The "v2 create_claim doesn't write change_log" framing was imprecise —
  v2 doesn't *have* a create_claim op; the gap is that if one were added,
  it wouldn't write change_log automatically

So this gap is **currently dormant**: there is no v2 path producing claims
that bypasses Neo4j mirror. The risk is forward-looking: future v2
extractors / ingestion pipelines could write to PG `claims` without
notifying the mirror.

## Proposed plan (3 work items)

### Item A — Reconcile worker (Gap 2, defensive)

Add a `reconcile_neo4j_with_pg` worker tick (separate from the change_log
mirror) that periodically diffs Neo4j against PG for the 5 main tables
(entities, relationships, claims, findings, documents, claim_evidence,
finding_evidence) and DETACH DELETEs orphans. Run once on startup + every
hour thereafter.

**Why this matters even with zero deletes today:**
- Cheap defense-in-depth (single Cypher query per table)
- Self-heals the 1 extra :Document node we saw during testing (the
  orphan that PG delete cleaned but Neo4j persisted)
- Sets up the pattern for when delete paths eventually arrive

**Scope:** ~80 lines (graphw.rs) + migration for a `last_reconciled_at`
column on a new `mirror_reconcile_runs` table for visibility.

**Risk:** low — diff is by `(kind, name)` / `(from_id, to_id, rel_type)`
/ `(document_id)` / etc. Won't accidentally delete anything that's
in PG. Idempotent. Test by manually deleting a PG row and watching the
next tick clean it up.

### Item B — change_log audit for v1 create_claim + create_finding (Gap 3 hardening)

Add change_log audit-row emit at the two existing PG-insert sites that
currently bypass the change_log mirror:

1. **`graphw.rs:405` `create_claim`** — after `INSERT INTO claims`,
   emit `(insert, claim, $claim_id, {text, status, created_by})` to
   `graph_change_log`. The translate_change_log function gets a new arm:
   `(*, claim, after={text,status,...} AND no linked/entity_id field)`
   → `create_claim` op shape (NEW op shape in `try_graph_write`).

2. **`store.rs:478` `create_finding`** — after `INSERT INTO findings` +
   `finding_evidence`, emit `(insert, claim, $finding_id, {entity_id,
   relation:"about"})` per finding, but only when caller has entity refs
   (currently `create_finding` doesn't take entities). So this becomes
   "if `intent.entities` is non-empty, emit change_log per entity" — but
   since the function signature doesn't currently take entities, the
   simpler fix is to add the audit row inside the v2 `mark_finding` op
   (which is where this is supposed to live) and document that direct
   `create_finding` callers should call `mark_finding` after if they
   want :ABOUT edges.

**Scope:** ~40 lines (graphw.rs) + 1 new op shape (`create_claim` for
translate) + 1 new arm in `try_graph_write`. Also need a new arm in
`translate_change_log` that triggers when `after` has neither `linked`
nor `entity_id`.

**Risk:** medium — adding a new v1 op shape touches the Cypher whitelist
+ the v1 mirror path. Need a translation that re-uses existing patterns.

**Future-proof:** any future v2 path that wants to insert claims
programmatically gets audit-for-free by writing the same change_log row.

### Item C — Finding→Entity canonical table (Gap 1)

Add a PG `finding_entities` table to make :ABOUT a first-class concept:
```
CREATE TABLE finding_entities (
    finding_id    uuid NOT NULL REFERENCES findings(finding_id) ON DELETE CASCADE,
    entity_id     uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
    role          text NOT NULL DEFAULT 'about',
    confidence    real NOT NULL DEFAULT 1.0,
    created_by    text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (finding_id, entity_id, role)
);
```

**Then:**
1. Modify `mark_finding_about_entity` v2 op to **also** insert into
   `finding_entities` (PG-canonical), keeping the change_log emit (which
   keeps the mirror happy).
2. Modify the backfill script to scan change_log `(*, claim, after={entity_id, relation:"about"})`
   rows and replay them into `finding_entities` — recovers past :ABOUT
   pairs.
3. Add a new accept-sp10 check (35): `PG finding_entities ≡ change_log
   about rows` (drift = 0) — guards against future divergence.

**Pre-existing 0/83 finding→entity gap:**
- The 83 findings on VM 410 have ZERO past :ABOUT calls (their change_log
  is empty for entity_id refs). The backfill can only recover what was
  *ever* audited. So the 0/83 will stay 0/83 unless we **also** do a
  re-derivation pass.
- Re-derivation option: for each finding without finding_entities, run
  the same SQL the compiler does — `INSERT INTO finding_evidence SELECT
  f.finding_id, o.document_id, 'supports' FROM observations o JOIN
  entities e ON e.entity_id = o.entity_id WHERE f.investigation_id = ?`
  — and use the observations→entity join to infer entity refs. This is a
  research-quality heuristic (entity observed by evidence document =
  entity this finding is *probably* about). Reasonable first pass,
  flag low confidence.
- **Alternative:** accept that 0/83 is a one-time data loss and document
  it. Future findings will populate :ABOUT via the canonical path. The
  sp9 acceptance test fixtures (Accept9Neighbors*) and the 30 Claim→Entity
  edges from claim_entities cover enough of the graph canvas to demo
  multi-hop navigation.

**Scope decision:** present to user.

## Recommended order

| Step | Effort | Risk | Forward-value |
|---|---|---|---|
| **A. Reconcile worker** | ~80 lines | low | defensive; future-proofs Gap 2 |
| **B. create_claim + create_finding audit** | ~40 lines | medium | closes the audit gap for the two real PG insert sites |
| **C1. finding_entities table + v2 emit + backfill** | ~30 lines | low | makes :ABOUT canonical |
| **C2. re-derive 0/83 via observations heuristic** | ~40 lines | medium | recovers legacy graph density |

I recommend shipping A + B + C1 in one PR, deferring C2 until user
confirms whether they want the heuristic recovery or to accept the
one-time data loss. All four together would be ~190 lines + 1 migration
+ 3 accept checks.

## Open questions for user

1. **Gap 1 (Finding→Entity)**: ship `finding_entities` table + future
   writes, or also re-derive the 0/83 via observations heuristic?
2. **Gap 2 (Reconcile worker)**: ship now as defense-in-depth, or wait
   until a real delete path appears?
3. **Gap 3 (create_claim audit)**: ship the `change_log` audit for v1
   `create_claim`, or also need a v2-side equivalent for future extractors?

Once these are answered I'll execute A + B + C1 as a single
`fix/graph-mirror-phase3` worktree, validate on VM 410, and merge.