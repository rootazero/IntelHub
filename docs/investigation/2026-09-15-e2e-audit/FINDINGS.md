# 2026-09-15 E2E Investigation — FINDINGS

> **Status:** ✅ Investigation complete · **Date:** 2026-09-15 · **Target:** VM 415 (IntelHub-test, 10.10.10.45)
> **Method:** 3 parallel threads (MCP contract / cross-store consistency / console UI render)
> **Tools:** 22 MCP tools probed · 1 PG/Neo4j/Redis diff pass · 17 console pages probed

## Executive Summary

**Total findings: 17** (🔴 3 · 🟠 5 · 🟡 5 · 🟢 3 · informational 1)

**Top 3 most impactful:**

1. **B-005 + A-005 (data bug, user-facing)** — `entity_aliases` PG table is empty while Neo4j has massively inflated duplicates (NASA FIRMS 82×, ACLED 64×, CISA 63×, NOAA 62×, BRICS 12×). The aliases are stored ONLY in Neo4j with no dedup constraint, and the duplicates **render inline in the `/entities/:id` page** — user sees ugly repeated strings.
2. **B-001 + B-002 + B-003 + B-004 (PG/Neo4j sync gaps)** — 1 PG claim missing Neo4j node, 2 Neo4j claims orphan (no PG), 1 PG claim_evidence row with no `:SUPPORTS` edge, 2 PG documents with no Neo4j `(Document)`. Same class as the 2026-09-14 `create_claim` bug — sync emit is dropping or order-of-operations broken.
3. **A-003 (schema/doc contradiction)** — `find_contradicting_claims` schema `required=[]` but description says "Exactly one must be supplied". Clients trusting the schema will send empty args; clients sending both will be rejected. Schema self-contradicts.

## Findings by Severity

### 🔴 Critical (data loss / sync drift / user-facing data bug)

| ID | Title | Thread | Components | Status |
|---|---|---|---|---|
| **B-005 / A-005** | **Entity aliases: PG table empty, Neo4j duplicates 12-82×, user-visible** | A + B | `entity_aliases` PG, Neo4j mirror, `/entities/:id` UI | **OPEN** — fix plan ready |
| **B-001** | 1 PG claim missing Neo4j `(Claim)` node | B | create_claim → graph_sync_queue | OPEN |
| **B-002** | 2 Neo4j `(Claim)` nodes orphan (no PG row) | B | v2_extract_claims (?) → Neo4j direct | OPEN |
| **B-008** | `embedding_status=COMPLETE` but `embedding IS NULL` (would be reported) | B | documents table | (small N — not yet observed on 415; flag for 410) |

### 🟠 High (sync gaps + user-facing UX gap)

| ID | Title | Thread | Components | Status |
|---|---|---|---|---|
| **B-003** | PG `claim_evidence` row missing Neo4j `:SUPPORTS` edge | B | graphw.rs::create_claim | OPEN — 2026-09-14 fix regressed? |
| **B-004** | 2 PG documents missing Neo4j `(Document)` node | B | crawl_url → sync | OPEN |
| **A-003** | `find_contradicting_claims` schema required=[] contradicts description | A | MCP tool_schema | OPEN — fix plan ready |
| **C-001** | Duplicate aliases render inline in `/entities/:id` (user-facing) | C | entity detail page | Same fix as B-005 |
| **B-007** | 0 monitor health cells in Redis despite 23 collectors (silent dead?) | B | monitor heartbeat writer | OPEN — needs investigation |

### 🟡 Medium (data shape / API ergonomics)

| ID | Title | Thread | Components | Status |
|---|---|---|---|---|
| A-001 | `create_claim.text` vs `create_finding.claim_text` field name drift | A | MCP create_* family | OPEN |
| A-002 | `create_claim.evidence_document_ids` vs `create_finding.evidence` shape drift | A | MCP create_* family | OPEN |
| A-004 | `search_entity` returns bare array, all other list_* return `{count, items}` | A | search_entity | OPEN |
| A-007 | MCP tool name vs schema field name discoverability (9/22 probe 422s) | A | multiple tool schemas | OPEN — AGENTS.md docs |
| C-002 | `/documents/:id` deep link returns blank page (no route) | C | React Router | OPEN — easy fix |

### 🟢 Low (informational / positive findings)

| ID | Title | Thread | Components | Status |
|---|---|---|---|---|
| A-006 | `hybrid_search.mode` is free-form string, not enum | A | MCP | DOCUMENTED |
| A-007 | (same as medium — also informational about probe coverage) | A | — | — |
| C-003 | `/investigations/:id` shows red error banner for missing ID (GOOD UX) | C | Investigations page | ✓ no fix needed |
| B-008 | Cache layer works as designed (verified hybrid_search miss→hit) | B | cache.rs | ✓ no fix needed |

---

## Thread A: MCP / REST API Contract Inconsistencies (7 findings)

> Full detail: `thread-A-contract.md` · Evidence: `evidence/thread-A/`

