# SP9 Knowledge Graph Memory — Decisions (Draft, Pending Lock-in)

Date: 2026-09-13
Branch: `feat/kg-memory-design`
Status: **DRAFT — awaiting user review/confirmation before SP9 spec is written**
Source: brainstorming Q&A in chat; recommendations marked `[推荐]` reflect my proposal,
user can override any before this doc is merged into the SP9 design.

---

## 0. Scope of this doc

This file captures the 5 gate decisions from the brainstorming session that pre-determine
SP9's architecture. Once confirmed, these flow into the full SP9 design doc
(`2026-09-13-intelhub-sp9-kg-memory-design.md`) without further Q&A. Code work begins
**only after** the full SP9 design is approved per the brainstorming HARD-GATE.

---

## Q1. Phasing — how many SPs?

**Proposal `[推荐]`**: Accept 4-SP phasing as proposed. Each SP has independent acceptance
and can ship alone.

| SP | Scope | Deliverable | UI? |
|---|---|---|---|
| **SP9** KG Memory Core | PG migration 0008 + Neo4j ontology expansion + graph compiler v2 + entity resolution + 10 MCP read tools + minimal `/graph` skeleton | data plane | minimal G6 canvas only |
| **SP10** Graph Workspace UI | AntV G6 filters/panels/full interaction; EntityDetail upgrade; Investigation subgraph view | UI plane | full |
| **SP11** Knowledge Timeline | Gantt-style timeline per entity/relationship; graph_change_log replay; investigation history | UI plane | full |
| **SP12** Investigation → Graph auto-write | Auto-extract closed-investigation findings into graph (typed intent, agent can reject) | pipeline | UI hook only |

**Rationale**: each SP independently shippable; AGENTS.md acceptance-per-SP rule preserved;
UI risk deferred to a dedicated SP where it doesn't block the data-plane upgrade.

**Alternatives rejected**:
- "Do SP9+10 together" — conflates data + UI; SP10 risk would gate SP9 acceptance
- "Skip SP12" — auto-write is a core value driver per brainstorming; deferred, not dropped
- "Combine SP10+11" — Timeline has independent data dependency (graph_change_log built in SP9)

---

## Q2. Temporal model

**Proposal `[推荐]`**: **A. Bi-temporal (valid_time + system_time)**

| column | meaning |
|---|---|
| `valid_from timestamptz` | when the fact started being true in the world (NULL = "we don't know when it started") |
| `valid_until timestamptz` | when it stopped being true (NULL = "still true / we don't know when it ended") |
| `discovered_at timestamptz` | when IntelHub first observed/learned the fact (NOT NULL default `now()`) |

**Rationale**:
- Matches Graphiti semantics — directly absorbable later
- Answers both "what was true at time T" AND "when did we know"
- OSINT distinction matters: a relationship might be valid for years but discovered last week
- Single index on `(valid_from, valid_until)` covers both temporal queries

**Alternatives rejected**:
- B (only valid_time) — loses discovery time, hurts "knowledge freshness" UX
- C (three columns valid_from/valid_until/discovered_at) — same as A; A is C with the proper
  semantic naming. (I had C as the original draft; **A is what I actually want.**)

**Scope**:
- Apply to: `entities` (entity existence), `relationships` (PG + Neo4j mirror)
- NOT apply to: `documents`, `observations`, `findings` — these already have sufficient
  observed_at / created_at semantics and would balloon the migration

---

## Q3. Document / Source / Investigation / Finding / Evidence as Neo4j labels

**Proposal `[推荐]`**: **B. Document + Source stay PG-only; Investigation + Finding are
Neo4j labels; Evidence is a Neo4j label too (light, no payload).**

