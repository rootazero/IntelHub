# Graph v2 Mirror Phase 2 — :Contradiction / :Document / :Finding + merge edge

**Date:** 2026-09-14
**Status:** Implemented in `fix/graph-mirror-phase2` worktree (main after merge)
**Author:** pi

## Background

Phase 1 (`782dafa`) added the `graph_change_log → Neo4j` replay worker for
the two op shapes that already had clean v1 op equivalents:
`create_entity` and `create_relationship`. Four change_log op kinds were
still skipped because their v2 semantics had no Neo4j mirror:
- `(contradict, contradiction)` — no `:Contradiction` node label
- `(insert|update, claim)` with `after={linked,relation}` — claim↔document
  edge, but no `:Document` node label and no claim-side bridge
- `(insert|update, claim)` with `after={entity_id,relation:"about"}` —
  finding↔entity "about" link, but no `:Finding` node label
- `(merge, entity)` — entity merge is administrative; no edge

The fix adds the missing node labels + bridges to v1's `try_graph_write`
match arm and translates the change_log rows into the corresponding
op shapes. It also extends the backfill script to bring pre-existing
PG data over (claim_evidence, finding_evidence, the bulk documents/findings
tables) so the graph canvas reflects the canonical store, not just the
post-fix writes.

## Op shape additions

| v1 op shape | Used by | Cypher |
|---|---|---|
| `create_document{document_id, url_canonical, title, source_id}` | backfill + future writes | `MERGE (d:Document {document_id}) SET d.url_canonical, d.title, d.source_id` |
| `create_finding{finding_id, title, claim_text, source_confidence, claim_confidence, investigation_id}` | backfill + future writes | `MERGE (f:Finding {finding_id}) SET f.*` |
| `create_contradiction{contradiction_id, claim_a, claim_b, reason}` | change_log (contradict) | `MERGE (c:Contradiction {contradiction_id}) SET c.* + MATCH both claims + MERGE (a)-[:INVOLVES]->(c), (b)-[:INVOLVES]->(c)` |
| `link_claim_evidence{claim_id, document_id, relation}` | change_log + backfill from claim_evidence | `MERGE (d:Document ...) MERGE (c:Claim ...) MERGE (c)-[r:SUPPORTS\|CONTRADICTS]->(d)` (rel whitelist inline) |
| `link_finding_about_entity{finding_id, entity_id}` | change_log | `MERGE (f:Finding ...) WITH f MATCH (e:Entity {entity_id}) MERGE (f)-[:ABOUT]->(e)` |
| `link_finding_evidence{finding_id, document_id}` | backfill from finding_evidence (no current v2 emit path) | `MERGE (f:Finding ...) MERGE (d:Document ...) MERGE (f)-[:SUPPORTS]->(d)` |
| merge reuse: `create_relationship{rel_type:MERGED_INTO}` | change_log (merge) | same v1 create_relationship arm with `MERGED_INTO` whitelisted in REL_TYPES |

## Worker translation table (translate_change_log)

| change_log row | v1 op shape(s) |
|---|---|
| `(contradict, contradiction)` | `create_contradiction` |
| `(insert\|update, claim)` with `after={linked, relation}` | `link_claim_evidence` |
| `(insert\|update, claim)` with `after={entity_id, relation:"about"}` | `link_finding_about_entity` |
| `(merge, entity)` | `create_relationship{rel_type:MERGED_INTO}` |

The claim-evidence vs finding-about dispatch is by `after` shape — the
v2 compiler binds different intent fields into the JSON, so it's
unambiguous.

## merge edge design (modeling decision)

When `resolve::apply_merge` merges entity A into entity B (junk-entity
detection at the v2 resolver), we surface it as a `:MERGED_INTO` edge
from A to B. Both nodes stay in Neo4j; the audit edge lets the graph
reader traverse the history. The canonical kept node (B) is what new
queries should follow — the dropped node (A) is a tombstone. This is
the conservative choice: it preserves existing edges from A (which
might point to entities that B doesn't otherwise know about) and keeps
the merge reversible if the resolver ever decides to split again.

Alternative considered: DELETE the dropped node and rewire all its
edges. Rejected because (a) the resolver's confidence score doesn't
justify irreversible destruction, (b) auditing a deletion is harder
than auditing an edge, (c) the backfill/canonical store relationship
gets muddier if Neo4j diverges from PG.

## Backfill

`scripts/backfill-neo4j-graph-mirror.py` extended with four new sections:
1. All 942 `documents` rows → `create_document`
2. All 83 `findings` rows → `create_finding`
3. All 36 `claim_evidence` rows → `link_claim_evidence`
4. All 84 `finding_evidence` rows → `link_finding_evidence`

The replay worker drains the queue at ~80 ops/min; ~1500 new ops
finishes in ~15 minutes on VM 410. Idempotent (all v1 ops are MERGE).

## Known gaps (unchanged from phase 1)

- **Finding→Entity :ABOUT** has no PG source. The v2 compiler writes
  the link to `graph_change_log` only (no `finding_entity` table), so
  the 83 PG findings have zero :ABOUT edges. Future writes will populate
  them via the worker; the pre-existing 0/83 is a one-time data loss
  in the v2 design, not a mirror problem. A follow-up would be a
  `findings.entities` table or a denormalized column.
- **Neo4j mirror is one-way**: deleting a PG row doesn't delete its
  Neo4j mirror (no delete op in graph_sync_queue). The current design
  is "additive only". Future: add a delete op reconcile worker if
  PG canonical rows get pruned.
- **No `:Claim` mirror of v2 create_claim**: v2's claim creation goes
  directly to PG without writing to `graph_change_log`. Phase 1 left
  this alone because v1's `create_claim` MCP tool does mirror to
  Neo4j. There may be v2-only claim creation paths (e.g. automatic
  claim extraction from documents) that don't reach Neo4j — out of
  scope here, but a follow-up would be to add a `(*, claim)` insert
  change_log row in v2's claim creation site.

## Files

- `hub-core/crates/hub-core/src/graphw.rs` — 6 new try_graph_write
  arms + 4 new translate_change_log arms + REL_TYPES extension +
  name_map covers merge uuids
- `scripts/backfill-neo4j-graph-mirror.py` — 4 new backfill sections
  (documents / findings / claim_evidence / finding_evidence)
- `scripts/accept-sp10.py` — 3 new assertions (32/33/34) for the
  claim_evidence / finding_evidence / documents mirror coverage

## Verification on VM 410

End-to-end inserts of one of each new op kind (contradiction,
link_evidence, mark_finding_about_entity, merge) all produced the
expected Neo4j nodes/edges within 20s of the worker tick. Full
backfill drained to DONE in ~15 min: 942 :Document, 83 :Finding,
36 Claim→Doc SUPPORTS, 84 Finding→Doc SUPPORTS edges, 1
:Contradiction + 2 :INVOLVES edges, 1 :MERGED_INTO edge (cleaned
after test). accept-sp10 34/34.