| ID | Sev | Title |
|---|---|---|
| A-001 | 🟡 | `create_claim.text` vs `create_finding.claim_text` field name drift |
| A-002 | 🟡 | `create_claim.evidence_document_ids` vs `create_finding.evidence` evidence shape drift |
| **A-003** | 🟠 | `find_contradicting_claims` schema required=[] contradicts description |
| A-004 | 🟡 | `search_entity` returns bare array, other list_* return `{count, items}` |
| **A-005** | 🟠 | BRICS entity aliases duplicated 12× in Neo4j (no dedup constraint) — see B-005 |
| A-006 | 🟢 | `hybrid_search.mode` is free-form string, not enum |
| A-007 | 🟢 | MCP tool name vs schema field name discoverability (9/22 probe 422s) |

**Repro:** `python3 evidence/thread-A/probe-runner.py` · 41 schemas in `evidence/thread-A/schemas/` · 22 happy-path responses in `evidence/thread-A/responses/`

---

## Thread B: PG / Neo4j / Redis Cross-Store Consistency (8 findings)

> Full detail: `thread-B-consistency.md` · Reusable script: `scripts/audit/cross-store-diff.py` · Evidence: `evidence/thread-B/`

| ID | Sev | Title | Stores affected |
|---|---|---|---|
| **B-001** | 🔴 | 1 PG claim missing Neo4j `(Claim)` node | PG ↔ Neo4j |
| **B-002** | 🔴 | 2 Neo4j `(Claim)` nodes orphan (no PG row) | Neo4j ↔ PG |
| **B-003** | 🟠 | PG `claim_evidence` row missing Neo4j `:SUPPORTS` edge | PG ↔ Neo4j |
| **B-004** | 🟠 | 2 PG documents missing Neo4j `(Document)` node | PG ↔ Neo4j |
| **B-005** | 🔴 | PG `entity_aliases` empty, Neo4j aliases inflated 12-82× | PG empty ↔ Neo4j dup |
| B-006 | 🟡 | 0 alias_norm cross-entity conflicts (informational — internal dup is the issue) | PG only |
| **B-007** | 🟠 | 0 monitor health cells in Redis (all 23 monitors silent?) | Redis empty + PG monitor_sources stale? |
| B-008 | ✓ | hybrid_search cache layer verified working | Cache ✓ |

---

## Thread C: Console UI Render Failures (3 findings)

> Full detail: `thread-C-ui.md` · Probe script: `console/probe-all-pages.mjs` · Evidence: `evidence/thread-C/`

| ID | Sev | Title |
|---|---|---|
| **C-001** | 🟠 | Duplicate aliases render inline in `/entities/:id` (user-facing) — same root cause as B-005 |
| C-002 | 🟡 | `/documents/:id` deep link returns blank page (no route) |
| C-003 | 🟢 | `/investigations/:id` shows red error banner for missing ID (positive — no fix needed) |

---

## Cross-Cutting Patterns

1. **Sync path lacks idempotency** (A-005/B-005/B-001/B-002/B-003/B-004): The graph_sync_queue worker has no INSERT...ON CONFLICT semantics. Every retry or replay duplicates work. The 2026-09-14 fix in AGENTS.md memory mentioned `create_claim` 漏 emit — but the pattern persists across `create_claim`, `crawl_url`, `entity_seeder`. **Single fix candidate:** re-architect sync emit to be idempotent per-op-type.

2. **Schema ↔ code ↔ doc drift** (A-001/A-002/A-003/A-004/A-007): Three adjacent create_* tools have diverged in field names and evidence shape. The `find_contradicting_claims` description is more correct than its schema. `search_entity` is the only list_* tool returning bare array. **Single fix candidate:** automated schema-vs-description consistency lint (CI hook that compares `tools/list` schemas to published MCP docs).

3. **Console ↔ API data quality** (C-001 + B-005): The same bug appears at 3 layers — Neo4j data, MCP API response, console UI render. The user sees the consequence (ugly duplicates on `/entities/:id`); they don't see the root cause (sync emit on `entity_seeder` duplicates aliases). **Single fix candidate:** dedup migration + UNIQUE constraint + idempotent write (one fix, all 3 layers benefit).

## Reproducibility

```bash
# Re-run Thread A
python3 /Volumes/TBU/Workspace/IntelHub-investigation-2026-09-15/docs/investigation/2026-09-15-e2e-audit/evidence/thread-A/probe-runner.py

# Re-run Thread B  
python3 /tmp/inv-e2e/thread-B/cross-store-diff.py  # after exporting KEY and routing to IntelHub-test

# Re-run Thread C
cd /Volumes/TBU/Workspace/IntelHub-investigation-2026-09-15/console
node probe-all-pages.mjs
```

All three are read-only. Thread A produces 41 schema dumps + 22 response dumps. Thread B produces 1 cross-store diff. Thread C produces 17 PNG screenshots + 1 probe-output.json.

## Fix Plans

Two fix plans are written for the highest-impact findings:

- `docs/superpowers/plans/2026-09-15-entity-aliases-dedup.md` — covers A-005 / B-005 / C-001 (the user-facing data bug)
- `docs/superpowers/plans/2026-09-15-find-contradicting-claims-xor.md` — covers A-003 (schema self-contradiction)

Other findings (B-001/B-002/B-003/B-004 sync gaps) bundled into a single "sync emit idempotency" follow-up recommended in plan.
