# IntelHub Hub Core — Sub-project 2B Design (Governance & Write Plane)

**Date:** 2026-09-09
**Status:** Approved → In implementation
**Depends on:** SP2A (deployed, 16/16 acceptance green), SP1 substrate
**Directive:** `OSINTIntelligenceHub.md` §36–44, §53–61, §66–72

## Context

SP2A delivered the data/tool plane: 17 read/search/ingest tools, attribution, audit, event bus. Agents still cannot write the graph, embeddings are queued but never computed (`semantic_search` returns keyword fallback), there is no cost governance (an agent can crawl unboundedly), no alert center, and no lifecycle visibility. SP2B adds the governance layer and the controlled write plane.

## Approved decisions (brainstorm 2026-09-09)

1. **Hub stays LLM-free** (user: option A). Entities/claims/relationships are created exclusively by external agents via typed write tools; hub does schema validation + parameterized writes + audit. Embedding is hub's only cloud call. No auto-extraction.
2. **Alerts: in-system loop + generic webhook** (user: option B). `alerts` table + bus events + SSE + REST/MCP actions; severity-filtered config-driven webhook dispatcher with retry + delivery audit. No per-channel code.
3. **Component lifecycle: read plane + controlled trigger** (user: option A). Hub aggregates status/versions/update-available and exposes whitelisted `upgrade|rollback|backup` script execution as Level 3 actions. No Docker-socket orchestration.
4. **Governance architecture: registry-driven single enforcement point** (recommended approach). Every tool statically declared with `{risk_level, cost_class}`; one wrapper does pre-flight (policy + budget) → execute → post-flight (cost + audit + alert triggers). No tool can bypass governance.

## Architecture

### §1 Policy Engine (directive §61)

`policy.rs` — static tool registry (match on tool name):

- **Level 1 (auto):** all SP2A read/search/investigation tools.
- **Level 2 (schema validation):** `crawl_url`/`fetch_document` (URL scheme + length), `create_entity`/`create_claim`/`create_relationship` (kind enum, NFC name normalization, length caps, property allowlist).
- **Level 3 (default DENY):** `run_component_action`; requires `X-Admin-Token` header matching `HUB_ADMIN_TOKEN` (secrets.env, constant-time compare) in addition to a valid agent key.
- Denials → structured `policy_denied` error + `audit_records` row + security alert. Cost accounting attaches a `cost_class` per tool (free | search | crawl | embed | write).

### §2 Cost Governor (directive §58–60)

`cost.rs` — post-flight writes to `cost_records` (existing table; kinds: tool_call | http | crawl_page | embedding_tokens). Budgets from env (defaults):

| Budget | Default |
|---|---|
| Per-agent daily tool calls | 2,000 |
| Per-agent daily crawl pages | 300 |
| Per-agent daily embedding tokens | 1M |
| Global daily crawl pages | 1,000 |
| Global daily embedding tokens | 5M |

State machine: **GREEN** <70% · **YELLOW** 70–90% (crawl limits halved, embedding agent-requested-only) · **RED** 90–100% (read-only: search/crawl/embed denied, get/list allowed) · **KILL** ≥100% (everything read-only + running tasks terminated). Per-agent isolation (§60); global limits apply across agents. Usage computed from `cost_records` (today, UTC) cached in Redis 30s. State transitions → `BUDGET_WARNING`/`BUDGET_EXCEEDED` bus events + budget alerts. Every pre-flight response embeds `budget_state` so agents can self-throttle.

### §3 Graph write tools (directive §38)

`graphw.rs` — `create_entity` / `create_claim` / `create_relationship`:
Agent → Typed Intent → schema validation (kind enum: person|org|domain|ip|location|event|infrastructure|software|handle|email|phone|crypto_wallet; property allowlist) → Level 2 authorization → **PG canonical write** (§36: entities/claims/relationships tables) → **Neo4j parameterized MERGE** (idempotent, §38) → audit + `GRAPH_ENTITY_CREATED` etc. events. If Neo4j is down: write to `graph_sync_queue` (§72 queue requirement) — replay worker retries with backoff. Existing PG entities/observations syncable into graph.

### §4 Embedding pipeline (directive §40–42)

`embed.rs` — worker consumes `embedding_jobs` (PENDING → RUNNING via `FOR UPDATE SKIP LOCKED`):

- **Decision rules (§40):** word_count ≥ 300, no simhash near-dup (hamming ≤ 3 against DONE docs), not a duplicate version. Violations → job SKIPPED with reason. `crawl_url` accepts `embed: "auto"|"force"|"skip"` (force embeds even <300 words).
- **Chunking:** 800-token windows, 80 overlap, `chunk_algorithm_version = "fixed-800-80-v1"` (tokens ≈ chars/4).
- **Cache key (§42):** content_hash + chunk_version + model — chunks recorded in `embedding_chunks` (UNIQUE on document+ix+version+model); identical content never re-embedded.
- **Execution:** T8star `text-embedding-3-small` (batch ≤ 16 chunks/req) → Qdrant upsert (payload: document_id, chunk_ix) → job DONE + `embedding_tokens` cost record. Failures: exponential backoff ×3 → FAILED + alert (PENDING preserved per §72).
- **`semantic_search` upgraded:** embed query (24h Redis cache) → Qdrant top-20 → join PG documents → `mode: "vector"` results.
- **`hybrid_search` upgraded:** keyword + vector channels fused via RRF (k=60) (§41 candidate set → rerank → evidence set).

### §5 Alert Engine (directive §54)

