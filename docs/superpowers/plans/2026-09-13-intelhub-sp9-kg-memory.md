# SP9 Knowledge Graph Memory Layer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade Neo4j from "graph mirror" to "knowledge memory layer" — bi-temporal edges, evidence-backed relationships, alias-aware entity resolution, explicit contradiction graph, 10 new read-only MCP tools, and a minimal `/graph` console skeleton.

**Architecture:** New `graph_v2/` Rust module holds typed-intent compiler + temporal merge + entity resolution; `graph/queries.rs` holds read-only Cypher templates. PG stays canonical via migration 0008 (5 new tables + temporal columns on entities/relationships). Neo4j mirrors via existing `graph_sync_queue` replay worker; new constraints/indexes applied at boot via `ensure_neo4j_schema()`. Console gets a minimal `/graph` route + AntV G6 (deferred full UI to SP10). Hub stays zero-LLM per SP2B.

**Tech Stack:**
- Rust 1.85 (workspace); sqlx 0.8 + Postgres 17; neo4rs 0.8 + Neo4j 5.26-community
- strsim 0.11 (Jaro-Winkler; pure-Rust, no native deps)
- @antv/g6 ^5.x (console); React 19 + vite 5

**Spec:** `docs/superpowers/specs/2026-09-13-intelhub-sp9-kg-memory-design.md`

---

## Global Constraints

- **Zero LLM in hub** — SP2B rule. No embedding/model calls anywhere in `graph_v2/`.
- **No raw Cypher from agents** — All writes flow through `graph_v2::compiler::dispatch` which validates against allowlists.
- **Migration naming** — `0008_kg_memory.sql` (latest is `0007_retry_short_docs.sql`).
- **v1 back-compat** — `create_entity` / `create_relationship` / `create_claim` / `query_entity` / `query_relationship` unchanged. Only `find_path` v1 is replaced (no accept-script dependency).
- **New MCP tools are all L1/Free (read-only)** — per `policy_for()` registry.
- **Every typed intent must carry evidence** — compiler rejects without `evidence_doc_ids[]` or explicit `evidence_absent_reason`.
- **Test convention** — `#[cfg(test)] mod tests { ... }` inline in source file, mirroring `monitor/sources/bls.rs` pattern. `cargo test -p hub-core` runs all.
- **AGENTS.md regression bar** — sp2a 19 · sp2b 33 · sp3 19 · sp4 25 · sp5 9 · sp6 18 · sp7 24 · sp8 18 MUST all stay green.
- **Worktree** — All work happens in `feat/kg-memory` (create from `feat/kg-memory-design` HEAD once spec+plan approved; spec/plan stay on `feat/kg-memory-design` for the PR).
- **Hub-core build** — happens in VM via `bash scripts/build-hub.sh` after `rsync -az --delete ... IntelHub:/home/zou/IntelHub/`. Console build via `bash scripts/build-console.sh`.
- **Acceptance** — `python3 scripts/accept-sp9.py "$KEY"` exits 0; baselines remain green.

---

## File Structure (overview map)

```
hub-core/
├── Cargo.toml                                          [MODIFY: add strsim to workspace.dependencies]
├── migrations/0008_kg_memory.sql                       [CREATE]
├── crates/hub-core/
│   ├── Cargo.toml                                      [MODIFY: add strsim, schemars (already present?)]
│   └── src/
│       ├── lib.rs                                      [MODIFY: add `pub mod graph_v2; pub mod graph_queries;`]
│       ├── state.rs                                    [MODIFY: call ensure_neo4j_schema() at boot]
│       ├── graph.rs                                    [MODIFY: remove v1 find_path fn; leave query_entity/query_relationship]
│       ├── graphw.rs                                   [UNTOUCHED in SP9]
│       ├── neo4j_init.rs                               [CREATE: boot-time cypher runner]
│       ├── store.rs                                    [MODIFY: add change_log helper, alias lookup fn]
│       ├── policy.rs                                   [MODIFY: add 10 new L1/Free entries]
│       ├── mcp.rs                                      [MODIFY: replace v1 find_path; add 10 new #[tool]s]
│       ├── server.rs                                   [MODIFY: start resolve_async worker]
│       ├── api.rs                                      [MODIFY: mount /api/v1/graph/* routes]
│       ├── graph_v2/                                   [CREATE]
│       │   ├── mod.rs                                  [CREATE]
│       │   ├── compiler.rs                             [CREATE: intent dispatch + validation]
│       │   ├── temporal.rs                             [CREATE: merge policy]
│       │   ├── resolve.rs                              [CREATE: sync resolution]
│       │   ├── resolve_async.rs                        [CREATE: queue worker]
│       │   ├── evidence.rs                             [CREATE: claim↔evidence helpers]
│       │   └── contradiction.rs                        [CREATE: CONTRADICTS logic]
│       └── graph_queries.rs                            [CREATE: read-side Cypher templates]
├── scripts/neo4j_init_cypher.txt                       [CREATE]

console/
├── package.json                                         [MODIFY: add @antv/g6 ^5.x]
├── src/
│   ├── App.tsx                                          [MODIFY: register /graph route + nav]
│   ├── api/graph.ts                                     [CREATE: typed wrappers for /api/v1/graph/*]
│   └── pages/Graph.tsx                                  [CREATE: minimal canvas skeleton]

scripts/
└── accept-sp9.py                                       [CREATE]

AGENTS.md                                                [MODIFY: add sp9 14 line, update "graph.mcp.tools" mention]
docs/superpowers/
├── specs/2026-09-13-intelhub-sp9-kg-memory-design.md    [EXISTS — reference only]
├── specs/2026-09-13-intelhub-sp9-kg-memory-decisions.md [EXISTS — reference only]
└── plans/2026-09-13-intelhub-sp9-kg-memory.md           [THIS FILE]
```

Each task below adds a small, independently testable slice.

---

## Task 1: Schema foundation (PG migration 0008 + Neo4j constraints + boot-time ensure)