| Node label | Where | Why |
|---|---|---|
| `Entity` | Neo4j + PG | already done; expand ontology |
| `Claim` | Neo4j + PG | already done |
| `Relationship` | Neo4j + PG (edge) | already done; add temporal props |
| `Source` | **PG only** | URL-reputation tuples, not graph-traversable |
| `Document` | **PG only** (Neo4j holds `:MENTIONS` to Entity but doc itself is FK ref) | Doc content is large; Neo4j shouldn't store it |
| `Investigation` | **Neo4j + PG** | investigations are first-class traversable objects |
| `Finding` | **Neo4j + PG** | findings are first-class; SP12 auto-writes them |
| `Evidence` | **Neo4j as thin label**, payload stays PG | `:SUPPORTS` / `:CONTRADICTS` edges reference evidence doc by FK |
| `Contradiction` | **Neo4j as thin label** | `(:Claim A)-[:CONTRADICTS]->(:Claim B)` with `reason` + `raised_by_task_id` |

**Rationale**:
- Avoid Neo4j-as-blob-store for documents (would duplicate 100GB+ of content)
- But Investigation/Finding are valuable traversal targets — "what did we learn about X?"
  is a 2-hop question (`Finding -[:ABOUT]-> Entity`)
- Evidence as thin Neo4j label lets `:SUPPORTS`/`:CONTRADICTS` be traversable without
  pulling PG payloads until needed (lazy hydration via FK)

**Neo4j constraints to add** (alongside existing):
- `CREATE CONSTRAINT entity_id IF NOT EXISTS FOR (e:Entity) REQUIRE e.entity_id IS UNIQUE`
- `CREATE CONSTRAINT claim_id IF NOT EXISTS FOR (c:Claim) REQUIRE c.claim_id IS UNIQUE`
- `CREATE CONSTRAINT investigation_id IF NOT EXISTS FOR (i:Investigation) REQUIRE i.investigation_id IS UNIQUE`
- `CREATE CONSTRAINT finding_id IF NOT EXISTS FOR (f:Finding) REQUIRE f.finding_id IS UNIQUE`
- `CREATE INDEX entity_alias IF NOT EXISTS FOR (e:Entity) ON (e.aliases)`
- `CREATE INDEX relationship_valid IF NOT EXISTS FOR ()-[r:REL]-() ON (r.valid_from)`

---

## Q4. Entity Resolution algorithm

**Proposal `[推荐]`**: **A. Jaro-Winkler ≥ 0.92 + alias table exact match**, no embedding.

**Sync path** (`resolve_entity(name, kind, source, confidence) -> entity_id`):
1. Exact match on `(kind, name_norm)` → return existing
2. Alias exact match on `(kind, alias_norm)` → return its `entity_id`
3. Jaro-Winkler ≥ 0.92 across all candidates of same kind (in-memory scan for now; ≤ 10k entities is fine)
4. If match found → write to `entity_aliases`, optionally set `merged_into` if high confidence
5. If no match → create new entity, seed self-alias

**Async path** (background):
- `entity_resolution_queue` table — low-confidence merges / unverified aliases queued
- Periodic sweep: every hour, take 100 items, run JW batched, write audit log
- Manual review API: `entity_review_queue` for ambiguous cases

**Matching library**: `strsim` Rust crate (Jaro-Winkler built-in, no deps).
**Threshold**: 0.92 default, configurable via `HUB_KG_RESOLVE_THRESHOLD`.

**Rationale**:
- Zero LLM (SP2B compliance preserved)
- Explainable (audit log shows which alias triggered which merge)
- Cheap (~µs per comparison for 10k candidates)
- Embedding-based deferred: would need Qdrant-side embedding for entity names, which is
  a separate decision and can be added without breaking existing data

**Alternatives rejected**:
- B (pg_trgm) — adds Postgres extension dependency for marginal gain; our entities are bounded in scale
- C (embedding-based) — violates SP2B zero-LLM hub principle; if needed, do client-side via MCP

---

## Q5. SP9 minimal `/graph` UI skeleton?

**Proposal `[推荐]`**: **A. Yes — minimal G6 canvas with data wiring, no full interaction.**