`alerts.rs` — `alerts` table: severity (info|warning|critical), source (osint|agent|sensor|infra|budget|security), links (task/investigation/entity/evidence), recommended_action, status (open|ack|muted), dedupe_key (open alert with same key <1h → update, not insert). **Triggers:** budget transitions (§2), sensor health flaps (periodic probe watcher), ingest failure bursts, security events (Level 3 denials, auth-failure bursts, injection probes), component update-available (§6). **Actions:** `list_alerts` / `acknowledge_alert` / `mute_alert` (MCP + REST). **Webhook dispatcher:** POST `HUB_ALERT_WEBHOOK_URL` for severity ≥ `HUB_ALERT_WEBHOOK_MIN_SEVERITY` (default warning), 3 retries with backoff, every attempt → `alert_deliveries`.

### §6 Component lifecycle read plane (directive §68–69)

`components.rs` — `GET /api/v1/components`: manifests (read from `HUB_MANIFESTS_DIR`) + `docker ps` live state + installed image version + latest-known version (Docker Hub / GHCR tag query per manifest channel, Redis-cached 6h) → `update_available` flag (→ info alert on flip). `run_component_action` (Level 3 MCP tool): whitelist = actions {upgrade, rollback, backup} × components {postgres, redis, neo4j, qdrant, searxng, crawl4ai, spiderfoot, huginn} → executes the corresponding pinned script under `HUB_SCRIPTS_DIR` (no arbitrary args — full command enumerated in code) → output + result to audit + alert.

### §7 Explicitly out of scope

Delete/bulk-modification tools; inter-agent messaging; auto entity extraction (decision 1); channel-native alert integrations; full upgrade orchestration (decision 3); Crucix/Grafana (SP4); UI (SP3).

## Schema additions (migration 0002)

`alerts`, `alert_deliveries`, `graph_sync_queue`, `embedding_chunks`, `relationships` (canonical relationship store; graph = relationship memory). Config additions: `HUB_ADMIN_TOKEN`, `HUB_BUDGET_*` (5 vars), `HUB_ALERT_WEBHOOK_URL`, `HUB_ALERT_WEBHOOK_MIN_SEVERITY`, `HUB_MANIFESTS_DIR`, `HUB_SCRIPTS_DIR`.

## Acceptance criteria

1. Level 3 without admin token → `policy_denied` + audit + security alert; with token → whitelisted script runs.
2. create_entity/claim/relationship → visible in PG + Neo4j; Neo4j down → queued, auto-replayed on recovery.
3. Budget state machine: injected costs push one agent over 70%/90%/100% → YELLOW throttling, RED read-only, KILL denial; other agents unaffected; transitions emit events + alerts.
4. New qualifying crawl embedded within a minute; `semantic_search` returns `mode:"vector"`; identical content never re-embedded (§42 cache proof).
5. Sensor failure → ALERT_RAISED event + alerts row + webhook delivery (test endpoint) + acknowledge works.
6. `/api/v1/components` returns all components with installed/latest/update_available.
7. SP2A regression: `scripts/accept-sp2a.py` 16/16 green.

---

## Amendment 2026-09-09 — Deployment Record (as-built)

Deployed and acceptance-tested 2026-09-09. **SP2B acceptance 32/32 + budget drill 12/12 + SP2A regression 16/16, all green.**

As-built facts:

- 25 MCP tools (17 SP2A gated + 8 new), every one passing `policy::preflight` (§61 levels) + `cost::budget_state` (§60) — single enforcement point in `HubMcp::gate()`; post-flight metering in `record()` (tool_call) + crawl (crawl_page) + embed worker/query (embedding_tokens)
- Migration 0002 applied: alerts, alert_deliveries, graph_sync_queue, relationships, claim_entities, claim_evidence, embedding_chunks; embedding_jobs gained force/attempts
- Workers in-process: embedding, graph-sync replay (15s), alert webhook dispatcher (5s, 3 retries), sensor flap watcher (30s), component update watcher (1h)
- HUB_ADMIN_TOKEN generated (VM core/admin-token.txt 0600); X-Admin-Token → AgentIdentity.admin flag (token never logged); L3 backup verified end-to-end (real restore point written)
- Budget drill (budget=10): GREEN→YELLOW(0.70)→RED(0.90)→KILL(1.20) all enforced; RED denied search_web/allowed keyword_search; KILL denied writes/allowed status reads; aleph unaffected while claude-code KILL (§60 isolation); BUDGET_WARNING/EXCEEDED events + 3 budget alerts
- Embedding: Wikipedia OSINT article → 24 chunks → DONE; semantic_search mode:"vector" with top-self hit; hybrid mode:"hybrid-rrf"; §42 cache proof: re-enqueued job → "24 reused from §42 cache, 0 tokens" (20337→20337 unchanged)
- Alert loop: searxng stop → critical alert → webhook DELIVERED (test sink :18899) → ack; security alert on L3 denial; sensor recovery → info alert
- /api/v1/components: 8 components with installed/latest (Docker Hub, 6h cache); update_available flags
- Webhook sink was a test artifact (/tmp/webhook_sink.py on VM, not retained); HUB_ALERT_WEBHOOK_URL left configured — point it at a real endpoint when desired

Deviations discovered during implementation:

10. **systemd ProtectSystem=strict blocks L3 scripts**: backup/upgrade write to backups/ and compose/.env → added both to ReadWritePaths (config/hub-core.service).
11. Entity name normalization is trim+whitespace-collapse (no NFC) — avoids a new dependency; documented.
12. Neo4j relationship types cannot be Cypher parameters — interpolated ONLY after whitelist validation (REL_TYPES enum); injection-impossible by construction.
13. accept-sp2a.py updated: tools 17→25, semantic_search mode assertion now accepts "vector" (SP2B behavior is the new correct baseline).
14. Embedding cost attribution: via parent_task → agent when known, else global-only (agent_id NULL).
