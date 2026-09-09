# IntelHub Hub Core SP2A「数据与工具平面」— Design Spec

Date: 2026-09-09
Status: Approved by user (2026-09-09)
Parent: OSINT Intelligence Hub 工程实施指令 (§20, §27–29, §32–44, §46–50, §62–66, §72–73)
Predecessor: 2026-09-09-intelhub-substrate-design.md (SP1, deployed)

## 0. Scope

SP2 was decomposed (user-approved) into SP2A + SP2B. This spec covers **SP2A only**.

**In scope:** Rust workspace + single `hub` binary (native, systemd); Postgres canonical schema; ingest/normalization module; MCP Gateway (Streamable HTTP) with read/search + low-risk write tools; Agent Identity (per-agent API keys); audit persistence; Redis Streams event bus + SSE broadcast; Qdrant collection schema (1536-dim, model-named); localhost DB bindings on SP1 stack.

**Out of scope (SP2B):** policy engine (3-level risk), Cost Governor (GREEN/YELLOW/RED/KILL), graph write tools (create_entity/claim/relationship), real embedding pipeline (`semantic_search` falls back to keyword+metadata in SP2A and is labeled as such), Alert Engine, Component Lifecycle API, TLS, UI (SP3).

## 1. Architecture

Single binary `hub`, one process containing:

- HTTP server (axum 0.8): MCP endpoint `POST /mcp`, REST `/api/v1/*`, SSE `/api/v1/events`, `/healthz`
- MCP gateway: rmcp 3.x (official MCP Rust SDK, Streamable HTTP server)
- Ingest worker: internal tokio task consuming Redis Streams queue (§50 "独立逻辑服务" = logical module boundary, not a second process)
- Modules: `config / types / store (sqlx) / auth / events / ingest / sensors / graph (neo4rs) / vector (qdrant REST) / mcp / api`

Code layout (binary + lib, YAGNI):

```
hub-core/
├── Cargo.toml                 # workspace [hub, hub-core]
├── crates/hub/                # binary: wiring, main.rs
└── crates/hub-core/           # library: all modules above
```

Key dependencies (exact versions pinned in Cargo.lock): tokio, axum 0.8, sqlx 0.8 (postgres, rustls), rmcp 3.0 (server, streamable-http), neo4rs 0.8 (Neo4j Labs official driver, bolt, Neo4j 5.x), redis-rs (streams), reqwest 0.12 (rustls), uuid, chrono, sha2, tracing.

## 2. Postgres Schema (sqlx migrate)

| Table | Key points |
|---|---|
| agents / api_keys | keys stored as SHA-256 hashes only, bound to agent_id; never plaintext at rest |
| investigations / tasks | first-class objects (§27/§62), agent-decoupled, created_by provenance |
| sources / documents | documents: `content_hash` UNIQUE (exact dedupe), tsvector full-text, url_canonical, retrieved_at/published_at, `raw_path` (raw capture on disk at `data/raw/<hash>`, DB stays lean), provenance jsonb, lang |
| entities / observations / claims | schema created now; ingest writes observations only; entity/claim writes are SP2B |
| findings / finding_evidence | agent findings (§28/§29): claim text, confidence fields (source/claim/model split per §57), supporting/contradicting evidence links — `create_finding` MCP tool enabled in SP2A (pure PG rows, low risk) |
| agent_runs / tool_calls | every MCP invocation: agent_id, session_id, task_id, request_id, trace_id, arg digest, latency, result ref (§34) |
| audit_records / events | audit trail + persisted bus events |
| embedding_jobs | status=PENDING queue (consumed by SP2B; failure model §72 built in) |
| cost_records | schema only (SP2B writes) |

## 3. Ingest Pipeline (osint-ingest module)

Queue: Redis Stream `hub.ingest`. Stages:

1. URL canonicalization (tracking params stripped, fragment removed, scheme/host normalized)
2. Timestamp normalization (published_at best-effort extraction; retrieved_at authoritative)
3. Encoding normalization (UTF-8)
4. Content hash: SHA-256 over canonical text → exact dedupe via UNIQUE constraint
5. Near-dup: simhash 64-bit, hamming ≤ 3 against recent documents
6. Source attribution + unified Evidence Event envelope (§50): event_id, source, url, retrieved_at, published_at, content_hash, content, metadata, provenance
7. Persist: raw body → `data/raw/<content_hash>`; metadata row → documents
8. Emit `DOCUMENT_INGESTED` to event bus; enqueue `embedding_jobs` row with status=PENDING (decision deferred = zero embedding cost, §40)