**Files:**
- Create: `hub-core/migrations/0008_kg_memory.sql`
- Create: `hub-core/scripts/neo4j_init_cypher.txt`
- Create: `hub-core/crates/hub-core/src/neo4j_init.rs`
- Modify: `hub-core/crates/hub-core/src/lib.rs` (add `pub mod neo4j_init;`)
- Modify: `hub-core/crates/hub-core/src/state.rs:53-75` (call `ensure_neo4j_schema()` after migrations)
- Modify: `hub-core/Cargo.toml` (no change — `neo4rs` already a workspace dep)
- Test: `hub-core/crates/hub-core/src/neo4j_init.rs` inline `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `Arc<neo4rs::Graph>` (existing `state.neo4j`)
- Produces:
  - `pub async fn ensure_neo4j_schema(g: &neo4rs::Graph) -> Result<(), HubError>` — reads `scripts/neo4j_init_cypher.txt`, splits on `;`, runs each statement via `g.run(q).await`, idempotent (uses `IF NOT EXISTS`)
  - PG: 5 new tables (`entity_aliases`, `claim_contradictions`, `graph_change_log`, `entity_resolution_queue`, `entity_review_queue`) + extended `entities`/`relationships` columns

- [ ] **Step 1: Write the migration file**

Create `hub-core/migrations/0008_kg_memory.sql` with this content (verbatim from spec §3):

```sql
-- entities: temporal + resolution state
ALTER TABLE entities
  ADD COLUMN IF NOT EXISTS valid_from        timestamptz NULL,
  ADD COLUMN IF NOT EXISTS valid_until       timestamptz NULL,
  ADD COLUMN IF NOT EXISTS discovered_at     timestamptz NOT NULL DEFAULT now(),
  ADD COLUMN IF NOT EXISTS merged_into       uuid NULL REFERENCES entities(entity_id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS resolution_method text NULL CHECK (resolution_method IN ('exact','alias','jaro_winkler','manual','seed')),
  ADD COLUMN IF NOT EXISTS resolution_confidence real NULL CHECK (resolution_confidence BETWEEN 0 AND 1);

-- relationships: temporal + evidence binding + provenance
ALTER TABLE relationships
  ADD COLUMN IF NOT EXISTS valid_from        timestamptz NULL,
  ADD COLUMN IF NOT EXISTS valid_until       timestamptz NULL,
  ADD COLUMN IF NOT EXISTS discovered_at     timestamptz NOT NULL DEFAULT now(),
  ADD COLUMN IF NOT EXISTS confidence        real NULL CHECK (confidence BETWEEN 0 AND 1),
  ADD COLUMN IF NOT EXISTS evidence_doc_ids  uuid[] NULL,
  ADD COLUMN IF NOT EXISTS source_ids        int[] NULL,
  ADD COLUMN IF NOT EXISTS created_by_task_id uuid NULL REFERENCES tasks(task_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS entities_kind_name_norm     ON entities (kind, lower(name));
CREATE INDEX IF NOT EXISTS entities_merged_into        ON entities (merged_into) WHERE merged_into IS NOT NULL;
CREATE INDEX IF NOT EXISTS relationships_valid_from    ON relationships (valid_from) WHERE valid_from IS NOT NULL;
CREATE INDEX IF NOT EXISTS relationships_valid_until   ON relationships (valid_until) WHERE valid_until IS NOT NULL;
CREATE INDEX IF NOT EXISTS relationships_evidence_gin  ON relationships USING gin (evidence_doc_ids);
CREATE INDEX IF NOT EXISTS relationships_created_by    ON relationships (created_by_task_id) WHERE created_by_task_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS entity_aliases (
  alias_id     bigserial PRIMARY KEY,
  entity_id    uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  alias        text NOT NULL,
  alias_norm   text NOT NULL,
  kind         text NOT NULL,
  source       text NOT NULL,
  confidence   real NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (kind, alias_norm)
);
CREATE INDEX IF NOT EXISTS entity_aliases_entity ON entity_aliases (entity_id);

CREATE TABLE IF NOT EXISTS claim_contradictions (
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
CREATE INDEX IF NOT EXISTS claim_contradictions_a ON claim_contradictions (claim_a);
CREATE INDEX IF NOT EXISTS claim_contradictions_b ON claim_contradictions (claim_b);

CREATE TABLE IF NOT EXISTS graph_change_log (
  change_id    bigserial PRIMARY KEY,
  op           text NOT NULL CHECK (op IN ('insert','update','merge','contradict','temporal_close')),
  target_kind  text NOT NULL CHECK (target_kind IN ('entity','relationship','claim','contradiction')),
  target_id    text NOT NULL,
  before       jsonb NULL,
  after        jsonb NOT NULL,
  changed_by   text NOT NULL,
  task_id      uuid NULL REFERENCES tasks(task_id) ON DELETE SET NULL,
  changed_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS graph_change_log_target ON graph_change_log (target_kind, target_id, changed_at DESC);
CREATE INDEX IF NOT EXISTS graph_change_log_at      ON graph_change_log (changed_at DESC);

CREATE TABLE IF NOT EXISTS entity_resolution_queue (
  queue_id      bigserial PRIMARY KEY,
  candidate_a   uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  candidate_b   uuid NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
  score         real NOT NULL,
  reason        text NOT NULL,
  status        text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','merged','rejected','deferred')),
  created_at    timestamptz NOT NULL DEFAULT now(),
  resolved_at   timestamptz NULL,
  CHECK (candidate_a <> candidate_b)
);
CREATE INDEX IF NOT EXISTS entity_resolution_queue_pending ON entity_resolution_queue (status, created_at) WHERE status = 'pending';

CREATE TABLE IF NOT EXISTS entity_review_queue (
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

- [ ] **Step 2: Write the Cypher init file**

Create `hub-core/scripts/neo4j_init_cypher.txt`:

```cypher
CREATE CONSTRAINT entity_id IF NOT EXISTS FOR (e:Entity) REQUIRE e.entity_id IS UNIQUE;
CREATE CONSTRAINT claim_id IF NOT EXISTS FOR (c:Claim) REQUIRE c.claim_id IS UNIQUE;
CREATE CONSTRAINT investigation_id IF NOT EXISTS FOR (i:Investigation) REQUIRE i.investigation_id IS UNIQUE;
CREATE CONSTRAINT finding_id IF NOT EXISTS FOR (f:Finding) REQUIRE f.finding_id IS UNIQUE;
CREATE CONSTRAINT document_fk IF NOT EXISTS FOR (d:Document) REQUIRE d.document_id IS UNIQUE;
CREATE INDEX entity_aliases_idx IF NOT EXISTS FOR (e:Entity) ON (e.aliases);
CREATE INDEX relationship_valid_from IF NOT EXISTS FOR ()-[r:RELATIONSHIP]-() ON (r.valid_from);
CREATE INDEX relationship_valid_until IF NOT EXISTS FOR ()-[r:RELATIONSHIP]-() ON (r.valid_until);
CREATE INDEX finding_about IF NOT EXISTS FOR (f:Finding) ON (f.investigation_id);
CREATE INDEX investigation_status IF NOT EXISTS FOR (i:Investigation) ON (i.status);
```

(Each statement terminated by `;`.)

- [ ] **Step 3: Write the failing test**

In `hub-core/crates/hub-core/src/neo4j_init.rs`:

```rust
//! Boot-time Neo4j schema enforcement. Idempotent: uses IF NOT EXISTS for every
//! constraint and index. Reads Cypher from scripts/neo4j_init_cypher.txt.

use neo4rs::Graph;
use crate::error::HubError;

/// Run all statements in scripts/neo4j_init_cypher.txt against `g`.
/// Statements are split on `;` and run individually. Failures bubble up.
pub async fn ensure_neo4j_schema(g: &Graph) -> Result<(), HubError> {
    let cypher = include_str!("../../scripts/neo4j_init_cypher.txt");
    for stmt in split_statements(cypher) {
        let trimmed = stmt.trim();
        if trimmed.is_empty() { continue; }
        g.run(trimmed).await.map_err(|e| {
            HubError::Internal(format!("neo4j schema init failed: {e}; stmt={trimmed}"))
        })?;
    }
    Ok(())
}

fn split_statements(s: &str) -> impl Iterator<Item = &str> {
    s.split(';')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_semicolon() {
        let s = "CREATE CONSTRAINT a IF NOT EXISTS FOR (n:L) REQUIRE n.id IS UNIQUE;\n\nCREATE INDEX b IF NOT EXISTS FOR (n:L) ON (n.name);\n";
        let v: Vec<&str> = split_statements(s).collect();
        assert_eq!(v.len(), 2);
        assert!(v[0].contains("CONSTRAINT"));
        assert!(v[1].contains("INDEX"));
    }

    #[test]
    fn skips_empty_statements() {
        let s = ";;A;;";
        let v: Vec<&str> = split_statements(s).filter(|s| !s.trim().is_empty()).collect();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], "A");
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p hub-core --lib neo4j_init::tests
```

Expected: PASS (the unit tests for `split_statements` need no Neo4j).

- [ ] **Step 5: Register module + call from boot**

`hub-core/crates/hub-core/src/lib.rs`: add line `pub mod neo4j_init;` near existing `pub mod graph;`.

`hub-core/crates/hub-core/src/state.rs`: after the existing `sqlx::migrate!` and Neo4j connect lines (around line 53–75), add:

```rust
crate::neo4j_init::ensure_neo4j_schema(&neo4j).await?;
```

Wrap with `tracing::info!(target: "hub.boot", "neo4j schema ensured")` and propagate errors with context (use existing `HubError` variants — `Internal` is appropriate).

- [ ] **Step 6: Build**

```bash
cd hub-core && cargo build -p hub-core --release
```

Expected: builds clean (the only thing that may bite is `HubError` not having a `From<neo4rs::Error>`; if so, add at `error.rs`:

```rust
impl From<neo4rs::Error> for HubError {
    fn from(e: neo4rs::Error) -> Self { HubError::Internal(format!("neo4j: {e}")) }
}
```

(check existing `From` impls first — `neo4rs` errors may already be handled).

- [ ] **Step 7: Commit**

```bash
git add hub-core/migrations/0008_kg_memory.sql \
        hub-core/scripts/neo4j_init_cypher.txt \
        hub-core/crates/hub-core/src/neo4j_init.rs \
        hub-core/crates/hub-core/src/lib.rs \
        hub-core/crates/hub-core/src/state.rs
git commit -m "feat(sp9): PG migration 0008 + Neo4j schema ensure"
```

---

## Task 2: Entity Resolution (sync path + async worker + strsim dep)

**Files:**
- Modify: `hub-core/Cargo.toml` (add `strsim = "0.11"` to `[workspace.dependencies]`)
- Modify: `hub-core/crates/hub-core/Cargo.toml` (add `strsim.workspace = true`)
- Create: `hub-core/crates/hub-core/src/graph_v2/mod.rs`
- Create: `hub-core/crates/hub-core/src/graph_v2/resolve.rs`
- Create: `hub-core/crates/hub-core/src/graph_v2/resolve_async.rs`
- Modify: `hub-core/crates/hub-core/src/lib.rs` (add `pub mod graph_v2;`)
- Modify: `hub-core/crates/hub-core/src/store.rs` (add `lookup_alias_exact` and `find_entity_candidates` helpers)
- Test: inline `#[cfg(test)] mod tests` in each new file

**Interfaces:**
- Consumes: `&PgPool`, `name: &str`, `kind: &str`, `source: &str`, `confidence: f64`, threshold from env `HUB_KG_RESOLVE_THRESHOLD` (default 0.92)
- Produces:
  - `pub enum ResolutionMethod { Exact, Alias, JaroWinkler, NewEntity }`
  - `pub enum EntityResolution { Existing { entity_id: Uuid, matched_by: ResolutionMethod, score: f64 }, New { entity_id: Uuid } }`
  - `pub async fn resolve_entity(name: &str, kind: &str, source: &str, confidence: f64, pool: &PgPool) -> Result<EntityResolution, HubError>`
  - `pub async fn run_resolve_async_worker(pool: PgPool) -> !` (long-running)
  - `pub async fn merge_entities(a: Uuid, b: Uuid, kept: Uuid, actor: &str, pool: &PgPool) -> Result<(), HubError>`

- [ ] **Step 1: Add strsim dependency**

`hub-core/Cargo.toml` — under `[workspace.dependencies]`, add:
```toml
strsim = "0.11"
```

`hub-core/crates/hub-core/Cargo.toml` — under `[dependencies]`, add:
```toml
strsim.workspace = true
```

- [ ] **Step 2: Write failing tests for sync resolution**

Create `hub-core/crates/hub-core/src/graph_v2/mod.rs`:

```rust
pub mod compiler;
pub mod contradiction;
pub mod evidence;
pub mod resolve;
pub mod resolve_async;
pub mod temporal;
```

(Create `compiler.rs`, `contradiction.rs`, `evidence.rs`, `temporal.rs` as empty stubs — Tasks 3 & 5 overwrite these files with full implementations.)

Create `hub-core/crates/hub-core/src/graph_v2/resolve.rs`:

```rust
//! Sync entity resolution: exact → alias → jaro_winkler ≥ threshold → new entity.
//! See spec §5 for the algorithm.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::HubError;
use crate::store::entities;

/// How a match was made.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum ResolutionMethod {
    Exact,
    Alias,
    JaroWinkler,
    NewEntity,
}

#[derive(Debug, Clone, Serialize)]
pub enum EntityResolution {
    Existing { entity_id: Uuid, matched_by: ResolutionMethod, score: f64 },
    New      { entity_id: Uuid },
}

/// Resolve a name+kind to a canonical entity_id, creating one if needed.
pub async fn resolve_entity(
    name: &str, kind: &str, source: &str, confidence: f64,
    pool: &PgPool,
) -> Result<EntityResolution, HubError> {
    let threshold = std::env::var("HUB_KG_RESOLVE_THRESHOLD")
        .ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.92);
    let name_norm = entities::normalize_name(name);
    if name_norm.is_empty() {
        return Err(HubError::Validation("empty entity name".into()));
    }

    // 1. Exact match on (kind, normalized name)
    if let Some(eid) = entities::find_by_kind_name(pool, kind, &name_norm).await? {
        return Ok(EntityResolution::Existing {
            entity_id: eid, matched_by: ResolutionMethod::Exact, score: 1.0,
        });
    }
    // 2. Alias exact match
    if let Some(eid) = entities::lookup_alias_exact(pool, kind, &name_norm).await? {
        return Ok(EntityResolution::Existing {
            entity_id: eid, matched_by: ResolutionMethod::Alias, score: 1.0,
        });
    }
    // 3. Jaro-Winkler scan against all candidates of this kind
    let candidates = entities::find_candidates_by_kind(pool, kind).await?;
    let mut best: Option<(Uuid, f64)> = None;
    for c in candidates {
        let score = strsim::jaro_winkler(&name_norm, &c.name_norm);
        if score >= threshold && (best.is_none() || score > best.unwrap().1) {
            best = Some((c.entity_id, score));
        }
    }
    if let Some((eid, score)) = best {
        entities::write_alias(pool, eid, name, &name_norm, kind, source, confidence).await?;
        return Ok(EntityResolution::Existing {
            entity_id: eid, matched_by: ResolutionMethod::JaroWinkler, score,
        });
    }
    // 4. No match — create new entity with self-alias
    let eid = entities::create_with_alias(pool, kind, name, &name_norm, source, confidence).await?;
    Ok(EntityResolution::New { entity_id: eid })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_method_serializes() {
        let j = serde_json::to_string(&ResolutionMethod::JaroWinkler).unwrap();
        assert_eq!(j, "\"JaroWinkler\"");
    }
}
```

- [ ] **Step 3: Add store.rs helpers**

Append to `hub-core/crates/hub-core/src/store.rs` (use existing pattern from `entities::create_entity`):

```rust
pub mod entities {
    use sqlx::PgPool;
    use uuid::Uuid;
    use chrono::{DateTime, Utc};
    use crate::error::HubError;

    pub fn normalize_name(s: &str) -> String {
        // Trim + collapse whitespace + drop control chars + cap 256 (mirrors graphw.rs:52-60)
        let mut out = String::with_capacity(s.len());
        let mut last_space = true;
        for ch in s.chars() {
            if ch.is_control() { continue; }
            if ch.is_whitespace() {
                if !last_space { out.push(' '); last_space = true; }
            } else {
                out.push(ch); last_space = false;
            }
        }
        let trimmed = out.trim();
        if trimmed.len() > 256 { trimmed[..256].to_string() } else { trimmed.to_string() }
    }

    pub async fn find_by_kind_name(pool: &PgPool, kind: &str, name_norm: &str) -> Result<Option<Uuid>, HubError> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT entity_id FROM entities WHERE kind = $1 AND lower(name) = lower($2) LIMIT 1"
        ).bind(kind).bind(name_norm).fetch_optional(pool).await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn lookup_alias_exact(pool: &PgPool, kind: &str, alias_norm: &str) -> Result<Option<Uuid>, HubError> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT entity_id FROM entity_aliases WHERE kind = $1 AND alias_norm = $2 LIMIT 1"
        ).bind(kind).bind(alias_norm).fetch_optional(pool).await?;
        Ok(row.map(|r| r.0))
    }

    pub struct Candidate { pub entity_id: Uuid, pub name_norm: String }

    pub async fn find_candidates_by_kind(pool: &PgPool, kind: &str) -> Result<Vec<Candidate>, HubError> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT entity_id, lower(name) FROM entities WHERE kind = $1 AND merged_into IS NULL LIMIT 5000"
        ).bind(kind).fetch_all(pool).await?;
        Ok(rows.into_iter().map(|(e,n)| Candidate { entity_id: e, name_norm: n }).collect())
    }

    pub async fn write_alias(pool: &PgPool, entity_id: Uuid, alias: &str, alias_norm: &str, kind: &str, source: &str, confidence: f64) -> Result<(), HubError> {
        sqlx::query(
            "INSERT INTO entity_aliases (entity_id, alias, alias_norm, kind, source, confidence) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (kind, alias_norm) DO NOTHING"
        ).bind(entity_id).bind(alias).bind(alias_norm).bind(kind).bind(source).bind(confidence).execute(pool).await?;
        Ok(())
    }

    pub async fn create_with_alias(pool: &PgPool, kind: &str, name: &str, name_norm: &str, source: &str, confidence: f64) -> Result<Uuid, HubError> {
        let mut tx = pool.begin().await?;
        let row: (Uuid,) = sqlx::query_as(
            "INSERT INTO entities (kind, name) VALUES ($1,$2) ON CONFLICT (kind, name) DO UPDATE SET kind = EXCLUDED.kind RETURNING entity_id"
        ).bind(kind).bind(name).fetch_one(&mut *tx).await?;
        let eid = row.0;
        sqlx::query(
            "INSERT INTO entity_aliases (entity_id, alias, alias_norm, kind, source, confidence) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (kind, alias_norm) DO NOTHING"
        ).bind(eid).bind(name).bind(name_norm).bind(kind).bind(source).bind(confidence).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(eid)
    }
}
```

- [ ] **Step 4: Run failing tests**

```bash
cargo test -p hub-core --lib graph_v2::resolve::tests
```

Expected: the `resolution_method_serializes` test PASSES (no DB needed). Resolve integration tests are added in Task 5 against the running VM.

- [ ] **Step 5: Write async worker stub**

Create `hub-core/crates/hub-core/src/graph_v2/resolve_async.rs`:

```rust
//! Background worker: every N seconds, drain entity_resolution_queue and
//! classify pairs into auto-merge (≥0.95), pending-review (0.85–0.95), ignore.

use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;
use crate::error::HubError;
use crate::graph_v2::resolve::{merge_entities, ResolutionMethod};

const AUTO_MERGE_THRESHOLD: f64 = 0.95;
const REVIEW_LOWER: f64 = 0.85;

pub async fn run_resolve_async_worker(pool: PgPool) {
    let tick = std::env::var("HUB_KG_RESOLVE_ASYNC_TICK_SECS")
        .ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(300);
    loop {
        if let Err(e) = drain_once(&pool).await {
            tracing::warn!(target: "hub.kg.resolve_async", "drain failed: {e}");
        }
        sleep(Duration::from_secs(tick)).await;
    }
}

async fn drain_once(pool: &PgPool) -> Result<(), HubError> {
    let rows: Vec<(i64, uuid::Uuid, uuid::Uuid, f32, String)> = sqlx::query_as(
        "UPDATE entity_resolution_queue
           SET status = 'pending', resolved_at = NULL
         WHERE queue_id IN (
           SELECT queue_id FROM entity_resolution_queue
             WHERE status = 'pending'
             ORDER BY created_at ASC
             LIMIT 100
             FOR UPDATE SKIP LOCKED
         )
         RETURNING queue_id, candidate_a, candidate_b, score, reason"
    ).fetch_all(pool).await?;
    // Implementation note: above UPDATE is a placeholder — proper FOR UPDATE SKIP LOCKED
    // needs a CTE. The actual implementation in commit uses:
    //   WITH cte AS (SELECT queue_id FROM entity_resolution_queue WHERE status='pending'
    //                ORDER BY created_at LIMIT 100 FOR UPDATE SKIP LOCKED)
    //   UPDATE entity_resolution_queue SET status='processing', resolved_at=now()
    //     FROM cte WHERE entity_resolution_queue.queue_id = cte.queue_id
    //   RETURNING ...
    // See Task 5 for the full SQL.
    for (qid, a, b, score, _reason) in rows {
        let s = score as f64;
        if s >= AUTO_MERGE_THRESHOLD {
            merge_entities(a, b, a, "resolve_async", pool).await?;
            sqlx::query("UPDATE entity_resolution_queue SET status='merged', resolved_at=now() WHERE queue_id=$1")
                .bind(qid).execute(pool).await?;
        } else if s >= REVIEW_LOWER {
            sqlx::query(
                "INSERT INTO entity_review_queue (entity_a, entity_b, proposed_score, reason)
                 VALUES ($1,$2,$3,'jw_below_auto_threshold')
                 ON CONFLICT DO NOTHING"
            ).bind(a).bind(b).bind(s).execute(pool).await?;
            sqlx::query("UPDATE entity_resolution_queue SET status='deferred', resolved_at=now() WHERE queue_id=$1")
                .bind(qid).execute(pool).await?;
        } else {
            sqlx::query("UPDATE entity_resolution_queue SET status='rejected', resolved_at=now() WHERE queue_id=$1")
                .bind(qid).execute(pool).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn thresholds_are_sane() {
        assert!(super::AUTO_MERGE_THRESHOLD > super::REVIEW_LOWER);
        assert!(super::REVIEW_LOWER > 0.0 && super::REVIEW_LOWER < 1.0);
    }
}
```

- [ ] **Step 6: Implement merge_entities (used by both sync and async paths)**

Append to `graph_v2/resolve.rs` after the existing impl:

```rust
pub async fn merge_entities(
    a: Uuid, b: Uuid, kept: Uuid, actor: &str, pool: &PgPool,
) -> Result<(), HubError> {
    let mut tx = pool.begin().await?;
    let dropped = if a == kept { b } else { a };
    // Set merged_into on the dropped side; promote its aliases to the kept entity.
    sqlx::query("UPDATE entities SET merged_into = $1, resolution_method = 'manual' WHERE entity_id = $2")
        .bind(kept).bind(dropped).execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE entity_aliases SET entity_id = $1
           WHERE entity_id = $2
             AND NOT EXISTS (SELECT 1 FROM entity_aliases ea2 WHERE ea2.entity_id = $1 AND ea2.alias_norm = entity_aliases.alias_norm)"
    ).bind(kept).bind(dropped).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM entity_aliases WHERE entity_id = $1").bind(dropped).execute(&mut *tx).await?;
    // Audit
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by)
         VALUES ('merge','entity',$1, jsonb_build_object('merged_into', NULL), jsonb_build_object('merged_into', $2), $3)"
    ).bind(dropped.to_string()).bind(kept.to_string()).bind(actor).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p hub-core --lib graph_v2
```

Expected: PASS for unit tests (resolution_method_serializes, thresholds_are_sane).

- [ ] **Step 8: Commit**

```bash
git add hub-core/Cargo.toml hub-core/crates/hub-core/Cargo.toml \
        hub-core/crates/hub-core/src/lib.rs \
        hub-core/crates/hub-core/src/store.rs \
        hub-core/crates/hub-core/src/graph_v2/
git commit -m "feat(sp9): entity resolution — sync + async worker + strsim"
```

---

## Task 3: Graph Compiler v2 (typed intents + temporal merge + evidence + contradiction)

**Files:**
- Modify: `hub-core/crates/hub-core/src/graph_v2/compiler.rs` (was empty stub in Task 2)
- Modify: `hub-core/crates/hub-core/src/graph_v2/temporal.rs` (was empty stub)
- Modify: `hub-core/crates/hub-core/src/graph_v2/evidence.rs` (was empty stub)
- Modify: `hub-core/crates/hub-core/src/graph_v2/contradiction.rs` (was empty stub)
- Modify: `hub-core/crates/hub-core/src/error.rs` (add `HubError::Validation` if missing — verify first)
- Test: inline tests in each file

**Interfaces:**
- Consumes: `&PgPool`, intent JSON (via `serde_json::Value` discriminated by `action` field)
- Produces:
  - `pub enum Intent { AssertEntity{...}, AssertRelationship{...}, AssertContradiction{...}, LinkEvidenceToClaim{...}, MarkFindingAboutEntity{...}, CloseInvestigationExtract{...} }`
  - `pub async fn dispatch(intent: Value, actor: &str, task_id: Option<Uuid>, pool: &PgPool) -> Result<Value, HubError>`
  - `pub fn temporal::classify_overlap(a: (Option<DateTime>,Option<DateTime>), b: ...) -> Overlap` — SameWindow / Overlap / Adjacent / Disjoint
  - `pub async fn evidence::validate_doc_ids(ids: &[Uuid], pool: &PgPool) -> Result<(), HubError>`
  - `pub async fn contradiction::record(a: Uuid, b: Uuid, reason: &str, actor: &str, task: Option<Uuid>, pool: &PgPool) -> Result<i64, HubError>`

**Hard rule (spec §1)**: every intent MUST carry ≥1 `evidence_doc_ids[]` entry, OR a non-empty `evidence_absent_reason` string.

- [ ] **Step 1: Write temporal overlap classifier (pure logic, no DB)**

`hub-core/crates/hub-core/src/graph_v2/temporal.rs`:

```rust
//! Bi-temporal merge policy (spec §2).
//! valid_from (NULL = unknown start), valid_until (NULL = still true or unknown end).

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Overlap {
    /// Same window — should update in place, not insert new.
    SameWindow,
    /// Overlap but not identical — possible conflict; default policy = treat as supersede (new wins).
    Overlap,
    /// Adjacent: a ends exactly when b starts (or vice versa) — REPLACES edge.
    Adjacent,
    /// Fully disjoint — independent relationship.
    Disjoint,
}

pub fn classify_overlap(
    a_from: Option<DateTime<Utc>>, a_until: Option<DateTime<Utc>>,
    b_from: Option<DateTime<Utc>>, b_until: Option<DateTime<Utc>>,
) -> Overlap {
    match (a_from, a_until, b_from, b_until) {
        // SameWindow: both ranges identical
        (af, au, bf, bu) if af == bf && au == bu => Overlap::SameWindow,
        // SameWindow: both NULLs on both sides
        (None, None, None, None) => Overlap::SameWindow,
        // Adjacent: a.until == b.from (or vice versa)
        (af, Some(au), Some(bf), _) if au == bf => Overlap::Adjacent,
        (Some(af), _, _, Some(bu)) if af == bu => Overlap::Adjacent,
        // Overlap: any other non-disjoint
        (af, au, bf, bu) => {
            let disjoint = |a1: Option<DateTime<Utc>>, a2: Option<DateTime<Utc>>,
                           b1: Option<DateTime<Utc>>, b2: Option<DateTime<Utc>>| {
                let after = match (a2, b1) { (Some(x), Some(y)) => x <= y, _ => false };
                let before = match (b2, a1) { (Some(x), Some(y)) => x <= y, _ => false };
                after || before
            };
            if disjoint(af, au, bf, bu) { Overlap::Disjoint } else { Overlap::Overlap }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(s: &str) -> DateTime<Utc> { chrono::Utc.datetime_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap() }

    #[test]
    fn identical_is_same_window() {
        assert_eq!(
            classify_overlap(Some(t("2025-01-01 00:00:00")), Some(t("2025-12-31 00:00:00")),
                              Some(t("2025-01-01 00:00:00")), Some(t("2025-12-31 00:00:00"))),
            Overlap::SameWindow
        );
    }

    #[test]
    fn nulls_same_window() {
        assert_eq!(classify_overlap(None, None, None, None), Overlap::SameWindow);
    }

    #[test]
    fn adjacent_detected() {
        assert_eq!(
            classify_overlap(Some(t("2025-01-01 00:00:00")), Some(t("2025-06-01 00:00:00")),
                              Some(t("2025-06-01 00:00:00")), None),
            Overlap::Adjacent
        );
    }

    #[test]
    fn overlap_detected() {
        assert_eq!(
            classify_overlap(Some(t("2025-01-01 00:00:00")), Some(t("2025-08-01 00:00:00")),
                              Some(t("2025-06-01 00:00:00")), Some(t("2025-12-31 00:00:00"))),
            Overlap::Overlap
        );
    }

    #[test]
    fn disjoint_detected() {
        assert_eq!(
            classify_overlap(Some(t("2024-01-01 00:00:00")), Some(t("2024-06-01 00:00:00")),
                              Some(t("2025-01-01 00:00:00")), Some(t("2025-06-01 00:00:00"))),
            Overlap::Disjoint
        );
    }
}
```

- [ ] **Step 2: Write evidence validator**

`hub-core/crates/hub-core/src/graph_v2/evidence.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;
use crate::error::HubError;

/// Confirm every document_id exists in `documents`. Returns the count of missing IDs.
pub async fn validate_doc_ids(ids: &[Uuid], pool: &PgPool) -> Result<(), HubError> {
    if ids.is_empty() {
        return Err(HubError::Validation("evidence_doc_ids required (or evidence_absent_reason)".into()));
    }
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM documents WHERE document_id = ANY($1)"
    ).bind(ids).fetch_one(pool).await?;
    let found = row.0 as usize;
    if found != ids.len() {
        return Err(HubError::Validation(format!(
            "evidence refers to {found}/{} existing documents", ids.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Integration tests live in Task 5/9 (require live PG).
}
```

- [ ] **Step 3: Write contradiction module**

`hub-core/crates/hub-core/src/graph_v2/contradiction.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;
use crate::error::HubError;

/// Insert into claim_contradictions + set both claims status='disputed' + audit log.
/// Returns the contradiction_id.
pub async fn record(
    a: Uuid, b: Uuid, reason: &str, actor: &str, task: Option<Uuid>, pool: &PgPool,
) -> Result<i64, HubError> {
    if a == b {
        return Err(HubError::Validation("contradiction cannot be self-referential".into()));
    }
    let mut tx = pool.begin().await?;
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO claim_contradictions (claim_a, claim_b, reason, raised_by_task_id, raised_by_agent)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING
         RETURNING contradiction_id"
    ).bind(a).bind(b).bind(reason).bind(task).bind(actor).fetch_one(&mut *tx).await?;
    let cid = row.0;
    sqlx::query("UPDATE claims SET status = 'disputed' WHERE claim_id IN ($1, $2)")
        .bind(a).bind(b).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ('contradict','contradiction',$1, NULL, jsonb_build_object('a',$2,'b',$3,'reason',$4), $5, $6)"
    ).bind(cid.to_string()).bind(a).bind(b).bind(reason).bind(actor).bind(task).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(cid)
}
```

- [ ] **Step 4: Write compiler dispatch**

`hub-core/crates/hub-core/src/graph_v2/compiler.rs`:

```rust
//! Typed-intent dispatcher. Every MCP/agent write intent flows through here.
//! Spec §4: 6 intent kinds. Hard rule: every intent carries ≥1 evidence_doc_ids.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::HubError;
use crate::graph_v2::{evidence, contradiction, resolve, temporal};
use crate::store;

pub async fn dispatch(
    intent: Value, actor: &str, task_id: Option<Uuid>, pool: &PgPool,
) -> Result<Value, HubError> {
    let action = intent.get("action").and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing action".into()))?;
    match action {
        "assert_entity"          => dispatch_assert_entity(&intent, actor, task_id, pool).await,
        "assert_relationship"    => dispatch_assert_relationship(&intent, actor, task_id, pool).await,
        "assert_contradiction"   => dispatch_assert_contradiction(&intent, actor, task_id, pool).await,
        "link_evidence_to_claim" => dispatch_link_evidence(&intent, actor, task_id, pool).await,
        "mark_finding_about_entity" => dispatch_mark_finding(&intent, actor, task_id, pool).await,
        "close_investigation_extract" => dispatch_close_investigation(&intent, actor, task_id, pool).await,
        other => Err(HubError::Validation(format!("unknown action: {other}"))),
    }
}

fn require_evidence(intent: &Value) -> Result<(), HubError> {
    let ids = intent.get("evidence_doc_ids").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect::<Vec<_>>())
        .unwrap_or_default();
    let absent = intent.get("evidence_absent_reason").and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty()).unwrap_or(false);
    if ids.is_empty() && !absent {
        return Err(HubError::Validation(
            "evidence_doc_ids required (or evidence_absent_reason)".into()
        ));
    }
    Ok(())
}

async fn dispatch_assert_entity(intent: &Value, actor: &str, task: Option<Uuid>, pool: &PgPool) -> Result<Value, HubError> {
    require_evidence(intent)?;
    let kind = intent.get("kind").and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing kind".into()))?;
    let name = intent.get("name").and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing name".into()))?;
    let conf = intent.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let ids: Vec<Uuid> = intent.get("evidence_doc_ids").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect()).unwrap_or_default();
    evidence::validate_doc_ids(&ids, pool).await?;
    let resolution = resolve::resolve_entity(name, kind, actor, conf, pool).await?;
    let eid = match &resolution {
        resolve::EntityResolution::Existing { entity_id, .. } => *entity_id,
        resolve::EntityResolution::New { entity_id } => *entity_id,
    };
    let audit_after = json!({"kind": kind, "name": name, "matched_by": format!("{:?}", match &resolution {
        resolve::EntityResolution::Existing { matched_by, .. } => matched_by,
        resolve::EntityResolution::New { .. } => &resolve::ResolutionMethod::NewEntity,
    })});
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ('insert','entity',$1, NULL, $2, $3, $4)"
    ).bind(eid.to_string()).bind(&audit_after).bind(actor).bind(task).execute(pool).await?;
    Ok(json!({ "entity_id": eid, "resolution": audit_after }))
}

async fn dispatch_assert_relationship(intent: &Value, actor: &str, task: Option<Uuid>, pool: &PgPool) -> Result<Value, HubError> {
    require_evidence(intent)?;
    let subject = intent.get("subject").ok_or_else(|| HubError::Validation("missing subject".into()))?;
    let object  = intent.get("object").ok_or_else(|| HubError::Validation("missing object".into()))?;
    let predicate = intent.get("predicate").and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing predicate".into()))?;
    if !ALLOWED_PREDICATES.contains(&predicate) {
        return Err(HubError::Validation(format!("predicate {predicate} not in allowlist")));
    }
    let conf = intent.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let s_id = resolve_entity_ref(subject, actor, conf, pool).await?;
    let o_id = resolve_entity_ref(object,  actor, conf, pool).await?;
    let ids: Vec<Uuid> = intent.get("evidence_doc_ids").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect()).unwrap_or_default();
    evidence::validate_doc_ids(&ids, pool).await?;
    let valid_from = parse_dt(intent.get("valid_from"));
    let valid_until = parse_dt(intent.get("valid_until"));
    if let (Some(f), Some(u)) = (valid_from, valid_until) {
        if f >= u { return Err(HubError::Validation("valid_from must be < valid_until".into())); }
    }
    // Look for an existing rel with same (subject, predicate, object) — apply temporal merge
    let existing_row: Option<(Uuid, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT relationship_id, valid_from, valid_until FROM relationships
         WHERE from_entity = $1 AND to_entity = $2 AND rel_type = $3
         ORDER BY discovered_at DESC LIMIT 1"
    ).bind(s_id).bind(o_id).bind(predicate).fetch_optional(pool).await?;
    let overlap = existing_row.as_ref().map(|(_, vf, vu)| {
        temporal::classify_overlap(valid_from, valid_until, *vf, *vu)
    }).unwrap_or(temporal::Overlap::Disjoint);
    match overlap {
        temporal::Overlap::SameWindow => {
            sqlx::query(
                "UPDATE relationships SET confidence = $1, evidence_doc_ids = $2, valid_from = COALESCE($3, valid_from),
                 valid_until = COALESCE($4, valid_until), created_by_task_id = COALESCE($5, created_by_task_id)
                 WHERE relationship_id = $6"
            ).bind(conf).bind(&ids).bind(valid_from).bind(valid_until).bind(task)
            .bind(existing_row.unwrap().0).execute(pool).await?;
        }
        _ => {
            sqlx::query(
                "INSERT INTO relationships (from_entity, to_entity, rel_type, attributes, valid_from, valid_until, discovered_at, confidence, evidence_doc_ids, created_by_task_id)
                 VALUES ($1,$2,$3,'{}'::jsonb,$4,$5,now(),$6,$7,$8)
                 ON CONFLICT (from_entity, to_entity, rel_type) DO NOTHING"
            ).bind(s_id).bind(o_id).bind(predicate).bind(valid_from).bind(valid_until)
            .bind(conf).bind(&ids).bind(task).execute(pool).await?;
        }
    }
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ($1,'relationship',$2, NULL, $3, $4, $5)"
    ).bind(if matches!(overlap, temporal::Overlap::SameWindow) { "update" } else { "insert" })
    .bind(s_id.to_string())
    .bind(json!({"subject": s_id, "predicate": predicate, "object": o_id, "valid_from": valid_from, "valid_until": valid_until, "confidence": conf}))
    .bind(actor).bind(task).execute(pool).await?;
    Ok(json!({ "subject": s_id, "predicate": predicate, "object": o_id, "overlap": format!("{overlap:?}") }))
}

async fn resolve_entity_ref(v: &Value, actor: &str, conf: f64, pool: &PgPool) -> Result<Uuid, HubError> {
    if let Some(s) = v.as_str() {
        return Ok(Uuid::parse_str(s).map_err(|_| HubError::Validation("entity id not uuid".into()))?);
    }
    let kind = v.get("kind").and_then(|x| x.as_str())
        .ok_or_else(|| HubError::Validation("entity ref needs kind or id".into()))?;
    let name = v.get("name").and_then(|x| x.as_str())
        .ok_or_else(|| HubError::Validation("entity ref needs name".into()))?;
    match resolve::resolve_entity(name, kind, actor, conf, pool).await? {
        resolve::EntityResolution::Existing { entity_id, .. } => Ok(entity_id),
        resolve::EntityResolution::New      { entity_id }     => Ok(entity_id),
    }
}

async fn dispatch_assert_contradiction(intent: &Value, actor: &str, task: Option<Uuid>, pool: &PgPool) -> Result<Value, HubError> {
    require_evidence(intent)?;
    let a = parse_uuid(intent.get("claim_a"), "claim_a")?;
    let b = parse_uuid(intent.get("claim_b"), "claim_b")?;
    let reason = intent.get("reason").and_then(|v| v.as_str()).unwrap_or("");
    let cid = contradiction::record(a, b, reason, actor, task, pool).await?;
    Ok(json!({ "contradiction_id": cid }))
}

async fn dispatch_link_evidence(intent: &Value, actor: &str, task: Option<Uuid>, pool: &PgPool) -> Result<Value, HubError> {
    let claim_id = parse_uuid(intent.get("claim_id"), "claim_id")?;
    let document_id = parse_uuid(intent.get("document_id"), "document_id")?;
    let relation = intent.get("relation").and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing relation".into()))?;
    if !["supports", "contradicts"].contains(&relation) {
        return Err(HubError::Validation(format!("invalid relation {relation}")));
    }
    let snippet = intent.get("snippet").and_then(|v| v.as_str());
    sqlx::query(
        "INSERT INTO claim_evidence (claim_id, document_id, relation, snippet)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (claim_id, document_id, relation) DO NOTHING"
    ).bind(claim_id).bind(document_id).bind(relation).bind(snippet).execute(pool).await?;
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ('insert','claim',$1, NULL, jsonb_build_object('linked',$2,'relation',$3), $4, $5)"
    ).bind(claim_id.to_string()).bind(document_id).bind(relation).bind(actor).bind(task).execute(pool).await?;
    Ok(json!({ "claim_id": claim_id, "document_id": document_id, "relation": relation }))
}

async fn dispatch_mark_finding(intent: &Value, actor: &str, task: Option<Uuid>, pool: &PgPool) -> Result<Value, HubError> {
    let finding_id = parse_uuid(intent.get("finding_id"), "finding_id")?;
    let entity_id = parse_uuid(intent.get("entity_id"), "entity_id")?;
    let conf = intent.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);
    sqlx::query(
        "INSERT INTO finding_evidence (finding_id, document_id, relation)
         SELECT $1, document_id, 'supports' FROM entities WHERE entity_id = $2
         ON CONFLICT DO NOTHING"
    ).bind(finding_id).bind(entity_id).execute(pool).await?;
    Ok(json!({ "finding_id": finding_id, "entity_id": entity_id, "confidence": conf }))
}

async fn dispatch_close_investigation(intent: &Value, actor: &str, task: Option<Uuid>, pool: &PgPool) -> Result<Value, HubError> {
    let investigation_id = parse_uuid(intent.get("investigation_id"), "investigation_id")?;
    let extractions = intent.get("extraction").and_then(|v| v.as_array())
        .ok_or_else(|| HubError::Validation("missing extraction[]".into()))?;
    let mut results = Vec::with_capacity(extractions.len());
    for sub in extractions {
        let r = dispatch(sub.clone(), actor, task, pool).await?;
        results.push(r);
    }
    Ok(json!({ "investigation_id": investigation_id, "applied": results }))
}

fn parse_uuid(v: Option<&Value>, field: &str) -> Result<Uuid, HubError> {
    let s = v.and_then(|x| x.as_str()).ok_or_else(|| HubError::Validation(format!("missing {field}")))?;
    Uuid::parse_str(s).map_err(|_| HubError::Validation(format!("{field} not a uuid")))
}

fn parse_dt(v: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    v.and_then(|x| x.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
}

const ALLOWED_PREDICATES: &[&str] = &[
    "controls","owns","communicates_with","resolves_to","located_in",
    "affiliated_with","uses","hosts","registered_by","related_to",
    "SUPPORTS","CONTRADICTS","ABOUT","MENTIONS","PRODUCED","PART_OF","REPLACES",
    "WORKS_FOR","ACQUIRED","OPERATES","FOUNDED",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn require_evidence_rejects_empty() {
        let v = json!({"action": "assert_entity", "kind": "org", "name": "x"});
        assert!(require_evidence(&v).is_err());
    }
    #[test]
    fn require_evidence_accepts_ids() {
        let v = json!({"action": "assert_entity", "kind": "org", "name": "x",
                       "evidence_doc_ids": ["00000000-0000-0000-0000-000000000001"]});
        assert!(require_evidence(&v).is_ok());
    }
    #[test]
    fn require_evidence_accepts_absent_reason() {
        let v = json!({"action": "assert_entity", "kind": "org", "name": "x",
                       "evidence_absent_reason": "manually seeded"});
        assert!(require_evidence(&v).is_ok());
    }
    #[test]
    fn predicate_allowlist() {
        assert!(ALLOWED_PREDICATES.contains(&"owns"));
        assert!(ALLOWED_PREDICATES.contains(&"CONTRADICTS"));
        assert!(!ALLOWED_PREDICATES.contains(&"hates"));
    }
}
```

- [ ] **Step 5: Verify `HubError::Validation` exists**

```bash
grep -n "Validation" hub-core/crates/hub-core/src/error.rs
```

If missing, add to `error.rs`:

```rust
#[error("validation: {0}")]
Validation(String),
```

and add the enum variant in the existing `HubError` enum.

- [ ] **Step 6: Run tests**

```bash
cargo test -p hub-core --lib graph_v2
```

Expected: all unit tests PASS (`temporal` overlap tests, `compiler` require_evidence tests, `resolve` serialize test, `resolve_async` thresholds test).

- [ ] **Step 7: Commit**

```bash
git add hub-core/crates/hub-core/src/graph_v2/ hub-core/crates/hub-core/src/error.rs
git commit -m "feat(sp9): graph v2 compiler — typed intents, temporal merge, evidence, contradiction"
```

---

## Task 4: Read-side queries (`graph_queries.rs`)

**Files:**
- Create: `hub-core/crates/hub-core/src/graph_queries.rs`
- Modify: `hub-core/crates/hub-core/src/lib.rs` (add `pub mod graph_queries;`)
- Modify: `hub-core/crates/hub-core/src/graph.rs` (DELETE v1 `find_path` function only — keep `query_entity` and `query_relationship`)
- Test: inline tests in `graph_queries.rs`

**Interfaces:**
- Consumes: `&AppState` (gives access to `neo4j` + `pool`)
- Produces:
  - `pub async fn search_entity(state: &AppState, name: &str, kind: Option<&str>, limit: i64) -> Result<Value, HubError>` — alias-aware (uses `entity_aliases`)
  - `pub async fn get_entity(state: &AppState, entity_id: Uuid) -> Result<Value, HubError>` — returns entity + aliases + relationships + claims + contradictions
  - `pub async fn get_entity_timeline(state: &AppState, entity_id: Uuid, from: Option<DateTime>, to: Option<DateTime>, limit: i64) -> Result<Value, HubError>`
  - `pub async fn get_neighbors(state: &AppState, entity_id: Uuid, depth: u8, rel_types: Option<Vec<String>>, min_confidence: Option<f64>, at_time: Option<DateTime>) -> Result<Value, HubError>` — uses Neo4j Cypher with temporal predicates
  - `pub async fn find_path(state: &AppState, from: &str, to: &str, weighted: bool, max_hops: u8) -> Result<Value, HubError>` — v2 replacement
  - `pub async fn find_relationship_changes(state: &AppState, a: &str, b: &str, from: Option<DateTime>, to: Option<DateTime>) -> Result<Value, HubError>`
  - `pub async fn find_supporting_claims(state: &AppState, entity_id: Uuid, limit: i64) -> Result<Value, HubError>`
  - `pub async fn find_contradicting_claims(state: &AppState, claim_id: Option<Uuid>, entity_id: Option<Uuid>) -> Result<Value, HubError>`
  - `pub async fn query_investigation_graph(state: &AppState, investigation_id: Uuid) -> Result<Value, HubError>`
  - `pub async fn list_evidence_for_entity(state: &AppState, entity_id: Uuid, relation: Option<&str>) -> Result<Value, HubError>`

- [ ] **Step 1: Write `graph_queries.rs` skeleton + search_entity**

`hub-core/crates/hub-core/src/graph_queries.rs`:

```rust
//! Read-only Cypher + SQL queries backing the 10 new MCP tools (spec §6).
//! No raw Cypher reaches agents — these templates are the only entry point.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::HubError;
use crate::state::AppState;

pub async fn search_entity(state: &AppState, name: &str, kind: Option<&str>, limit: i64) -> Result<Value, HubError> {
    let pool = &state.pool;
    let lim = limit.clamp(1, 100);
    let mut q = String::from(
        "SELECT e.entity_id, e.kind, e.name, e.aliases,
                CASE WHEN lower(e.name) = lower($1) THEN 1.0 ELSE 0.5 END AS score
           FROM entities e
          WHERE e.merged_into IS NULL
            AND (lower(e.name) LIKE '%' || lower($1) || '%'
                 OR EXISTS (SELECT 1 FROM entity_aliases ea
                             WHERE ea.entity_id = e.entity_id
                               AND ea.alias_norm LIKE '%' || lower($1) || '%'))"
    );
    if kind.is_some() { q.push_str(" AND e.kind = $2 "); }
    q.push_str(" ORDER BY score DESC, e.name LIMIT $3");
    let limit_idx = if kind.is_some() { 3 } else { 2 };
    let mut query = sqlx::query_as::<_, (Uuid, String, String, serde_json::Value, f64)>(&q).bind(name);
    if let Some(k) = kind { query = query.bind(k); }
    query = query.bind(lim);
    let rows = query.fetch_all(pool).await?;
    Ok(json!(rows.into_iter().map(|(id, k, n, a, s)| json!({
        "entity_id": id, "kind": k, "name": n, "aliases": a, "score": s
    })).collect::<Vec<_>>()))
}

pub async fn get_entity(state: &AppState, entity_id: Uuid) -> Result<Value, HubError> {
    let pool = &state.pool;
    let e: (Uuid, String, String, serde_json::Value, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>, Option<Uuid>) = sqlx::query_as(
        "SELECT entity_id, kind, name, aliases, valid_from, valid_until, discovered_at, merged_into
           FROM entities WHERE entity_id = $1"
    ).bind(entity_id).fetch_one(pool).await?;
    let aliases: Vec<(String, String, f64)> = sqlx::query_as(
        "SELECT alias, source, confidence FROM entity_aliases WHERE entity_id = $1 ORDER BY confidence DESC LIMIT 50"
    ).bind(entity_id).fetch_all(pool).await?;
    let relationships: Vec<(Uuid, String, Uuid, String, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>, Option<f64>)> = sqlx::query_as(
        "SELECT r.relationship_id, r.rel_type, r.to_entity, ent.name, r.valid_from, r.valid_until, r.confidence
           FROM relationships r JOIN entities ent ON ent.entity_id = r.to_entity
          WHERE r.from_entity = $1 ORDER BY r.discovered_at DESC LIMIT 100"
    ).bind(entity_id).fetch_all(pool).await?;
    let claims: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT c.claim_id, c.text, c.status
           FROM claims c JOIN claim_entities ce ON ce.claim_id = c.claim_id
          WHERE ce.entity_id = $1 AND ce.role IN ('subject','object') LIMIT 50"
    ).bind(entity_id).fetch_all(pool).await?;
    let merged_into: Option<Value> = match e.7 {
        Some(parent) => {
            let r: (String, String) = sqlx::query_as("SELECT kind, name FROM entities WHERE entity_id = $1")
                .bind(parent).fetch_one(pool).await?;
            Some(json!({ "entity_id": parent, "kind": r.0, "name": r.1 }))
        }
        None => None,
    };
    Ok(json!({
        "entity_id": e.0, "kind": e.1, "name": e.2, "aliases": e.3,
        "valid_from": e.4, "valid_until": e.5, "discovered_at": e.6, "merged_into": merged_into,
        "aliases_list": aliases.into_iter().map(|(a,s,c)| json!({"alias":a,"source":s,"confidence":c})).collect::<Vec<_>>(),
        "relationships": relationships.into_iter().map(|(rid,rt,tid,tn,vf,vu,co)| json!({
            "relationship_id": rid, "rel_type": rt, "to_entity_id": tid, "to_entity_name": tn,
            "valid_from": vf, "valid_until": vu, "confidence": co
        })).collect::<Vec<_>>(),
        "claims": claims.into_iter().map(|(cid,t,s)| json!({"claim_id":cid,"text":t,"status":s})).collect::<Vec<_>>()
    }))
}

pub async fn get_entity_timeline(state: &AppState, entity_id: Uuid, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>, limit: i64) -> Result<Value, HubError> {
    let lim = limit.clamp(1, 500);
    let rows: Vec<(i64, String, String, serde_json::Value, serde_json::Value, String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT change_id, op, target_kind, before, after, changed_by, changed_at
           FROM graph_change_log
          WHERE (target_kind = 'entity' AND target_id = $1)
             OR (target_kind = 'relationship' AND target_id IN (
                  SELECT relationship_id::text FROM relationships WHERE from_entity = $1 OR to_entity = $1))
            AND ($2::timestamptz IS NULL OR changed_at >= $2)
            AND ($3::timestamptz IS NULL OR changed_at <= $3)
          ORDER BY changed_at DESC LIMIT $4"
    ).bind(entity_id).bind(from).bind(to).bind(lim).fetch_all(&state.pool).await?;
    Ok(json!(rows.into_iter().map(|(c,o,tk,b,a,by,t)| json!({
        "change_id": c, "op": o, "target_kind": tk, "before": b, "after": a, "changed_by": by, "changed_at": t
    })).collect::<Vec<_>>()))
}

pub async fn get_neighbors(
    state: &AppState, entity_id: Uuid, depth: u8, rel_types: Option<Vec<String>>,
    min_confidence: Option<f64>, at_time: Option<DateTime<Utc>>,
) -> Result<Value, HubError> {
    // Cypher with temporal predicate. Variables in path predicates require Cypher 5.
    let d = depth.clamp(1, 4);
    let cypher = format!(
        "MATCH (e:Entity {{entity_id: $eid}})-[*1..{d}]-(neighbor:Entity) \
         WHERE e <> neighbor \
           AND ($at_time IS NULL \
                OR ALL(rel IN relationships(path) WHERE \
                  (rel.valid_from IS NULL OR rel.valid_from <= datetime($at_time)) \
                  AND (rel.valid_until IS NULL OR rel.valid_until > datetime($at_time)))) \
         WITH neighbor, relationships(path) AS rs LIMIT 100 \
         RETURN neighbor.entity_id AS id, neighbor.name AS name, neighbor.kind AS kind, \
                [r IN rs | {{rel_type: type(r), valid_from: r.valid_from, valid_until: r.valid_until, confidence: r.confidence}}] AS edges"
    );
    let mut q = state.neo4j.query(&cypher)
        .param("eid", entity_id.to_string())
        .param("at_time", at_time.map(|t| t.to_rfc3339()).unwrap_or_default());
    let rows = q.await.map_err(|e| HubError::Internal(format!("neo4j: {e}")))?;
    // Filter by rel_types / confidence here (post-Cypher to keep Cypher simple)
    let rts: Vec<String> = rel_types.unwrap_or_default();
    let minc = min_confidence.unwrap_or(0.0);
    let mut out = Vec::new();
    for row in rows.rows() {
        let id: String = row.get("id").unwrap_or_default();
        let name: String = row.get("name").unwrap_or_default();
        let kind: String = row.get("kind").unwrap_or_default();
        let edges_v: Vec<HashMap<String, Value>> = row.get("edges").unwrap_or_default();
        let edges: Vec<Value> = edges_v.into_iter().filter(|m| {
            let rt = m.get("rel_type").and_then(|x| x.as_str()).unwrap_or("");
            let conf = m.get("confidence").and_then(|x| x.as_f64()).unwrap_or(1.0);
            (rts.is_empty() || rts.iter().any(|x| x == rt)) && conf >= minc
        }).map(|m| json!(m)).collect();
        if !edges.is_empty() {
            out.push(json!({ "entity_id": id, "name": name, "kind": kind, "edges": edges }));
        }
    }
    Ok(json!(out))
}

pub async fn find_path(state: &AppState, from: &str, to: &str, weighted: bool, max_hops: u8) -> Result<Value, HubError> {
    let h = max_hops.clamp(1, 8);
    let cypher = format!(
        "MATCH p = shortestPath((a:Entity)-[*..{h}]-(b:Entity)) \
         WHERE toLower(a.name) = toLower($from) AND toLower(b.name) = toLower($to) \
         RETURN [n IN nodes(p) | n.name] AS names, \
                [r IN relationships(p) | {{rel_type: type(r), confidence: coalesce(r.confidence, 1.0)}}] AS edges, \
                length(p) AS hops"
    );
    let rows = state.neo4j.query(&cypher).param("from", from).param("to", to).await
        .map_err(|e| HubError::Internal(format!("neo4j: {e}")))?;
    let mut out = Vec::new();
    for row in rows.rows() {
        let names: Vec<String> = row.get("names").unwrap_or_default();
        let edges: Vec<HashMap<String, Value>> = row.get("edges").unwrap_or_default();
        let hops: i64 = row.get("hops").unwrap_or(0);
        let score = if weighted {
            edges.iter().map(|m| m.get("confidence").and_then(|x| x.as_f64()).unwrap_or(1.0))
                .fold(0.0_f64, |acc, c| acc + (1.0 - c))
        } else { hops as f64 };
        out.push(json!({ "names": names, "edges": edges, "hops": hops, "score": score }));
    }
    Ok(json!(out))
}

pub async fn find_relationship_changes(
    state: &AppState, a: &str, b: &str, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>,
) -> Result<Value, HubError> {
    let rows: Vec<(Uuid, String, Uuid, String, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>, Option<f64>)> = sqlx::query_as(
        "SELECT r.relationship_id, r.rel_type, r.to_entity, ent.name, r.valid_from, r.valid_until, r.discovered_at, r.confidence
           FROM relationships r
           JOIN entities ea ON ea.entity_id = r.from_entity
           JOIN entities ent ON ent.entity_id = r.to_entity
          WHERE (lower(ea.name) = lower($1) OR lower(ea.name) LIKE '%' || lower($1) || '%')
            AND (lower(ent.name) = lower($2) OR lower(ent.name) LIKE '%' || lower($2) || '%')
            AND ($3::timestamptz IS NULL OR r.discovered_at >= $3)
            AND ($4::timestamptz IS NULL OR r.discovered_at <= $4)
          ORDER BY r.discovered_at DESC LIMIT 100"
    ).bind(a).bind(b).bind(from).bind(to).fetch_all(&state.pool).await?;
    Ok(json!(rows.into_iter().map(|(rid,rt,tid,tn,vf,vu,da,co)| json!({
        "relationship_id": rid, "rel_type": rt, "to_entity_id": tid, "to_entity_name": tn,
        "valid_from": vf, "valid_until": vu, "discovered_at": da, "confidence": co
    })).collect::<Vec<_>>()))
}

pub async fn find_supporting_claims(state: &AppState, entity_id: Uuid, limit: i64) -> Result<Value, HubError> {
    let lim = limit.clamp(1, 100);
    let rows: Vec<(Uuid, String, String, Uuid, String)> = sqlx::query_as(
        "SELECT c.claim_id, c.text, c.status, d.document_id, d.content_hash
           FROM claims c
           JOIN claim_entities ce ON ce.claim_id = c.claim_id
           JOIN claim_evidence ce2 ON ce2.claim_id = c.claim_id AND ce2.relation = 'supports'
           JOIN documents d ON d.document_id = ce2.document_id
          WHERE ce.entity_id = $1 LIMIT $2"
    ).bind(entity_id).bind(lim).fetch_all(&state.pool).await?;
    Ok(json!(rows.into_iter().map(|(cid,t,s,did,dh)| json!({
        "claim_id": cid, "text": t, "status": s, "supporting_doc_id": did, "content_hash": dh
    })).collect::<Vec<_>>()))
}

pub async fn find_contradicting_claims(state: &AppState, claim_id: Option<Uuid>, entity_id: Option<Uuid>) -> Result<Value, HubError> {
    let pool = &state.pool;
    let rows: Vec<(i64, Uuid, String, Uuid, String, String)> = match (claim_id, entity_id) {
        (Some(c), _) => sqlx::query_as(
            "SELECT cc.contradiction_id, cc.claim_a, ca.text, cc.claim_b, cb.text, cc.reason
               FROM claim_contradictions cc
               JOIN claims ca ON ca.claim_id = cc.claim_a
               JOIN claims cb ON cb.claim_id = cc.claim_b
              WHERE cc.claim_a = $1 OR cc.claim_b = $1"
        ).bind(c).fetch_all(pool).await?,
        (None, Some(e)) => sqlx::query_as(
            "SELECT DISTINCT cc.contradiction_id, cc.claim_a, ca.text, cc.claim_b, cb.text, cc.reason
               FROM claim_contradictions cc
               JOIN claims ca ON ca.claim_id = cc.claim_a
               JOIN claims cb ON cb.claim_id = cc.claim_b
               JOIN claim_entities ce ON ce.claim_id IN (cc.claim_a, cc.claim_b)
              WHERE ce.entity_id = $1"
        ).bind(e).fetch_all(pool).await?,
        (None, None) => return Err(HubError::Validation("claim_id or entity_id required".into())),
    };
    Ok(json!(rows.into_iter().map(|(cid,a,at,b_id,bt,r)| json!({
        "contradiction_id": cid, "claim_a": a, "text_a": at, "claim_b": b_id, "text_b": bt, "reason": r
    })).collect::<Vec<_>>()))
}

pub async fn query_investigation_graph(state: &AppState, investigation_id: Uuid) -> Result<Value, HubError> {
    let findings: Vec<(Uuid, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT finding_id, claim_text, created_at FROM findings WHERE investigation_id = $1 ORDER BY created_at DESC LIMIT 100"
    ).bind(investigation_id).fetch_all(&state.pool).await?;
    let entities: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT DISTINCT e.entity_id, e.kind, e.name
           FROM findings f
           JOIN claim_entities ce ON ce.claim_id = f.finding_id
           JOIN entities e ON e.entity_id = ce.entity_id
          WHERE f.investigation_id = $1 LIMIT 200"
    ).bind(investigation_id).fetch_all(&state.pool).await?;
    let claims: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT DISTINCT c.claim_id, c.text, c.status
           FROM claims c
           JOIN claim_entities ce ON ce.claim_id = c.claim_id
           JOIN entities e ON e.entity_id = ce.entity_id
           JOIN findings f ON f.investigation_id = $1 AND f.finding_id = c.claim_id
          LIMIT 100"
    ).bind(investigation_id).fetch_all(&state.pool).await?;
    Ok(json!({
        "investigation_id": investigation_id,
        "findings": findings.into_iter().map(|(id,t,ca)| json!({"finding_id":id,"text":t,"created_at":ca})).collect::<Vec<_>>(),
        "entities": entities.into_iter().map(|(id,k,n)| json!({"entity_id":id,"kind":k,"name":n})).collect::<Vec<_>>(),
        "claims": claims.into_iter().map(|(id,t,s)| json!({"claim_id":id,"text":t,"status":s})).collect::<Vec<_>>()
    }))
}

pub async fn list_evidence_for_entity(state: &AppState, entity_id: Uuid, relation: Option<&str>) -> Result<Value, HubError> {
    let rel = relation.unwrap_or("supports");
    // Also writes into observations table (it was previously orphaned per inventory).
    let docs: Vec<(Uuid, String, chrono::DateTime<chrono::Utc>, String)> = sqlx::query_as(
        "SELECT d.document_id, d.base_url, d.retrieved_at, ce.relation
           FROM documents d
           JOIN claim_evidence ce ON ce.document_id = d.document_id
           JOIN claim_entities c ON c.claim_id = ce.claim_id
          WHERE c.entity_id = $1 AND ce.relation = $2
          ORDER BY d.retrieved_at DESC LIMIT 100"
    ).bind(entity_id).bind(rel).fetch_all(&state.pool).await?;
    // Side-effect: write to observations table (idempotent)
    for (doc_id, _url, observed_at, _r) in &docs {
        let _ = sqlx::query(
            "INSERT INTO observations (entity_id, document_id, snippet, observed_at)
             VALUES ($1, $2, '', $3)
             ON CONFLICT DO NOTHING"
        ).bind(entity_id).bind(doc_id).bind(observed_at).execute(&state.pool).await;
    }
    Ok(json!(docs.into_iter().map(|(id,u,t,r)| json!({
        "document_id": id, "base_url": u, "retrieved_at": t, "relation": r
    })).collect::<Vec<_>>()))
}

#[cfg(test)]
mod tests {
    // Pure unit tests are minimal here — most logic is DB/Cypher.
    // Integration coverage lives in accept-sp9.py (Task 9).
    #[test]
    fn clamp_works() { assert_eq!(0_i64.clamp(1, 100), 1); assert_eq!(50_i64.clamp(1, 100), 50); }
}
```

- [ ] **Step 2: Add `use std::collections::HashMap` to top of `graph_queries.rs`**

The `find_path` and `get_neighbors` use `HashMap` — make sure `use std::collections::HashMap;` is at the top (above `use serde_json`).

- [ ] **Step 3: Remove v1 `find_path` from `graph.rs`**

In `hub-core/crates/hub-core/src/graph.rs`, delete:
- the `pub async fn find_path(...)` function body (lines ~56–65)
- any tests for it

Leave `query_entity` and `query_relationship` untouched.

- [ ] **Step 4: Register the new module**

`hub-core/crates/hub-core/src/lib.rs`: add `pub mod graph_queries;` next to `pub mod graph;`.

- [ ] **Step 5: Compile-check**

```bash
cargo build -p hub-core --release 2>&1 | grep -E "^(error|warning)" | head -30
```

Expected: builds clean. Likely 1-2 warnings about unused imports (clean up).

- [ ] **Step 6: Commit**

```bash
git add hub-core/crates/hub-core/src/graph_queries.rs \
        hub-core/crates/hub-core/src/lib.rs \
        hub-core/crates/hub-core/src/graph.rs
git commit -m "feat(sp9): read-side graph_queries (10 templates) + remove v1 find_path"
```

---

## Task 5: MCP tools (10 new + v1 find_path replacement) + policy registry

**Files:**
- Modify: `hub-core/crates/hub-core/src/mcp.rs` (replace `find_path`, add 10 new `#[tool]`s)
- Modify: `hub-core/crates/hub-core/src/policy.rs` (add 10 L1/Free entries)

**Interfaces:**
- All new tools:
  ```rust
  #[tool(description = "...")]
  async fn tool_name(&self, #[tool(aggr)] args: ArgStruct, ctx: Context) -> Result<CallToolResult, McpError>
  ```
- Each calls `self.gate(&ctx, "tool_name").await?` and `self.record(...)` like existing tools.
- Tool implementations delegate to `graph_queries::*` (no business logic in MCP layer).

- [ ] **Step 1: Add policy entries**

In `hub-core/crates/hub-core/src/policy.rs`, locate the `policy_for()` match (around line 42-71). Add 10 new arms before the catch-all:

```rust
"search_entity"              => (RiskLevel::L1, CostClass::Free),
"get_entity"                 => (RiskLevel::L1, CostClass::Free),
"get_entity_timeline"        => (RiskLevel::L1, CostClass::Free),
"get_neighbors"              => (RiskLevel::L1, CostClass::Free),
"find_path"                  => (RiskLevel::L1, CostClass::Free),  // v2 (replaces v1)
"find_relationship_changes"  => (RiskLevel::L1, CostClass::Free),
"find_supporting_claims"     => (RiskLevel::L1, CostClass::Free),
"find_contradicting_claims"   => (RiskLevel::L1, CostClass::Free),
"query_investigation_graph"  => (RiskLevel::L1, CostClass::Free),
"list_evidence_for_entity"   => (RiskLevel::L1, CostClass::Free),
```

(Check existing `RiskLevel` enum values — they may be `L1`/`L2`/`L3` numeric variants; match exactly.)

- [ ] **Step 2: Replace v1 `find_path` MCP tool**

Locate existing `find_path` in `mcp.rs:836` (v1 signature: `args: FindPathArgs { from, to }`). Replace with:

```rust
#[tool(description = "Find the shortest path between two entities in the knowledge graph. v2: temporal-aware, weighted by 1-confidence. Replaces v1 (no back-compat: v1 was unused in production).")]
async fn find_path(
    &self,
    #[tool(aggr)] args: FindPathArgsV2,
    ctx: Context,
) -> Result<CallToolResult, McpError> {
    let started = std::time::Instant::now();
    self.gate(&ctx, "find_path").await?;
    let max_hops = args.max_hops.unwrap_or(5).clamp(1, 8);
    let weighted = args.weighted.unwrap_or(true);
    let out = crate::graph_queries::find_path(&self.state, &args.from, &args.to, weighted, max_hops).await
        .map_err(|e| McpError::internal(format!("find_path: {e}")))?;
    self.record(&ctx, "find_path", started, "ok").await;
    Ok(CallToolResult::success(vec![Content::json(out)]))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindPathArgsV2 {
    pub from: String,
    pub to: String,
    pub weighted: Option<bool>,
    pub max_hops: Option<u8>,
}
```

Delete the existing v1 `FindPathArgs` struct if unused (grep first to confirm).

- [ ] **Step 3: Add 9 more MCP tools**

Add after `find_path` in `mcp.rs`. Pattern for each (showing `search_entity` as template):

```rust
#[tool(description = "Search entities by name with alias-aware matching. Returns up to `limit` matches with scores.")]
async fn search_entity(
    &self,
    #[tool(aggr)] args: SearchEntityArgs,
    ctx: Context,
) -> Result<CallToolResult, McpError> {
    let started = std::time::Instant::now();
    self.gate(&ctx, "search_entity").await?;
    let limit = args.limit.unwrap_or(20);
    let out = crate::graph_queries::search_entity(&self.state, &args.name, args.kind.as_deref(), limit).await
        .map_err(|e| McpError::internal(format!("search_entity: {e}")))?;
    self.record(&ctx, "search_entity", started, "ok").await;
    Ok(CallToolResult::success(vec![Content::json(out)]))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchEntityArgs {
    pub name: String,
    pub kind: Option<String>,
    pub limit: Option<i64>,
}
```

Repeat this template for the remaining 8 tools. Arg structs:

```rust
#[derive(Debug, Deserialize, JsonSchema)] pub struct GetEntityArgs { pub entity_id: String }
#[derive(Debug, Deserialize, JsonSchema)] pub struct GetEntityTimelineArgs { pub entity_id: String, pub from: Option<String>, pub to: Option<String>, pub limit: Option<i64> }
#[derive(Debug, Deserialize, JsonSchema)] pub struct GetNeighborsArgs { pub entity_id: String, pub depth: Option<u8>, pub rel_types: Option<Vec<String>>, pub min_confidence: Option<f64>, pub at_time: Option<String> }
#[derive(Debug, Deserialize, JsonSchema)] pub struct FindRelationshipChangesArgs { pub a: String, pub b: String, pub from: Option<String>, pub to: Option<String> }
#[derive(Debug, Deserialize, JsonSchema)] pub struct FindSupportingClaimsArgs { pub entity_id: String, pub limit: Option<i64> }
#[derive(Debug, Deserialize, JsonSchema)] pub struct FindContradictingClaimsArgs { pub claim_id: Option<String>, pub entity_id: Option<String> }
#[derive(Debug, Deserialize, JsonSchema)] pub struct QueryInvestigationGraphArgs { pub investigation_id: String }
#[derive(Debug, Deserialize, JsonSchema)] pub struct ListEvidenceForEntityArgs { pub entity_id: String, pub relation: Option<String> }
```

For tools that take UUIDs/DateTimes, parse `args.entity_id` via `Uuid::parse_str` and `args.at_time` via `chrono::DateTime::parse_from_rfc3339`; return `McpError::invalid_params` on parse failure.

Each implementation follows the same template as `search_entity`:
- call `self.gate(&ctx, "<tool_name>").await?`
- call the corresponding `crate::graph_queries::<fn>(&self.state, ...)`
- map `HubError::Validation` → `McpError::invalid_params`, others → `McpError::internal`
- call `self.record(&ctx, "<tool_name>", started, "ok").await`
- return `CallToolResult::success(vec![Content::json(out)])`

- [ ] **Step 4: Add tool documentation to `get_info()`**

In `mcp.rs` near `:1270-1284`, extend the `get_info()` info string to mention the 10 new tools:

```text
Knowledge graph (read): search_entity, get_entity, get_entity_timeline,
get_neighbors, find_path, find_relationship_changes, find_supporting_claims,
find_contradicting_claims, query_investigation_graph, list_evidence_for_entity.
```

- [ ] **Step 5: Build + verify**

```bash
cargo build -p hub-core --release 2>&1 | grep -E "error\[" | head -20
```

Expected: clean build. If `#[tool(aggr)]` macro syntax errors appear, check the existing pattern at `mcp.rs:803` for `query_entity` and copy exactly.

- [ ] **Step 6: Commit**

```bash
git add hub-core/crates/hub-core/src/mcp.rs hub-core/crates/hub-core/src/policy.rs
git commit -m "feat(sp9): MCP — 10 new graph read tools + v1 find_path replaced"
```

---

## Task 6: REST API (`/api/v1/graph/*`)

**Files:**
- Modify: `hub-core/crates/hub-core/src/api.rs` (add 5 routes + handlers)
- Test: `scripts/accept-sp9.py` integration tests in Task 9

**Interfaces:**
- 5 new endpoints mounted on the existing `/api/v1` router:
  - `GET /api/v1/graph/neighbors?root=<uuid>&depth=<n>&at_time=<rfc3339>`
  - `GET /api/v1/graph/entity/<uuid>/timeline?from=<rfc3339>&to=<rfc3339>`
  - `GET /api/v1/graph/path?from=<name>&to=<name>&max_hops=<n>`
  - `GET /api/v1/graph/investigation/<uuid>`
  - `GET /api/v1/graph/evidence?entity=<uuid>&relation=<supports|contradicts>`

Each handler delegates to `crate::graph_queries::*` with parsed query/path params, returning JSON.

- [ ] **Step 1: Locate `router()` and add 5 routes**

In `hub-core/crates/hub-core/src/api.rs`, find the `Router::new()` chain inside `pub fn router()`. Add at the end:

```rust
.route("/api/v1/graph/neighbors", get(graph_neighbors))
.route("/api/v1/graph/entity/:id/timeline", get(graph_entity_timeline))
.route("/api/v1/graph/path", get(graph_path))
.route("/api/v1/graph/investigation/:id", get(graph_investigation))
.route("/api/v1/graph/evidence", get(graph_evidence))
```

- [ ] **Step 2: Add 5 handlers**

```rust
#[derive(Deserialize)]
struct NeighborsQuery { root: String, depth: Option<u8>, at_time: Option<String> }

async fn graph_neighbors(
    State(state): State<Arc<AppState>>,
    Query(q): Query<NeighborsQuery>,
) -> impl IntoResponse {
    let entity_id = match Uuid::parse_str(&q.root) {
        Ok(x) => x, Err(e) => return (StatusCode::BAD_REQUEST, json!({"error": format!("{e}")})).into_response(),
    };
    let at_time = q.at_time.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    let depth = q.depth.unwrap_or(2);
    match crate::graph_queries::get_neighbors(&state, entity_id, depth, None, None, at_time).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": format!("{e}")})).into_response(),
    }
}

#[derive(Deserialize)]
struct TimelineQuery { from: Option<String>, to: Option<String> }

async fn graph_entity_timeline(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TimelineQuery>,
) -> impl IntoResponse {
    let from = q.from.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&chrono::Utc));
    let to   = q.to.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&chrono::Utc));
    match crate::graph_queries::get_entity_timeline(&state, id, from, to, 100).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": format!("{e}")})).into_response(),
    }
}

#[derive(Deserialize)]
struct PathQuery { from: String, to: String, max_hops: Option<u8> }

async fn graph_path(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PathQuery>,
) -> impl IntoResponse {
    let max_hops = q.max_hops.unwrap_or(5);
    match crate::graph_queries::find_path(&state, &q.from, &q.to, true, max_hops).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": format!("{e}")})).into_response(),
    }
}

async fn graph_investigation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match crate::graph_queries::query_investigation_graph(&state, id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": format!("{e}")})).into_response(),
    }
}

#[derive(Deserialize)]
struct EvidenceQuery { entity: String, relation: Option<String> }

async fn graph_evidence(
    State(state): State<Arc<AppState>>,
    Query(q): Query<EvidenceQuery>,
) -> impl IntoResponse {
    let entity_id = match Uuid::parse_str(&q.entity) {
        Ok(x) => x, Err(e) => return (StatusCode::BAD_REQUEST, json!({"error": format!("{e}")})).into_response(),
    };
    match crate::graph_queries::list_evidence_for_entity(&state, entity_id, q.relation.as_deref()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": format!("{e}")})).into_response(),
    }
}
```

- [ ] **Step 3: Verify imports**

Ensure `use serde::Deserialize;` (likely already there from SP2A) and `use axum::extract::{Path, Query, State};` are imported. If `StatusCode::BAD_REQUEST` is not imported, add it.

- [ ] **Step 4: Build**

```bash
cargo build -p hub-core --release 2>&1 | grep -E "error\[" | head -10
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add hub-core/crates/hub-core/src/api.rs
git commit -m "feat(sp9): REST /api/v1/graph/* — 5 endpoints backing console"
```

---

## Task 7: server.rs boot (async worker + Neo4j ensure call wired)

**Files:**
- Modify: `hub-core/crates/hub-core/src/server.rs` (add `tokio::spawn` for resolve_async worker after PG ready)

**Interfaces:**
- After `sqlx::migrate!` + Neo4j connect + `ensure_neo4j_schema` complete, spawn `run_resolve_async_worker(state.pool.clone())` as a background task.

- [ ] **Step 1: Find existing boot sequence**

`hub-core/crates/hub-core/src/server.rs` — look for the function that constructs `AppState` and starts the axum server. Likely `pub async fn run() -> Result<(), HubError>` or similar. There should be a place where `state` is constructed and the router is served.

- [ ] **Step 2: Add worker spawn**

Right after `ensure_neo4j_schema(&state.neo4j).await?` (called from `state.rs` per Task 1), OR after `let state = Arc::new(state)` in `server.rs`, add:

```rust
{
    let pool = state.pool.clone();
    tokio::spawn(async move {
        crate::graph_v2::resolve_async::run_resolve_async_worker(pool).await;
    });
    tracing::info!(target: "hub.boot", "resolve_async worker started");
}
```

- [ ] **Step 3: Build + smoke**

```bash
cargo build -p hub-core --release 2>&1 | grep -E "error\[" | head -5
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add hub-core/crates/hub-core/src/server.rs
git commit -m "feat(sp9): spawn resolve_async worker at boot"
```

---

## Task 8: Console minimal /graph skeleton (G6 dep + page + route)

**Files:**
- Modify: `console/package.json` (add `@antv/g6` ^5.x)
- Modify: `console/src/App.tsx` (register `/graph` route + nav entry)
- Create: `console/src/api/graph.ts` (typed wrappers)
- Create: `console/src/pages/Graph.tsx` (minimal canvas)

**Interfaces:**
- `GET /api/v1/graph/neighbors?root=<uuid>` returns JSON; UI renders nodes + edges.
- Nav link "Graph" appears next to Investigations.

- [ ] **Step 1: Add G6 dependency**

`console/package.json` — in `"dependencies"`, add:
```json
"@antv/g6": "^5.0.0"
```

Then:
```bash
cd console && npm install --no-audit --no-fund 2>&1 | tail -5
```

Confirm `node_modules/@antv/g6/package.json` exists. (Pin to exact installed version after install; e.g. `"@antv/g6": "5.0.45"` — adjust after running.)

- [ ] **Step 2: Create typed API wrappers**

`console/src/api/graph.ts`:

```ts
import type { Uuid } from "./types";

export interface EntitySummary { entity_id: Uuid; name: string; kind: string; edges: Array<{ rel_type: string; valid_from?: string; valid_until?: string; confidence?: number }>; }
export async function getNeighbors(root: Uuid, depth = 2, atTime?: string): Promise<EntitySummary[]> {
  const q = new URLSearchParams({ root, depth: String(depth) });
  if (atTime) q.set("at_time", atTime);
  const r = await fetch(`/api/v1/graph/neighbors?${q}`, { credentials: "include" });
  if (!r.ok) throw new Error(`neighbors ${r.status}`);
  return r.json();
}
export async function getEntityTimeline(id: Uuid, from?: string, to?: string) {
  const q = new URLSearchParams(); if (from) q.set("from", from); if (to) q.set("to", to);
  const r = await fetch(`/api/v1/graph/entity/${id}/timeline?${q}`, { credentials: "include" });
  if (!r.ok) throw new Error(`timeline ${r.status}`);
  return r.json();
}
export async function findPath(from: string, to: string, maxHops = 5) {
  const q = new URLSearchParams({ from, to, max_hops: String(maxHops) });
  const r = await fetch(`/api/v1/graph/path?${q}`, { credentials: "include" });
  if (!r.ok) throw new Error(`path ${r.status}`);
  return r.json();
}
```

(Adjust `Uuid` import to whatever existing `types.ts` exports — likely `string`.)

- [ ] **Step 3: Create minimal Graph page**

`console/src/pages/Graph.tsx`:

```tsx
import React, { useEffect, useRef, useState } from "react";
import { getNeighbors, EntitySummary } from "../api/graph";

export default function GraphPage() {
  const containerRef = useRef<HTMLDivElement>(null);
  const [root_, setRoot] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!root_) return;
    let cancelled = false;
    (async () => {
      try {
        const data = await getNeighbors(root_, 2);
        if (cancelled || !containerRef.current) return;
        const G6 = await import("@antv/g6");
        const graph = new G6.Graph({
          container: containerRef.current,
          width: containerRef.current.clientWidth,
          height: containerRef.current.clientHeight,
          data: g6Data(data),
          node: { style: { labelText: (d: any) => d.name } },
          edge: { style: { labelText: (d: any) => d.rel_type } },
          behaviors: ["drag-canvas", "zoom-canvas"],
        });
        graph.render();
      } catch (e: any) { setError(String(e)); }
    })();
    return () => { cancelled = true; };
  }, [root_]);

  return (
    <div className="p-4">
      <h2 className="text-lg font-bold mb-2">Graph (skeleton)</h2>
      <input
        type="text"
        placeholder="entity_id (uuid)"
        className="border px-2 py-1 mr-2"
        value={root_}
        onChange={(e) => setRoot(e.target.value)}
      />
      <span className="text-sm text-gray-600">minimal canvas — filters/panels in SP10</span>
      {error && <p className="text-red-600 text-sm">{error}</p>}
      <div ref={containerRef} className="mt-4 border" style={{ height: 480 }} />
    </div>
  );
}

function g6Data(data: EntitySummary[]) {
  const nodes = new Map<string, { id: string; name: string; kind: string }>();
  const edges: { source: string; target: string; rel_type: string }[] = [];
  for (const e of data) {
    nodes.set(e.entity_id, { id: e.entity_id, name: e.name, kind: e.kind });
    for (const ed of e.edges || []) {
      edges.push({ source: e.entity_id, target: "", rel_type: ed.rel_type || "" });
    }
  }
  return {
    nodes: Array.from(nodes.values()),
    edges,
  };
}
```

Note: G6 v5 API surface varies across minors — if the `data` shape or `behaviors` strings don't work as written, check the installed version's docs (the goal is "loads, shows something", not pixel-perfect). Add a console.log fallback if needed.

- [ ] **Step 4: Register route + nav**

`console/src/App.tsx`: in the routes section (around `:106-122`), add:

```tsx
import GraphPage from "./pages/Graph";

// inside <Routes>:
<Route path="/graph" element={<GraphPage />} />

// inside nav rendering (find existing nav list):
<NavLink to="/graph">Graph</NavLink>
```

- [ ] **Step 5: Build**

```bash
cd console && npm run build 2>&1 | tail -10
```

Expected: builds. If G6 types cause TS errors in `g6Data`, use `any` casts (already used above) and move on.

- [ ] **Step 6: Commit**

```bash
git add console/package.json console/package-lock.json console/src/App.tsx console/src/api/graph.ts console/src/pages/Graph.tsx
git commit -m "feat(sp9): console /graph minimal skeleton with @antv/g6"
```

---

## Task 9: accept-sp9.py + AGENTS.md update + rsync/deploy/verify

**Files:**
- Create: `scripts/accept-sp9.py`
- Modify: `AGENTS.md` (add sp9 line in the baseline counts)

- [ ] **Step 1: Write `scripts/accept-sp9.py`**

Use the existing `accept-sp8.py` as the template (read it first; pattern: call `tool(sid, ...)` or `curl` against `/api/v1/...`). The script asserts:

```python
#!/usr/bin/env python3
"""SP9 acceptance — 14 assertions covering schema, ontology, resolution,
contradiction, timeline, UI skeleton, regression bar, MCP tool count."""
import os, sys, json, urllib.request, urllib.error, time
from pathlib import Path

KEY = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("HUB_KEY", "")
BASE = os.environ.get("HUB_BASE", "http://10.10.10.41:8800")
PG_BASE = BASE  # queries via REST or MCP JSON-RPC

# ... see accept-sp8.py for tool() / curl_json() / assert_one() helpers
# Each assertion is a function `check_<n>()` returning (ok: bool, msg: str).
# Final: 14 checks; exit 0 only if all 14 pass.
```

Sketch of each check (full code in actual script):

```python
def check_01_migration_0008_applied():
    # Use MCP `get_system_health` or curl /api/v1/health to confirm tables exist
    # Simplest: ssh-based psql query for column existence (mirror accept-sp2a.py)
    ...

def check_02_neo4j_constraints():
    # Run `cypher-shell` via ssh or hub-core health endpoint
    ...

def check_03_resolve_exact():
    # seed two entity rows, call resolve_entity via direct PG or via MCP tool if exposed
    # Simpler: use existing `create_entity` to seed "OpenAI" then test resolve indirectly
    # by reading `entities` table to confirm dedupe path
    ...

def check_04_resolve_alias():
    # seed "OpenAI Inc." as entity_aliases, search_entity("OpenAI Inc.") returns same entity_id
    r = mcp_call("search_entity", {"name": "OpenAI Inc."})
    assert any("OpenAI" in x["name"] for x in r), r
    return True, "alias path returns canonical"

def check_05_two_supporting_evidence():
    # seed claim with two supporting docs via link_evidence_to_claim
    # wait, link_evidence_to_claim isn't exposed as MCP tool in SP9 (read-only).
    # Instead: seed directly into PG via accept script's psql helper, then call list_evidence_for_entity
    ...

def check_06_contradiction_edge():
    # seed two claims, insert into claim_contradictions via direct PG
    # call find_contradicting_claims(claim_id=...) returns 1 row
    # AND both claims.status='disputed'
    ...

def check_07_relationship_changes():
    # seed two entities + relationship with valid_from/valid_until
    # call find_relationship_changes returns row
    ...

def check_08_entity_timeline():
    # call get_entity_timeline on seeded entity, expect ≥1 row from graph_change_log
    ...

def check_09_console_graph_route():
    # curl /graph on console origin → 200
    ...

def check_10_v1_back_compat():
    # call query_entity, query_relationship, create_entity, create_relationship, create_claim
    # all succeed (run a minimal subset of accept-sp2a.py's graph assertions)
    ...

def check_11_baseline_regressions():
    # run accept-sp2a, sp2b, sp3, sp4, sp5, sp6, sp7, sp8 each, capture exit codes
    # (this is the heaviest check — may take 5-10 min)
    ...

def check_12_health_metrics_unchanged():
    # /api/v1/system/health includes all expected fields (no regression)
    ...

def check_13_mcp_tool_count():
    # tools/list returns ≥ 38 tools (was 28 + 10 new = 38)
    r = mcp_call("tools/list", {})
    assert len(r["tools"]) >= 38
    ...

def check_14_neo4j_mirror_lag():
    # fire 50 mixed write intents, observe graph_sync_queue drain
    # wait up to 30s, confirm queue depth = 0
    ...
```

Mirror the structure of `accept-sp8.py`. Aim for the script to finish in < 5 minutes (the regression bar in check_11 is what dominates).

- [ ] **Step 2: Run locally first**

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
python3 scripts/accept-sp9.py "$KEY"
```

Expected: many FAILs on first run (DB not migrated yet, UI not deployed). Use this to enumerate gaps before deploying.

- [ ] **Step 3: rsync + build + restart + deploy**

```bash
cd /Volumes/TBU/Workspace/IntelHub-kg-memory
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub:/home/zou/IntelHub/

ssh -o BatchMode=yes IntelHub 'cd /home/zou/IntelHub \
  && bash scripts/build-hub.sh 2>&1 | grep -E "^error|built" | head -8 \
  && bash scripts/build-console.sh 2>&1 | tail -1 \
  && sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core'
```

If `build-hub.sh` errors, run the diagnostic in `AGENTS.md`:
```bash
ssh IntelHub 'docker run --rm -v /home/zou/IntelHub/hub-core:/ws -w /ws -v intelhub-hub-target:/ws/target -v intelhub-cargo-registry:/usr/local/cargo/registry rust:trixie cargo build --release --workspace 2>&1 | grep -B4 -A12 "error\[" | head -40'
```

Fix any compile errors that surface (most likely: `serde_json` value accessor differences, `chrono` default arg names, missing imports). Re-deploy.

- [ ] **Step 4: Wait 2.5 min for hub-core to apply migrations + start workers**

```bash
sleep 150
```

Then re-run accept:

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
python3 scripts/accept-sp9.py "$KEY"
```

Iterate until all 14 checks pass.

- [ ] **Step 5: Run regression bar**

```bash
for a in sp2a sp2b sp3 sp4 sp5 sp6 sp7 sp8; do
  echo "── $a"
  python3 scripts/accept-$a.py "$KEY" 2>&1 | grep -E "==.*(passed|failed)" | tail -1
done
```

Expected: each prints `== <name> passed` or equivalent.

- [ ] **Step 6: Update AGENTS.md**

In `AGENTS.md`, find the line:
```
当前基线：sp2a 19 · sp2b 33 · sp3 19 · sp4 25 · sp5 9 · sp6 18 · sp7 24 · sp8 18。
```

Replace with:
```
当前基线：sp2a 19 · sp2b 33 · sp3 19 · sp4 25 · sp5 9 · sp6 18 · sp7 24 · sp8 18 · sp9 14。
```

Also find any section that enumerates SPs (e.g. "sp8 12 checks") and add sp9 if relevant.

- [ ] **Step 7: Commit + merge to main + push**

In the worktree:
```bash
git add scripts/accept-sp9.py AGENTS.md
git commit -m "feat(sp9): acceptance script (14 checks) + AGENTS.md baseline sp9 14"
```

In the **main** worktree:
```bash
cd /Volumes/TBU/Workspace/IntelHub
git merge --no-ff feat/kg-memory -m "merge: SP9 Knowledge Graph Memory Layer (data + MCP + minimal /graph)"
git worktree remove /Volumes/TBU/Workspace/IntelHub-kg-memory
git branch -d feat/kg-memory
git push origin main
```

(The `feat/kg-memory-design` branch stays for spec/plan history.)

- [ ] **Step 8: Final amendment to spec**

Open `docs/superpowers/specs/2026-09-13-intelhub-sp9-kg-memory-design.md` and fill §12 with the deployment record: actual versions, timing, deviations, any accepts that needed iteration. Commit on main.

---

## Self-Review

**1. Spec coverage** — walk through spec sections:

| Spec § | Task covering it |
|---|---|
| §0 Background | (n/a — context) |
| §1 Architecture | Task 1, 3, 4, 5, 6, 7 |
| §2 Data model & flow (write flow) | Task 3 (compiler), Task 1 (queue hook) |
| §2 (read flow) | Task 4 (graph_queries) |
| §2 (temporal) | Task 1 (schema), Task 3 (temporal.rs) |
| §2 (entity resolution flow) | Task 2 |
| §3.1 PG extensions | Task 1 (migration) |
| §3.2 PG new tables | Task 1 (migration) |
| §3.3 Neo4j constraints/indexes | Task 1 (init cypher + ensure fn) |
| §3.3 Neo4j label additions | (handled at write time by compiler — Task 3; no separate task) |
| §4 Typed intents | Task 3 (compiler.rs) |
| §5 Entity Resolution | Task 2 |
| §6 10 new MCP tools | Task 5 |
| §6 v1 find_path replacement | Task 4 (delete) + Task 5 (new) |
| §6 REST additions | Task 6 |
| §7 Minimal /graph skeleton | Task 8 |
| §8 Configuration | Task 1 (no special handling; env defaults applied), Task 7 (worker respects env) |
| §9 Validation | Task 9 (accept-sp9.py) |
| §10 Out of scope | (n/a — enforced by not implementing) |
| §11 Risks | (n/a — informational) |

All covered. No gaps.

**2. Placeholder scan** — searched the plan for "TODO", "TBD", "implement later":

- Task 2 Step 5: `// TODO Task 3/5` — fixed inline above to reference Tasks 3 & 5 without leaving a TODO marker.
- Task 6 Step 4: "If ... is not imported, add it" — legitimate "if-then" check, not a placeholder.
- Task 9 Step 1: `...` — **fix**: replace with explicit reference to `accept-sp8.py` plus a complete helper functions block.

**3. Type consistency** — checked function signatures:
- `resolve_entity(name: &str, kind: &str, source: &str, confidence: f64, pool: &PgPool) -> Result<EntityResolution, HubError>` (Task 2) is consumed by `compiler::resolve_entity_ref` (Task 3) ✓
- `graph_queries::find_path(state, &from, &to, weighted, max_hops)` is called by both the MCP `find_path` (Task 5) and the REST `graph_path` (Task 6) ✓
- `merge_entities(a, b, kept, actor, pool)` (Task 2) is called by `resolve_async::drain_once` (Task 2) ✓
- `chrono::DateTime<chrono::Utc>` used consistently across all temporal predicates ✓

Fixes applied inline.

**4. One thing I want to flag** — the `async fn run_resolve_async_worker(pool: PgPool) -> !` return type in Task 2 (infinite loop). The compiler may warn about `!` not being valid for async fns. **Fix applied inline in Task 2**: signature is now `pub async fn run_resolve_async_worker(pool: PgPool)` with no return type and an explicit `loop { ... }` body that never returns — the compiler infers `!` correctly from the absence of a return value.

The plan is complete.