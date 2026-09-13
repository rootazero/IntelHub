# SP9 — Knowledge Graph Memory Layer (Core)

Date: 2026-09-13
Status: **DRAFT — user confirmation pending after this spec is read**
Branch: `feat/kg-memory-design` (decisions captured in `2026-09-13-intelhub-sp9-kg-memory-decisions.md`)
Depends on: SP2A (data plane), SP2B (write plane + graph_sync_queue), SP3 (console routes), SP5 (entity_seeder)

## 0. Background & motivation

Today the hub writes entities / claims / relationships into Postgres (canonical) and mirrors
them into Neo4j via `graph_sync_queue`. Neo4j is a *graph mirror*, not a *knowledge layer*:

- No temporal dimension (no valid_from / valid_until / discovered_at)
- Only 12 entity kinds, 10 relationship types (hard-coded allowlist in `graphw.rs:14-30`)
- No Document / Source / Investigation / Finding / Evidence as Neo4j labels — those are
  PG-only, so traversal can't reach them
- Aliases stored on `entities.aliases jsonb` but no read path ever matches against them
  (every read does `name ILIKE`)
- Contradiction only exists as `claims.status='contradicted'` — no explicit graph structure
- Existing graph tools (`query_entity`, `query_relationship`, `find_path`) are
  name-substring / exact-name only; no alias awareness, no temporal, no evidence join

The decisions doc (`…-decisions.md`) locks in 5 gate decisions (phasing, temporal model,
Neo4j label scope, resolution algorithm, UI skeleton scope). SP9 implements only the
*data + MCP* layer of those decisions. SP10/SP11/SP12 carry the UI and auto-write.

## 1. Architecture

```
hub-core (single binary, systemd, LLM-free per SP2B)
└── graph_v2/                     [NEW]
    ├── mod.rs                    typed-intent registry + result envelope
    ├── compiler.rs               intent → validated write plan (no raw Cypher)
    ├── temporal.rs               valid_time/system_time merge policies
    ├── resolve.rs                entity resolution (sync path)
    ├── resolve_async.rs          background entity_resolution_queue worker
    ├── contradiction.rs          CONTRADICTS-edge logic
    ├── evidence.rs               claim↔evidence binding helpers
    └── tests/                    per-intent unit tests

└── graph/queries.rs              [NEW] read-only Cypher templates used by MCP tools
                                  (extends existing graph.rs which stays v1)

└── api_v1/graph.rs               [NEW] REST: GET endpoints (5) for console
└── api_v1/mod.rs                 wire-in (mounts new router, alongside existing /api/v1/entities etc.)

└── mcp.rs                        add 10 new #[tool] entries (all L1/Free, read-only)
└── policy.rs                     register 10 new entries in policy_for() table

└── store.rs                      small additions: claim_contradictions + graph_change_log helpers
└── server.rs                     boot: ensure_neo4j_schema() + start resolve_async worker

└── migrations/0008_kg_memory.sql [NEW] PG schema additions (see §3)
└── scripts/neo4j_init_cypher.txt [NEW] idempotent constraints + indexes (read at boot)
```

**Hard rules carried from SP2B** (must NOT be broken):
- Hub stays LLM-free. No embedding calls in graph_v2.
- No raw Cypher from MCP tools. Every write intent goes through `compiler.rs` validation.
- All write operations still PG-canonical + `graph_sync_queue` enqueue + Neo4j MERGE.
- Existing v1 tools (`create_entity`/`create_claim`/`create_relationship`) unchanged in SP9.

**New hard rule introduced in SP9**:
- v2 compiler refuses any write intent that doesn't carry ≥1 `evidence_doc_ids` entry
  (unless intent is explicitly `evidence_absent_reason="no_public_record"`).
  Rationale: prevents "Agent says X → X becomes fact" without provenance (user §22).

## 2. Data model & flow

### Write flow