## 4. MCP Gateway

rmcp 3.x Streamable HTTP at `POST /mcp`. Bearer auth (per-agent API key) enforced by axum middleware before the rmcp service; unauthorized → 401. Middleware also: per-agent Redis token bucket (basic rate limit; full budget model is SP2B) and authoritative tool_call/audit attribution at the JSON-RPC layer (unspoofable; created_by inside handlers is auxiliary).

Tools (agent-neutral capability verbs, §32):

| Category | Tools |
|---|---|
| Search/crawl | `search_web` (→SearXNG), `crawl_url` (→Crawl4AI→ingest), `fetch_document` |
| Retrieval | `get_evidence`, `get_document`, `keyword_search`, `hybrid_search` (SP2A = keyword+metadata; vector channel schema-reserved), `semantic_search` (SP2A returns explicit `fallback: "keyword"` marker — never disguises keyword results as semantic) |
| Graph (read-only) | `query_entity`, `query_relationship`, `find_path` — typed intents → parameterized templates → Neo4j. No arbitrary Cypher (§38); injection attempts rejected and audited |
| Low-risk writes | `create_investigation`, `update_investigation`, `create_finding` (must bind evidence_ids, §29), `list_investigations` |
| Status | `get_task_status`, `get_system_health` (aggregates SP1 health checks) |

## 5. Event Bus & Audit

Redis Stream `hub.events`; envelope: event_id, type, ts, actor, investigation_id, trace_id, payload. Consumers in-process: Postgres `events` persister + SSE broadcaster. SP2A event subset of §63: TASK_CREATED/STARTED/PROGRESS/COMPLETED, DOCUMENT_INGESTED, FINDING_CREATED, SENSOR_ERROR. SSE at `/api/v1/events` is SP3 Console's future data source.

## 6. REST API (SP3 prerequisite)

`/api/v1`: investigations CRUD, evidence/documents read, search, findings, health. Shares the service layer with MCP tools — no duplicated logic.

## 7. Build & Deployment

- Build: on VM, Docker (pinned `rust:1-trixie` digest, cargo-chef layer caching), musl static binary
- Run: `/home/zou/IntelHub/core/hub`, `hub-core.service`: User=zou, NoNewPrivileges, ProtectSystem=strict, EnvironmentFile=`core/secrets.env` (0600), Restart=always
- Listen: `10.10.10.41:8800` plain HTTP, LAN-only via nftables (TLS + `intel.local` deferred to SP3)
- SP1 change: `compose.data.yml` adds `127.0.0.1:5432/6379/7687/6333` bindings (LAN exposure unchanged, loopback only)
- Secrets: T8star relay key lives only in VM-side `core/secrets.env` (0600, gitignored); configs reference variable names, never values. Embedding endpoint `https://ai.t8star.org/v1`, model `text-embedding-3-small` (verified 1536-dim 2026-09-09); relay also exposes LLM models for SP2B/SP3
- Qdrant collection `evidence__openai-text-embedding-3-small__1536` (empty in SP2A; naming embeds model + dims per §42 so a model swap = new collection, not rebuild)

## 8. Failure Model (SP2A)

- neo4j down → graph tools return clean structured errors; search/crawl/ingest unaffected
- qdrant down → no evidence loss (nothing writes to qdrant in SP2A)
- redis down → ingest queue + events degrade: hub returns 503 on affected tools, keeps serving reads from PG
- Embedding API down → rows stay embedding_jobs.PENDING (§72)
- hub crash → systemd Restart=always; all state externalized (PG/Redis/filesystem)

## 9. Acceptance Criteria

1. systemd-managed `/healthz` OK; invalid API key → 401
2. MCP `initialize` + `tools/list` over Streamable HTTP succeeds
3. `search_web` returns normalized SearXNG results
4. `crawl_url` → document row + content_hash + DOCUMENT_INGESTED event; re-crawl same URL → dedupe hit, no duplicate raw capture
5. Every tool call in tool_calls/audit_records carries agent_id + trace_id; SSE delivers live events
6. Seeded Neo4j data: `query_entity` / `find_path` correct; Cypher-injection attempt rejected + audited
7. Qdrant collection exists with dim 1536
8. Failure drill: neo4j stopped → graph tools error cleanly, search still works; qdrant stopped → no evidence loss
