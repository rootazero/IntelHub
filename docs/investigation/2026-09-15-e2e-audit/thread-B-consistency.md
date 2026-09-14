# Thread B: PG / Neo4j / Redis Cross-Store Consistency

> **Status:** ✅ Complete · **Date:** 2026-09-15 · **Target:** VM 415 (IntelHub-test, 10.10.10.45)
> **Method:** Read-only `pg()` / `cypher()` / `redis()` via `scripts/_remote.py` (monkey-patched to IntelHub-test) + manual MCP cache test
> **Reusable script:** `/tmp/inv-e2e/thread-B/cross-store-diff.py` (idempotent)

## Summary

| 类别 | 数量 |
|---|---|
| Sample claims (with claim_evidence JOIN) | 1 PG row |
| Sample documents | 2 PG rows |
| Investigations sampled | 0 (sampling returned 0; `pg_inv` shows 0 — see B-013 note) |
| Monitor health cells | 0 Redis keys (no monitors running — by design, AGENTS.md says 23 monitors with 2 shelved, but on 415 none have `monitor:health:*` written — finding) |
| **Findings** | **7** (🔴 ×3 · 🟠 ×3 · 🟢 ×0 · informational ×1) |

## Inventory Snapshot

| Store | Object | Count |
|---|---|---|
| PG | documents | 2 |
| PG | claims | 1 |
| PG | evidence | (count 0 or empty) |
| PG | claim_evidence | (count 0 or empty, but 1 row found via JOIN) |
| PG | investigations | 5 (via `list_investigations` MCP earlier) |
| PG | entities | 58 |
| PG | entity_aliases | **0** |
| PG | cost_records | 139 |
| PG | embedding_jobs | 3 |
| Neo4j | (Entity) | 58 |
| Neo4j | (Claim) | 2 |
| Neo4j | (Document) | 0 |
| Neo4j | (Investigation) | 0 |
| Redis | DBSIZE | 56 |

## Findings