```
Agent / MCP client
   │
   │  typed intent JSON (see §4)
   ▼
MCP tool (mcp.rs::HubMcp)
   │  gate → policy::preflight (cost=L2/Write for asserts, L1/Free for reads)
   │  record → store::record_tool_call
   ▼
graph_v2::compiler::dispatch(intent)
   │  schema validation (allowlists: entity kinds, predicates, evidence shape)
   │  evidence existence check (every doc_id must exist in documents)
   │  temporal merge policy (temporal.rs)
   │  entity resolution (resolve.rs) if intent touches an entity by name
   ▼
PG canonical write
   ├── entities / relationships / claims / findings (insert/update with temporal cols)
   ├── entity_aliases (append-only)
   ├── claim_contradictions (append-only)
   └── graph_change_log (append-only audit row: op, before, after, by, at)
   │
   ▼
graph_sync_queue::enqueue(op, target)         // reuses existing queue + replay worker
   │
   ▼
Neo4j MERGE (existing graphw worker)
   │  APOC + new constraints/indexes (boot-time ensure)
   ▼
Neo4j mirror
```

### Read flow

```
Agent / Console
   │
   │  GET /mcp or GET /api/v1/graph/*
   ▼
MCP tool (mcp.rs::HubMcp)        OR       axum handler (api_v1/graph.rs)
   │  gate → policy::preflight (all reads L1/Free)
   │  record → store::record_tool_call
   ▼
graph::queries::<name>(args)              (Cypher templates; no agent-supplied Cypher)
   │
   ├── Neo4j (graph traversal, temporal predicates)
   └── PG JOINs (entity hydration: aliases, evidence docs, contradiction pairs)
   ▼
Result envelope (serde_json::Value, no internal types leak)
```

### Temporal model (§Q2)

| column | meaning | nullable | default |
|---|---|---|---|
| `valid_from timestamptz` | fact started being true in the world | NULL | NULL |
| `valid_until timestamptz` | fact stopped being true | NULL | NULL |
| `discovered_at timestamptz` | when hub first observed this fact | NOT NULL | `now()` |

Predicates:
- "true at time T" iff `(valid_from IS NULL OR valid_from ≤ T) AND (valid_until IS NULL OR valid_until > T)`
- "currently true" iff `valid_until IS NULL`
- "expired" iff `valid_until IS NOT NULL AND valid_until ≤ now()`
- Apply to: `entities` (entity existence window), `relationships` (PG + Neo4j mirror).

### Entity resolution flow (§Q4)

```
sync path (called from compiler on every intent that names an entity):
  resolve_entity(name, kind, source, confidence) → entity_id
  ├─ exact match on (kind, normalize(name))                  → return
  ├─ exact match on entity_aliases (kind, normalize(alias))   → return
  ├─ jaro_winkler ≥ threshold against all candidates of kind → return + write alias
  └─ no match                                                 → create new entity + self-alias

async path (background worker, every 5 min):
  drain 100 rows from entity_resolution_queue
  ├─ run JW batch, classify: auto-merge (≥0.95) / pending-review (0.85–0.95) / ignore
  ├─ auto-merge: update merged_into + write graph_change_log
  └─ pending-review: write to entity_review_queue (manual API)

strsim 0.11 (JW built-in) — already in tokio-friendly deps, no new native deps
HUB_KG_RESOLVE_THRESHOLD=0.92, HUB_KG_AUTO_MERGE_THRESHOLD=0.95 (env-overridable)
```

## 3. Schema additions (migration 0008_kg_memory.sql)

### 3.1 PG: extend existing tables

```sql
-- entities: temporal + resolution state
ALTER TABLE entities
  ADD COLUMN valid_from        timestamptz NULL,
  ADD COLUMN valid_until       timestamptz NULL,
  ADD COLUMN discovered_at     timestamptz NOT NULL DEFAULT now(),
  ADD COLUMN merged_into       uuid NULL REFERENCES entities(entity_id) ON DELETE SET NULL,
  ADD COLUMN resolution_method text NULL CHECK (resolution_method IN ('exact','alias','jaro_winkler','manual','seed')),
  ADD COLUMN resolution_confidence real NULL CHECK (resolution_confidence BETWEEN 0 AND 1);

-- relationships: temporal + evidence binding + provenance
ALTER TABLE relationships
  ADD COLUMN valid_from        timestamptz NULL,
  ADD COLUMN valid_until       timestamptz NULL,
  ADD COLUMN discovered_at     timestamptz NOT NULL DEFAULT now(),
  ADD COLUMN confidence        real NULL CHECK (confidence BETWEEN 0 AND 1),
  ADD COLUMN evidence_doc_ids  uuid[] NULL,
  ADD COLUMN source_ids        int[] NULL,
  ADD COLUMN created_by_task_id uuid NULL REFERENCES tasks(task_id) ON DELETE SET NULL;
```

