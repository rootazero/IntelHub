# IntelHub Unified Console — Sub-project 3 Design

**Date:** 2026-09-09
**Status:** Approved → In implementation
**Depends on:** SP2A+SP2B (deployed, all acceptance green), SP1 substrate
**Directive:** `OSINTIntelligenceHub.md` §24–29, §53–57, §63, §66–69, §72, §74–76

## Context

SP2B completed the governed write plane. Everything the console needs exists as MCP tools + a partial REST surface, but there is no human interface — the directive's central product requirement (§78: "把搜索、传感器、Evidence、Graph、Semantic Memory、Agent Findings、实时告警和系统状态统一到一个工作空间"). SP3 delivers the Unified Console: a TS/React SPA served by hub-core itself, talking ONLY to the Hub API (§24).

## Approved decisions (brainstorm 2026-09-09)

1. **Stack & delivery: React 19 + Vite + TypeScript + Tailwind v4, static build served by hub-core** (user: A). dist at `/home/zou/IntelHub/console/dist`, same-origin with the API (zero CORS), no new component, frontend updates via rsync without hub restart, static outage cannot affect `/mcp` (§72).
2. **Human auth: dedicated `console` agent key** (user: A), pasted once into the browser, held in localStorage, sent as Bearer. Console actions audit as `agent:console` (§66); key individually revocable.
3. **Scope: full intelligence work surface in one pass** (user: A) — Overview (§25), Investigation Workspace (§27–29), Evidence browser, Unified Search (§67), Alert Center (§54), Agent Activity (§55/56), Audit (§66), Health (§68), Component Manager (§69 read + L3 buttons). **Excluded:** Radar map (§26 → SP4 Crucix), Sensor start/stop (§53 control → later), Grafana (SP4).

## Architecture

### §1 Delivery

- `console/` workspace in repo; deps fully locked (package-lock committed).
- `scripts/build-console.sh`: docker `node:22-trixie` on the VM runs `npm ci && vite build` (same pattern as build-hub.sh; no node needed on Mac).
- hub-core static service: `GET /` → index.html, `/assets/*` immutable cache, SPA fallback to index.html. Static paths skip auth (shell holds no data); API unchanged (bearer). `HUB_CONSOLE_DIR` env (default `/home/zou/IntelHub/console/dist`).

### §2 Hub Console API (new read-only aggregates, L1)

| Endpoint | Purpose |
|---|---|
| `GET /api/v1/overview` | §25 single-request home aggregate: health summary, active investigations, recent alerts, sensor status, per-agent calls today, evidence ingested today, graph changes (entities/relationships today), semantic memory (chunks/embedded docs), cloud usage (embedding tokens + est. cost), per-agent budget states |
| `GET /api/v1/search/unified?q=` | §67 grouped results: exact entities, documents (keyword + vector RRF), graph entities/relationships, findings, alerts, investigations |
| `GET /api/v1/documents` | Document list (q/source/status/time/limit+offset) |
| `GET /api/v1/entities`, `GET /api/v1/entities/{id}` | Entity browse + relationships (PG canonical + graph) |
| `GET /api/v1/agents/activity` | §55 per-agent timeline: tool_calls aggregates, recent calls, costs, budget |
| `GET /api/v1/audit` | §66 filterable audit query (actor/action/result/time) |
| `GET /api/v1/tasks` | Task list (status filter) |
| `GET /api/v1/investigations/{id}/workspace` | §27 single-request aggregate: investigation + findings (with evidence chains) + documents + entities + tasks + alerts + audit |

### §3 Pages (information hierarchy §76)

1. **Overview** (§25): all panels + live event stream (SSE) + Radar placeholder ("activates in SP4")
2. **Investigations**: list + Workspace — overview/hypothesis, findings with expandable evidence chains down to original documents (§28/29), FACT/OBSERVATION/AGENT CLAIM/HYPOTHESIS/ALERT visually distinct (§29), entities, timeline, alerts, audit
3. **Search** (§67): single box → grouped results with click-through to Evidence/Entity/Investigation
4. **Evidence**: document list + detail (provenance Source→Retrieved→Hash + reverse references findings/claims)
5. **Alerts** (§54): severity/source filters, ack/mute, "create investigation from alert", recommended actions
6. **Agents** (§55/56): activity timeline, budget state bars, finding stats (supporting/contradicting/independent sources)
7. **Audit** (§66): filterable table
8. **System**: Health Center (§68) + Component Manager (§69 update_available flags; L3 buttons prompt for admin token per-use) + sensor status (read-only)
9. Visual: §75 information-dense, signal-first, dark ops-console, no vanity charts

### §4 Explicitly out of scope

Radar map (§26 → SP4), sensor start/stop control (§53), user management/roles, Grafana (SP4), any iframe integration (§24).

## Acceptance criteria

1. `GET /` serves the console, `/assets/*` load, SPA routes survive refresh; static failure cannot affect `/mcp`
2. All 8 new Console API endpoints return 200 + correct shapes with console key; 401 without
3. Every page's backing endpoint verified script-wise
4. SSE events reach the stream live (event manufactured → visible)
5. Component Manager L3 button without admin token → `policy_denied`
6. Regressions: accept-sp2a 16/16 + accept-sp2b 32/32 green
7. package-lock committed; clean-checkout build reproduces