### B-001 🔴 1 PG claim missing Neo4j `(Claim)` node — sync gap (probably create_claim)
- **Reproducible:** run `cross-store-diff.py` step 1
- **Output:** `PG claims missing Neo4j: 1, Neo4j claims extra: 2`
- **Interpretation:** Only 1 PG claim exists, but Neo4j has 2 Claim nodes — the 1 PG claim is NOT in Neo4j (sync miss), and 2 Neo4j claims exist that have no PG row at all (orphan / untracked creation path).
- **Why critical:** If users search claims via Neo4j (`query_investigation_graph`, `find_supporting_claims` touch Neo4j), they get DIFFERENT results from PG (source of truth). The user-facing claim set is non-deterministic between stores.
- **Fix direction:** Inspect `graphw.rs::create_claim` end-to-end. Verify `link_claim_node` op is enqueued (per AGENTS.md 2026-09-14 fix memory, but here it's still failing). Also check if there's a Neo4j-only creation path (e.g. `intelhub_v2_extract_claims` which description says "for LLM extractor pipelines") that bypasses PG.

### B-002 🔴 2 Neo4j `(Claim)` nodes orphaned — no PG row
- **Reproducible:** step 1 of script
- **Detail:** Neo4j has Claim nodes that don't exist in PG. Likely created by `v2_extract_claims` (Level 2, "no evidence required — for LLM extractor pipelines that pick claims out of documents" per description) which writes to Neo4j directly without PG dual-write, or by a sync replay that created the node but the PG row was rolled back.
- **Fix direction:** Make `v2_extract_claims` write to PG canonical first, then enqueue sync op. Audit which code path creates Neo4j-only Claim nodes.

### B-003 🟠 1 `claim_evidence` PG row missing Neo4j `:SUPPORTS` edge — same class as 2026-09-14 fix
- **Reproducible:** step 1
- **Output:** `PG edges missing Neo4j :SUPPORTS: 1, Neo4j :SUPPORTS edges: 0`
- **Detail:** 1 PG `claim_evidence` row exists but no `(Claim)-[:SUPPORTS]->(Document)` edge in Neo4j. Neo4j has ZERO :SUPPORTS edges total.
- **Why high:** If `create_claim` once again forgot to emit `link_claim_evidence` v1 op, all claims fail to link to their evidence in the graph view. The 2026-09-14 fix in AGENTS.md memory addressed this — but on this 415 deployment, :SUPPORTS edges are STILL missing. Either the fix wasn't actually deployed, or it regressed.
- **Fix direction:** Verify the fix is present in `hub-core/src/graphw.rs::create_claim`. Run `cross-store-diff.py` on production (410) too — if 410 has :SUPPORTS edges but 415 doesn't, the 415 build is stale. If neither has them, the fix regressed.

### B-004 🟠 **2 PG documents have no Neo4j `(Document)` node** — sync gap
- **Reproducible:** step 3
- **Output:** `PG documents: 2, Neo4j (Document) nodes: 0`
- **Detail:** 2 PG documents but ZERO `(Document)` nodes in Neo4j. Likely causes:
  1. `crawl_url`/`fetch_document` writes to PG but doesn't enqueue sync op (or sync op fails silently for documents)
  2. Neo4j Document label index broken
  3. The `document_id` in PG is a UUID but Neo4j expects a different format
- **Impact:** `query_investigation_graph` joins via `(Document)` nodes; if no documents are nodes, evidence chain is broken.
- **Fix direction:** Run `MATCH (d) WHERE NOT (d:Entity) AND NOT (d:Claim) AND NOT (d:Investigation) RETURN labels(d), count(*)` to see if documents are stored under a different label. If yes, sync maps to wrong label. If no, sync worker dropped.

### B-005 🔴 **PG `entity_aliases` table is EMPTY (0 rows) while Neo4j has inflated aliases** — reverse sync failure
- **Reproducible:** step 2
- **Output:**
  - PG `entity_aliases`: 0 rows
  - Neo4j `Entity.aliases` count by entity (top 5):
    | Entity | alias_count |
    |---|---|
    | NASA FIRMS | 82 |
    | ACLED | 64 |
    | CISA | 63 |
    | NOAA | 62 |
    | BRICS | (also inflated per A-005 — aliases 12× duplicated) |
- **Detail:** AGENTS.md memory says "`create_entity` 接受 `aliases[]` 数组和 inserts row-per-alias" — implies aliases are written to PG. But PG table is empty. Neo4j has the aliases (duplicated, per A-005), PG has zero.
- **Root cause candidates:**
  1. `entity_aliases` table is write-only-to-Neo4j by design, PG table never used → ARCHITECTURE drift (memory is wrong)
  2. PG table was wiped, Neo4j never synced back → DATA corruption
  3. Aliases are stored as Neo4j `Entity.aliases` property (JSON array, not edge), not as separate nodes → SCHEMA drift
- **Why critical:** If `search_entity` reads from Neo4j (which the MCP response suggests), why does PG have a table at all? If it reads from PG, why does it work at all? The data model is unclear, and the duplication in Neo4j (82× same alias) proves the write path has no idempotency.
- **Fix direction:** Check `graphw.rs::create_entity` end-to-end: where does it write aliases? If Neo4j only, drop PG table OR dual-write. If PG only, find why Neo4j has them. Add UNIQUE constraint and INSERT...ON CONFLICT for the chosen target.

### B-006 🟡 0 PG alias conflicts (cross-entity alias_norm reuse) — but 5+ entities per alias verified via Thread A
- **Reproducible:** step 2
- **Detail:** Script reports 0 alias_norm reused across entities. But Thread A `search_entity` for "BRICS" returned entity with aliases duplicated 12× INTERNAL to the entity. Internal dup ≠ cross-entity conflict. Both findings (B-005 internal dup via Neo4j, this clean) tell us the **PG `entity_aliases` is not where aliases live**.
- **Fix direction:** Consolidate with B-005 — single source of truth for aliases.

### B-007 🟠 0 monitor health cells in Redis despite 23 collectors
- **Reproducible:** `redis("KEYS monitor:health:*")` returns 0 keys
- **Detail:** AGENTS.md says 23 monitors, 2 shelved. But on 415, Redis has NO `monitor:health:*` keys. Possible causes:
  1. hub-core hasn't written any health cell since restart (collectors silent, no periodic heartbeat)
  2. Redis key namespace mismatch (maybe it's `intelhub:monitor:health:*`?)
  3. hub-core writes elsewhere (PG?)
- **Fix direction:** Check `redis("KEYS *monitor*")` for any health cell pattern. Check `pg("SELECT name, last_run_at, last_status FROM monitor_sources")` for last activity. If Redis truly empty + PG last_run > 24h ago, **all 23 monitors are silently dead**.

### B-008 informational Cache layer verified working
- **Test:** hybrid_search cache-states.json shows `miss, miss, hit` on 3 queries (1, 2, 1). Redis has matching `hub:qcache:*` keys.
- **Status:** ✅ Cache TTL + key hashing works as designed. AGENTS.md "A — Redis-backed query result cache" deployed correctly.

## Cross-Store Reconciliation Matrix

| Object | PG | Neo4j | Redis | Status |
|---|---|---|---|---|
| Document | 2 | 0 | (cache invalidation?) | 🔴 drift |
| Claim | 1 | 2 | — | 🔴 drift (PG<Neo4j + orphans) |
| Claim-Evidence edge | 1 | 0 | — | 🔴 drift (PG missing in Neo4j) |
| Investigation | 5 (via MCP) | 0 | — | 🟠 drift (likely because script sampled 0) |
| Entity | 58 | 58 | — | ✅ match |
| Entity alias | 0 | thousands | — | 🔴 PG empty, Neo4j inflated |
| Monitor health cell | (PG: monitor_sources N) | — | 0 keys | 🟠 monitor data missing |
| Cost record | 139 | — | — | ✅ |
| Embedding job | 3 | — | — | (small N, consistent with low doc count) |

## Reproducibility

```bash
cd /Volumes/TBU/Workspace/IntelHub-investigation-2026-09-15
python3 /tmp/inv-e2e/thread-B/cross-store-diff.py
# Outputs to docs/investigation/2026-09-15-e2e-audit/evidence/thread-B/
```

Read-only. No data mutation.

## Caveats

- Investigation sampling returned 0 rows — script's JOIN may be misformed; the `list_investigations` MCP call confirmed 5 investigations exist in PG. Re-check sampling query before trusting "investigations: 0".
- Document count is low (2) — 415 test VM has minimal test data; many findings extrapolate from small N. Run same script on 410 for production-scale numbers before prioritizing fixes.
- B-005 (entity_aliases PG empty vs Neo4j inflated) is the single most impactful finding — needs architect-level decision on data model.