Indexes:
```sql
CREATE INDEX entities_kind_name_norm     ON entities (kind, lower(name));
CREATE INDEX entities_merged_into        ON entities (merged_into) WHERE merged_into IS NOT NULL;
CREATE INDEX relationships_valid_from    ON relationships (valid_from) WHERE valid_from IS NOT NULL;
CREATE INDEX relationships_valid_until   ON relationships (valid_until) WHERE valid_until IS NOT NULL;
CREATE INDEX relationships_evidence_gin  ON relationships USING gin (evidence_doc_ids);
CREATE INDEX relationships_created_by    ON relationships (created_by_task_id) WHERE created_by_task_id IS NOT NULL;
```

### 3.2 PG: new tables

```sql
-- entity_aliases: normalized alias index for resolution (replaces jsonb for lookup)
CREATE TABLE entity_aliases (
  alias_id     bigserial PRIMARY KEY,
  entity_id    uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  alias        text NOT NULL,
  alias_norm   text NOT NULL,           -- normalize_name() applied
  kind         text NOT NULL,           -- duplicated for index locality
  source       text NOT NULL,           -- 'seed'|'manual'|'jw_match'|'agent_assert'
  confidence   real NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (kind, alias_norm)
);
CREATE INDEX entity_aliases_entity ON entity_aliases (entity_id);

-- claim_contradictions: explicit graph edge between claims
CREATE TABLE claim_contradictions (
  contradiction_id   bigserial PRIMARY KEY,
  claim_a            uuid NOT NULL REFERENCES claims(claim_id) ON DELETE CASCADE,
  claim_b            uuid NOT NULL REFERENCES claims(claim_id) ON DELETE CASCADE,
  reason             text NOT NULL,
  raised_by_task_id  uuid NULL REFERENCES tasks(task_id) ON DELETE SET NULL,
  raised_by_agent    text NULL,
  raised_at          timestamptz NOT NULL DEFAULT now(),
  resolved           boolean NOT NULL DEFAULT false,
  resolution_note    text NULL,
  CHECK (claim_a <> claim_b)
);
CREATE INDEX claim_contradictions_a ON claim_contradictions (claim_a);
CREATE INDEX claim_contradictions_b ON claim_contradictions (claim_b);

-- graph_change_log: append-only audit trail of graph mutations
CREATE TABLE graph_change_log (
  change_id    bigserial PRIMARY KEY,
  op           text NOT NULL CHECK (op IN ('insert','update','merge','contradict','temporal_close')),
  target_kind  text NOT NULL CHECK (target_kind IN ('entity','relationship','claim','contradiction')),
  target_id    text NOT NULL,           -- entity_id (uuid), rel id (uuid), claim_id (uuid), contradiction_id (bigint::text)
  before       jsonb NULL,
  after        jsonb NOT NULL,
  changed_by   text NOT NULL,           -- agent_id or 'system' or 'resolve_async'
  task_id      uuid NULL REFERENCES tasks(task_id) ON DELETE SET NULL,
  changed_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX graph_change_log_target ON graph_change_log (target_kind, target_id, changed_at DESC);
CREATE INDEX graph_change_log_at      ON graph_change_log (changed_at DESC);

-- entity_resolution_queue: async low-confidence merge candidates
CREATE TABLE entity_resolution_queue (
  queue_id      bigserial PRIMARY KEY,
  candidate_a   uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  candidate_b   uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  score         real NOT NULL,
  reason        text NOT NULL,          -- 'low_conf_alias'|'jw_below_threshold'|'manual_flag'
  status        text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','merged','rejected','deferred')),
  created_at    timestamptz NOT NULL DEFAULT now(),
  resolved_at   timestamptz NULL,
  CHECK (candidate_a <> candidate_b)
);
CREATE INDEX entity_resolution_queue_pending ON entity_resolution_queue (status, created_at) WHERE status = 'pending';

-- entity_review_queue: ambiguous merges flagged for human review
CREATE TABLE entity_review_queue (
  review_id     bigserial PRIMARY KEY,
  entity_a      uuid NOT NULL REFERENCES entities(entity_id),
  entity_b      uuid NOT NULL REFERENCES entities(entity_id),
  proposed_score real NOT NULL,
  reason        text NOT NULL,
  status        text NOT NULL DEFAULT 'open' CHECK (status IN ('open','approved','rejected')),
  reviewer_note text NULL,
  reviewed_at   timestamptz NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
```