What's in the skeleton:
- Add `@antv/g6` to `console/package.json`
- New route `/graph` with empty canvas + 1 default query (`/api/v1/graph/neighbors?root=<sample_entity>`)
- Canvas renders nodes + edges (no filter panel, no side panel)
- Build wired; if g6 has bundle/perf issues, we find out now (not in SP10)

What's deferred to SP10:
- Filters (kind, rel_type, time, confidence, source)
- Side panel (entity/relationship properties)
- Click → drilldown
- Investigation subgraph view
- Live refresh via SSE

**Rationale**:
- Surfaces bundle/SSR/perf issues at small scope
- Lets console team (or you) eyeball "is this even the right shape" before committing to SP10

**Alternative rejected**:
- B (no skeleton) — discover g6 issues during SP10 work, harder to isolate

---

## Open / not-yet-decided

These are NOT gates but need to be settled during the SP9 spec write:

| # | Item | Lean |
|---|---|---|
| O1 | How to handle "merge confidence" — auto-merge at ≥0.95, mark-pending at 0.85–0.95, ignore below | auto ≥ 0.95, pending 0.85–0.95 |
| O2 | `graph_change_log` retention | 90 days default, configurable |
| O3 | New MCP tools: read-write or read-only? | per AGENTS.md spirit (no auto-write), **all 10 are read-only** in SP9 |
| O4 | `find_path` weighting: `1-confidence` or `1-log(confidence+ε)` | `1-confidence` for simplicity |
| O5 | `get_neighbors` default depth | 2 (matches typical 2-hop OSINT question) |
| O6 | Whether `create_entity`/`create_relationship`/`create_claim` (existing v1) stay as-is or are deprecated in favor of v2 typed intents | **stay as-is in SP9**; deprecation in SP12 |
| O7 | `valid_time` on Neo4j edge properties vs only PG relationships | **both**, kept in sync via graph_sync_queue |

---

## What's NOT in SP9 (explicit out-of-scope)

- LLM-based entity extraction (SP2B zero-LLM rule)
- Embedding-based entity resolution (deferred indefinitely)
- Full Graph Workspace UI (SP10)
- Knowledge Timeline UI (SP11)
- Investigation → Graph auto-write pipeline (SP12)
- Replacing existing `create_entity` / `create_relationship` / `create_claim` v1 tools (kept for back-compat)
- Any change to agent auto-write behavior (still agent-initiated via typed intents in SP9)
- Cross-investigation contradiction discovery (heuristic — deferred, manual via `find_contradicting_claims`)

---

## Acceptance preview for SP9 (to be refined in spec)

`scripts/accept-sp9.py` will assert (at minimum):

1. Migration 0008 applied; new columns present; entity_aliases / claim_contradictions / graph_change_log exist
2. Neo4j constraints + indexes created (verified via SHOW CONSTRAINTS / SHOW INDEXES)
3. `resolve_entity` dedupes "OpenAI Inc." → existing "OpenAI" entity (alias path)
4. `resolve_entity` dedupes "Apple Computer" → existing "Apple Inc." entity (JW ≥ 0.92 path)
5. Two `:SUPPORTS` edges on a claim resolve to 2 evidence docs via claim_id FK
6. `:CONTRADICTS` edge between Claim A and Claim B created via `assert_contradiction`
7. `find_relationship_changes(A, B, t1, t2)` returns the temporal diff
8. `get_entity_timeline` returns ≥1 row for an entity with `discovered_at` set
9. Console `/graph` route loads; default neighbors query returns ≥ 1 node
10. Back-compat: existing `create_entity` / `create_relationship` / `create_claim` v1 tools still pass their previous acceptance assertions
11. All previous SPs (sp2a 19 · sp2b 33 · sp3 19 · sp4 25 · sp5 9 · sp6 18 · sp7 24 · sp8 18) still green
12. `hub_monitor_events_total` and other existing health metrics unchanged
13. No regression in agent auto-discovery (MCP `tools/list` still advertises all old + new tools)