### 3.3 Neo4j: new constraints + indexes (idempotent, run at boot)

`scripts/neo4j_init_cypher.txt` — read by `state.rs::ensure_neo4j_schema()` once at startup:

```cypher
// Constraints (uniqueness for v2 IDs)
CREATE CONSTRAINT entity_id IF NOT EXISTS
  FOR (e:Entity) REQUIRE e.entity_id IS UNIQUE;
CREATE CONSTRAINT claim_id IF NOT EXISTS
  FOR (c:Claim) REQUIRE c.claim_id IS UNIQUE;
CREATE CONSTRAINT investigation_id IF NOT EXISTS
  FOR (i:Investigation) REQUIRE i.investigation_id IS UNIQUE;
CREATE CONSTRAINT finding_id IF NOT EXISTS
  FOR (f:Finding) REQUIRE f.finding_id IS UNIQUE;
CREATE CONSTRAINT document_fk IF NOT EXISTS
  FOR (d:Document) REQUIRE d.document_id IS UNIQUE;

// Indexes for v2 traversal patterns
CREATE INDEX entity_aliases_idx IF NOT EXISTS
  FOR (e:Entity) ON (e.aliases);
CREATE INDEX relationship_valid_from IF NOT EXISTS
  FOR ()-[r:RELATIONSHIP]-() ON (r.valid_from);
CREATE INDEX relationship_valid_until IF NOT EXISTS
  FOR ()-[r:RELATIONSHIP]-() ON (r.valid_until);
CREATE INDEX finding_about IF NOT EXISTS
  FOR (f:Finding) ON (f.investigation_id);
CREATE INDEX investigation_status IF NOT EXISTS
  FOR (i:Investigation) ON (i.status);
```

Neo4j label additions (no DDL needed — labels are created on first write):
- `:Document` (thin — `document_id` + optional `kind` only)
- `:Investigation` (`investigation_id`, `status`, `title`)
- `:Finding` (`finding_id`, `investigation_id`, `claim_text`)
- `:Evidence` (thin — `evidence_id` only; payload via PG)

Neo4j relationship type additions:
- `:SUPPORTS` (Claim → Evidence)
- `:CONTRADICTS` (Claim → Claim, mirrored from `claim_contradictions`)
- `:ABOUT` (Finding → Entity) — *new edge type, supersedes the old v1 use*
- `:MENTIONS` (Document → Entity) — read-only, populated by v2 from observations
- `:PRODUCED` (Investigation → Finding)
- `:PART_OF` (Finding → Investigation) — same data, inverse direction for traversal
- `:REPLACES` (Claim → Claim, temporal succession; populated when v2 closes a temporal window)

Allowlist for typed-intent writes (in `graph_v2/compiler.rs`):
- Entity kinds: v1 set (12) + `Person`, `Organization`, `GovernmentEntity`, `Company`,
  `Asset`, `Aircraft`, `Vessel`, `Source` (raw web source), `ManualEntity`
- Predicates: v1 set (10) + `SUPPORTS`, `CONTRADICTS`, `ABOUT`, `MENTIONS`,
  `PRODUCED`, `PART_OF`, `REPLACES`

(v1 allowlist in `graphw.rs:14-30` stays untouched. v2 allowlist is a strict superset.)

## 4. Typed intents (graph compiler v2)

Each intent is a JSON object with a discriminator `action` field. Compiler dispatches
by `action`, validates the payload against a per-action JSON Schema, and emits 0..N
PG writes + 1 graph_sync_queue op. No raw Cypher in MCP tool code.

| Intent | Required fields | Notes |
|---|---|---|
| `assert_entity` | `kind`, `name`, `aliases[]?`, `attributes{}?`, `evidence_doc_ids[uuid]≥1`, `valid_from?`, `valid_until?` | Calls `resolve_entity` for dedupe; if match, returns existing entity_id |
| `assert_relationship` | `subject (entity_id or {kind,name})`, `predicate`, `object (...)`, `evidence_doc_ids[uuid]≥1`, `confidence`, `valid_from?`, `valid_until?`, `discovered_at?` | Temporal merge: same (s,p,o) check overlap; if overlap → update + graph_change_log; if non-overlap → REPLACES edge in Neo4j |
| `assert_contradiction` | `claim_a`, `claim_b`, `reason`, `evidence_doc_ids[uuid]≥1` | Mirrors to Neo4j `:CONTRADICTS` edge; sets `claims.status='disputed'` on both |
| `link_evidence_to_claim` | `claim_id`, `document_id`, `relation ('supports'\|'contradicts')`, `snippet?` | Mirrors to Neo4j `:SUPPORTS` or `:CONTRADICTS` between Claim and Evidence |
| `mark_finding_about_entity` | `finding_id`, `entity_id`, `confidence` | Adds `:ABOUT` edge in Neo4j; no PG write (FK via `finding_evidence` already covers PG) |
| `close_investigation_extract` | `investigation_id`, `extraction[]` (list of `assert_entity`/`assert_relationship` payloads) | Bulk helper; SP12 wires this to auto-trigger on `investigations.status='closed'` |

Validation rules (in `compiler.rs`):
1. Every intent must carry ≥1 `evidence_doc_ids` (or explicit `evidence_absent_reason`)
2. `predicate` ∈ v2 allowlist (see §3.3)
3. `kind` ∈ v2 allowlist
4. Subject/object entity FKs must exist after `resolve_entity` resolves them
5. `valid_from < valid_until` if both present
6. `confidence ∈ [0,1]` if present
7. References to `claim_id` / `document_id` / `finding_id` / `investigation_id` must exist
8. Caller must supply `actor` (agent_id or 'system'); recorded in `graph_change_log.changed_by`

Backpressure: if Neo4j is DOWN for >5 min, `graph_sync_queue` accumulates. Existing
replay worker (started in `server.rs`) handles drain. SP9 adds: `graph_change_log`
writes are PG-only, so even if Neo4j mirror lags, the change log is intact.

## 5. Entity Resolution

`graph_v2/resolve.rs`:

```rust
pub fn resolve_entity(
    name: &str, kind: &str, source: &str, confidence: f64,
    pool: &PgPool,
) -> Result<EntityResolution, HubError>;

pub enum EntityResolution {
    Existing { entity_id: Uuid, matched_by: ResolutionMethod, score: f64 },
    New      { entity_id: Uuid },     // freshly created
}
```

Sync logic (§Q4): exact → alias → JW (strsim::jaro_winkler). Threshold via
`HUB_KG_RESOLVE_THRESHOLD` env (default 0.92).

Async worker (`graph_v2/resolve_async.rs`):
- Started in `server.rs` as a `tokio::spawn` after PG ready
- Ticks every 5 min; drains up to 100 pending rows from `entity_resolution_queue`
- Classifies into auto-merge / pending-review / ignore
- Writes audit entries to `graph_change_log`

Manual override API (MCP, L2/Write):
- `merge_entities(a, b, kept_id)` — manual merge, sets `merged_into`, writes change log
- `reject_merge(review_id, note)` — closes a `entity_review_queue` row as rejected

## 6. New MCP tools (all L1/Free, read-only)

| # | Tool | Args | Backed by | Notes |
|---|---|---|---|---|
| 1 | `search_entity` | `name`, `kind?`, `limit?` | PG `entities` + `entity_aliases` + JW fallback | alias-aware; returns score per match |
| 2 | `get_entity` | `entity_id` | PG JOINs | returns entity + relationships + claims + investigations + findings + aliases + merged_into chain |
| 3 | `get_entity_timeline` | `entity_id`, `from?`, `to?`, `limit?` | PG `graph_change_log` WHERE target matches | ordered DESC by changed_at |
| 4 | `get_neighbors` | `entity_id`, `depth?` (default 2), `rel_types?`, `min_confidence?`, `at_time?` | Neo4j MATCH with temporal predicate | returns subgraph nodes + edges |
| 5 | `find_path` | `from`, `to`, `weighted?` (default true), `max_hops?` (default 5) | Neo4j shortestPath / APOC | weighted = `1 - confidence`. **Replaces v1 `find_path`** (no back-compat needed: v1 was never independently tested in accept-sp2a/b; v1 implementation deleted in same commit) |
| 6 | `find_relationship_changes` | `a`, `b`, `from?`, `to?` | PG `relationships` WHERE temporal overlap | returns diff list |
| 7 | `find_supporting_claims` | `entity_id`, `limit?` | PG JOINs (claims ↔ finding_evidence ↔ findings_about_entity) | |
| 8 | `find_contradicting_claims` | `entity_id or claim_id` | PG `claim_contradictions` | |
| 9 | `query_investigation_graph` | `investigation_id` | Neo4j MATCH (:Investigation)-[*]-(…) | returns subgraph of findings, claims, entities |
| 10 | `list_evidence_for_entity` | `entity_id`, `relation?` | PG observations JOIN documents | replaces unused `observations` table; populates it as a side effect |

Existing v1 tools `query_entity` and `query_relationship` stay — they are exercised by
`accept-sp2a.py:159-162` and `accept-sp2b.py:146-148`. v1 `find_path` is **replaced**
in SP9 (no accept script depends on it; v1 implementation deleted from `graph.rs` in
same commit as the v2 implementation lands in `graph/queries.rs`). The v2 `find_path`
preserves the v1 arg shape (`from`, `to`, `limit`) and adds `weighted` + `max_hops`,
so semantic back-compat holds for any caller that was using the bare name match.

REST additions (`/api/v1/graph/*`) for console:
- `GET /api/v1/graph/neighbors?root=<id>&depth=2&at_time=...`
- `GET /api/v1/graph/entity/<id>/timeline?from=...&to=...`
- `GET /api/v1/graph/path?from=<id>&to=<id>`
- `GET /api/v1/graph/investigation/<id>`
- `GET /api/v1/graph/evidence?entity=<id>&relation=supports`

These power SP10's full UI and SP9's minimal /graph skeleton (§7).

## 7. Minimal /graph UI skeleton

Console additions:
- `console/package.json`: add `"@antv/g6": "^5.0.x"` (pin to latest 5.x at install time)
- `console/src/pages/Graph.tsx`: new minimal page — empty canvas + 1 default query
  (`GET /api/v1/graph/neighbors?root=<seed>`) + zoom/pan only
- `console/src/api/graph.ts`: typed wrappers for the 5 REST endpoints
- `console/src/App.tsx`: register `/graph` route + nav entry (next to Investigations)

Deferred to SP10: filter panel, side panel, drilldown, investigation subgraph view,
live SSE refresh, contrast theme tokens.

## 8. Configuration

`core/hub.env` additions:
```
HUB_KG_RESOLVE_THRESHOLD=0.92
HUB_KG_AUTO_MERGE_THRESHOLD=0.95
HUB_KG_CHANGE_LOG_RETENTION_DAYS=90
HUB_KG_FIND_PATH_MAX_HOPS=5
HUB_KG_NEIGHBORS_DEFAULT_DEPTH=2
HUB_KG_RESOLVE_ASYNC_TICK_SECS=300
```

All have sensible defaults; none require restart of dependent services.

`scripts/health-check.sh` extension: assert new tables exist, assert new MCP tools
advertised (`tools/list`), assert Neo4j constraints present.

## 9. Validation

### Unit tests (per module)
- `graph_v2::compiler::assert_relationship` — happy path, missing evidence rejected,
  invalid predicate rejected, temporal overlap detected
- `graph_v2::resolve` — exact / alias / JW / new (4 cases)
- `graph_v2::temporal::merge_policy` — overlap, sequential, concurrent
- `graph::queries::get_neighbors` — temporal predicate, depth, rel_type filter
- `graph::queries::find_path` — unweighted + weighted, max_hops honored

### `scripts/accept-sp9.py` (new)

1. Migration 0008 applied; new columns + 5 new tables exist
2. Neo4j 4 new constraints + 5 new indexes present (`SHOW CONSTRAINTS`/`SHOW INDEXES`)
3. `resolve_entity("OpenAI Inc.","org","seed",0.95)` returns existing entity (alias path)
4. `resolve_entity("Apple Computer","org","seed",0.95)` returns existing "Apple Inc." (JW path)
5. Two `:SUPPORTS` edges on a Claim resolve to 2 evidence docs via claim_id FK
6. `:CONTRADICTS` edge between Claim A and Claim B created via `assert_contradiction`
   sets `claim_contradictions` row + Neo4j edge + both claims' status to `disputed`
7. `find_relationship_changes(A, B, t1, t2)` returns the temporal diff
8. `get_entity_timeline` returns ≥1 row for an entity with `discovered_at` set
9. Console `/graph` route loads (200 OK); default neighbors query returns ≥1 node
10. Back-compat: v1 `create_entity` / `create_relationship` / `create_claim` tools still
    pass their previous acceptance assertions
11. SP2A 19 · SP2B 33 · SP3 19 · SP4 25 · SP5 9 · SP6 18 · SP7 24 · SP8 18 — all green
12. `hub_monitor_events_total` and other existing health metrics unchanged
13. MCP `tools/list` advertises all old + 10 new tools; tool count increased by 10
14. Neo4j mirror lag < 30 s under smoke test (50 mixed write intents, drain < 30 s)

### Regression bar
All existing accept scripts unchanged. New `accept-sp9.py` adds 14 assertions.

## 10. Out of scope (explicit)

- LLM-based entity extraction (SP2B zero-LLM rule stands)
- Embedding-based entity resolution (deferred indefinitely; needs design discussion)
- Full Graph Workspace UI with filters/panels (SP10)
- Knowledge Timeline UI (SP11)
- Investigation → Graph auto-write pipeline (SP12)
- Replacing v1 `create_entity` / `create_relationship` / `create_claim` (back-compat; SP12 deprecates). v1 `find_path` IS replaced in SP9 (no accept-test dependency)
- Cross-investigation contradiction discovery (heuristic; manual via `find_contradicting_claims` for SP9)
- Cleaning up the unused `observations` table (it gets a write path in SP9 via
  `list_evidence_for_entity`, but historical backfill not done)

## 11. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Entity resolution creates spurious merges (e.g. "Apple Inc" software ↔ "Apple" fruit) | Threshold 0.92 default + auto-merge only ≥0.95 + `entity_review_queue` for ambiguous + full audit in `graph_change_log` |
| Bi-temporal model confuses agents (when should `valid_from` be set?) | compiler rejects write with `valid_from` in the future; future use of valid_from requires explicit JSON field — defaults stay NULL |
| Neo4j constraints collision on startup (cold start race) | `ensure_neo4j_schema()` is idempotent (IF NOT EXISTS); retry once on transient errors |
| Schema migration 0008 fails on existing prod data (NOT NULL DEFAULT now()) | `discovered_at NOT NULL DEFAULT now()` is safe (fills in for existing rows); temporal columns are NULLABLE |
| Migration adds nullable columns with index → bloat on large tables | `CREATE INDEX CONCURRENTLY` for relationships (likely the largest); 0008 marked CONCURRENTLY-safe |
| `@antv/g6` bundle size or SSR issues | Skeleton in §7 only loads G6 on /graph route (lazy); pin to 5.x; if too heavy, SP10 swaps to Sigma/cytoscape |
| `graph_change_log` grows unbounded | `HUB_KG_CHANGE_LOG_RETENTION_DAYS=90`; daily vacuum task in scheduler |
| v2 typed-write tooling tempts agents to auto-write without validation | Hard rule §1: every intent must carry evidence_doc_ids; compiler rejects otherwise |
| Existing v1 tools still being used (no migration pressure) | v1 paths unaffected; SP12 introduces deprecation warnings |

## 12. Amendment (filled at deploy)

Deployment record (versions, deviations, timing) added after first green accept.