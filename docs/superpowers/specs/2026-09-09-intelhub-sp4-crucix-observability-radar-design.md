# SP4 Design — Crucix Signal Layer + Observability Stack + Global Radar Activation

**Date:** 2026-09-09
**Status:** Approved by user (all recommended options + hybrid basemap amendment), implementation authorized
**Directive anchors:** §13/§16 (mgmt-net, egress-net), §17 (port exposure), §19 (crucix/grafana/node-exporter/cAdvisor), §24 (no iframe integration), §26 (Global Radar), §45/§51 (Crucix as macro signal layer — aggregate, don't replicate), §52 (Grafana + hub-aggregated metrics), §53 (sensor control center), §68/§69 (health center + component manager), §71 (resource limits), §72 (failure model)

## 0. Scope

Sub-project 4 of the OSINT Intelligence Hub. Three workstreams:

1. **Crucix deployment + hub integration**: Docker-deploy Crucix (calesthio/Crucix, AGPL-3.0, 27 OSINT sources), poll its `/api/data` into a hub-owned `geo_events` table, convert high-tier alerts into hub alerts, bridge sweep events onto the hub bus.
2. **Observability stack**: Prometheus + node-exporter + cAdvisor + Grafana (pinned images, provisioned datasource/dashboard); hub aggregates §52 metrics via the Prometheus HTTP API.
3. **Console Radar activation**: new Radar page (Leaflet, dual-mode basemap), Overview radar card goes live, System page gains §52 metrics.

## 1. Approved decisions

| # | Question | Decision |
|---|---|---|
| Q1 | Crucix source keys | **A** — enable all no-key sources now; optional keys (FRED, FIRMS, EIA, AISSTREAM, ACLED, CLOUDFLARE) get env slots in `compose/.env`, user fills later |
| Q2 | Crucix → Hub data path | **A** — hub `crucix-sync` worker polls and normalizes into `geo_events` (auditable, joinable, alertable); sweep events bridged to hub bus |
| Q3 | Radar basemap | **Hybrid (user amendment)** — online CARTO `dark_all` tiles by default (egress via fake-ip proxy); on tile load failure/timeout auto-fallback to bundled Natural Earth 110m coastline GeoJSON vector layer; `TILES: ONLINE/OFFLINE` badge + manual toggle |
| Q4 | Observability | **A** — Prometheus + node-exporter + cAdvisor + Grafana; hub System page aggregates §52 metrics from Prometheus API; Grafana native UI is a link, never an iframe (§24) |
| Q4b | Crucix LLM layer | **Enabled via build-time patch** — Crucix hardcodes `https://api.openai.com/v1` (verified in `lib/llm/openai.mjs`, no base-URL env). `build-crucix.sh` sed-patches it to the T8star relay at image build (deterministic per pinned SHA, `CRUCIX_LLM_PATCH=1` default, cheap model `gpt-4.1-mini`). Hub core remains LLM-free. |

## 2. Architecture

```
                egress-net (outbound, fake-ip transparent proxy)
                     │
   ┌─────────────────┼────────────────────┐
   ▼                 ▼                    ▼
┌─────────┐   ┌────────────┐       ┌───────────┐
│ Crucix  │   │ Prometheus │       │  Grafana  │   mgmt-net
│ :3117   │   │   :9090    │◄──────│   :3000   │
└────┬────┘   └─────┬──────┘       └───────────┘
     │ /api/data    │ scrape: node-exporter :9100, cAdvisor :8080, (hub /metrics if added later)
     │ /api/health  ▼
     ▼
hub-core crucix-sync worker ──► geo_events (PG) ──► GET /api/v1/radar/events
     │                                                    │
     ├─► hub alerts (FLASH/PRIORITY conversion)            ▼
     └─► hub event bus (sweep lifecycle)          Console Radar page (Leaflet dual-basemap)
```

## 3. Components (all pinned, all in Component Manager §69)

| Component | Image / build | Networks | Resources (§71) | LAN port |
|---|---|---|---|---|
| crucix | **source build, pinned git SHA `3db7068`** (no release tags exist), local tag `crucix:sha-3db7068` | mgmt + egress | 1G / 1.0 cpu | 3117 (native UI, diagnostic entry like searxng:8888) |
| prometheus | prom/prometheus (pinned via resolve-versions) | mgmt | 1G / 0.5 | — (internal only) |
| node-exporter | prom/node-exporter (pinned) | mgmt | 128M / 0.2 | — |
| cadvisor | gcr.io/cadvisor/cadvisor (pinned) | mgmt | 256M / 0.3 | — |
| grafana | grafana/grafana (pinned) | mgmt | 512M / 0.5 | 3001 |

- `config/prometheus/prometheus.yml`: scrape node-exporter, cAdvisor, prometheus itself; 15s interval, 15d retention.
- Grafana provisioning (`config/grafana/provisioning/`): Prometheus datasource + one "IntelHub Overview" dashboard (CPU/RAM/disk/network + container health).
- Upgrade paths: 4 standard images via existing `upgrade.sh` whitelist extension; crucix via `scripts/build-crucix.sh <new-sha>` (manifest channel `git-commit`).

## 4. Hub changes (Rust)

- **Migration `0003_geo.sql`**: `geo_events(event_id uuid PK default gen_random_uuid(), source text, external_id text, kind text, title text, lat double precision, lon double precision, severity text, occurred_at timestamptz, payload jsonb, ingested_at timestamptz default now(), UNIQUE(source, external_id))`; indexes on `occurred_at DESC`, `severity`, `(source, kind)`.
- **`crucix.rs` worker** (started in server.rs with the other SP2B workers):
  - Every 60s: `GET {HUB_CRUCIX_URL}/api/health` (5s timeout) → compare `lastSweep` with last-seen value in redis.
  - New sweep → `GET /api/data` (30s timeout) → normalize each geo-bearing section (fires, conflicts, flights/hotspots, radiation, maritime/chokepoints, geolocated news, health alerts) into geo_events rows; `(source, external_id)` upsert = idempotent; counts logged + bus event `crucix_sweep_ingested`.
  - Payload alert entries at FLASH/PRIORITY severity → hub `alerts` via existing `raise_alert` dedupe (source=`crucix`).
  - Crucix down → worker logs, backs off 60s, zero impact on other functions (§72).
- **REST**:
  - `GET /api/v1/radar/events?from&to&severity&source&kind&limit` (auth, L1) — bounded `LIMIT 500` default.
  - `GET /api/v1/metrics/summary` — instant Prometheus queries (host CPU %, RAM %, disk %, net rx/tx rate, container count); degrades to `{telemetry: "unavailable"}` when Prometheus is down.
  - `/api/v1/overview` += `crucix: {sources_ok, sources_failed, last_sweep, geo_events_24h}`.
  - `system_health` += crucix / grafana / prometheus probes (3s-capped like all probes).
- **components.rs**: register 5 new manifests so §69 Component Manager shows them.

## 5. Console changes

- **Radar page** (`/radar`, new top-level nav): Leaflet map; default CARTO dark_all tiles; `tileerror` burst (≥4 failures) or first-load timeout (5s) → swap to bundled coastline GeoJSON vector layer; `TILES: ONLINE/OFFLINE FALLBACK` badge, manual retry button. Event markers colored by severity, shaped by kind; filters: time window (1h/24h/7d), severity, source, kind; click marker → sidebar detail (title, source, time, coords, raw payload) + "转为调查" action (create investigation, link event). SSE `crucix_sweep_ingested` triggers refetch (live drops).
- Coastline asset: Natural Earth 110m land GeoJSON (~2MB, public domain) bundled into `console/public/`.
- **Overview**: radar card live (24h event count + severity breakdown + link).
- **System page**: §52 metrics section (CPU/RAM/disk/net + container table) from `/api/v1/metrics/summary`; links to Grafana `:3001` and Crucix `:3117` native UIs (plain links).
- npm: add `leaflet` (pinned, package-lock committed).

## 6. Acceptance (`scripts/accept-sp4.py`)

1. 5 new containers healthy; registered in manifests + `hub-compose.sh ps`.
2. Crucix completes ≥1 sweep: `/api/health` `sourcesOk ≥ 15`.
3. `geo_events` populated ≤90s after sweep; radar endpoint filters (severity/kind/time) verified.
4. Alert conversion function unit-verified (synthetic FLASH payload if no live alert).
5. Prometheus `/api/v1/targets` all up; Grafana datasource API healthy + dashboard provisioned.
6. `/api/v1/metrics/summary` returns real CPU/RAM numbers.
7. Console: `/radar` 200 + markers render; System metrics section populated.
8. Degradation drill: stop crucix → hub + console unaffected, radar endpoint returns graceful empty.
9. **Regressions**: SP2A 16/16, SP2B 32/32, SP3 19/19.

## 7. Non-goals

- No Crucix native UI embedding (§24); no replication of full Crucix dataset (§51 — key events/alerts/geo only).
- No sensor start/stop orchestration (SP3 decision stands).
- No PostGIS (plain lat/lon doubles are sufficient at this scale).
- No Prometheus alerting rules (hub alerts remain the single alert center, §54).

---

## Amendment 2026-09-09 — Deployment Record (as-built)

Deployed and acceptance-tested 2026-09-09. **SP4 acceptance 25/25 + regressions SP2A 16/16, SP2B 32/32, SP3 19/19 — all green.**

As-built facts:

- **Crucix**: source-built image `crucix:sha-3db7068` (pinned commit, no upstream tags exist); LLM base-URL build patch to T8star relay verified live (`llmEnabled: true`, model gpt-4.1-mini); first sweep 26/29 sources OK (3 failures are the key-gated sources — FIRMS/ACLED/EIA per Q1=A; env slots ready in `compose/.env.crucix`).
- **Observability**: prom/prometheus v3.14.0, prom/node-exporter v1.12.1, gcr.io/cadvisor/cadvisor v0.55.1, grafana/grafana 13.2.1 — all healthy; Prometheus targets 3/3 up; Grafana has provisioned Prometheus datasource + "IntelHub Overview" dashboard (uid intelhub-overview); native UI at http://10.10.10.41:3001 (admin password in VM compose/.env).
- **Hub**: migration `0003_geo.sql` (geo_events); `crucix.rs` worker (60s poll → new sweep → normalize → idempotent upsert; FLASH/PRIORITY → hub alerts); `GET /api/v1/radar/events` (time/severity/source/kind filters, SQL-level time bounds); `GET /api/v1/metrics/summary` (5 concurrent Prometheus instant queries, 3s cap, graceful `telemetry:unavailable`); system_health += crucix/prometheus/grafana probes; overview += radar block.
- **Console**: `/radar` page (Leaflet 1.9.4, CARTO dark tiles default → auto-fallback to bundled Natural Earth 110m countries GeoJSON 426KB on tileerror burst/5s timeout, TILES ONLINE/OFFLINE badge + manual retry, severity-colored markers, kind/severity/time filters, detail sidebar + 转为调查, SSE live refresh on `crucix_sweep_ingested`); Overview radar card live; System page §52 telemetry section + Grafana/Crucix native-UI links.

Deviations/incidents (fixed, regression-covered):

19. **mgmt-net subnet collision**: directive-style 172.30.1.0/24 was already auto-assigned to intelhub-egress by Docker in SP1 → mgmt-net pinned to **172.30.3.0/24** (data=.2, egress=.1, sensor=.0, mgmt=.3). Static IPs: prometheus .20, crucix .21, grafana .22, node-exporter .23, cadvisor .24.
20. **Crucix /api/data serves the synthesized dashboard payload** (top-level sections: thermal, acled, chokepoints, news, …), NOT the raw `sources{}` briefing (that lives in runs/latest.json). Normalizer walks top-level sections with a skip-list.
21. metrics/summary ran 5 sequential 3s-timeout Prometheus queries (up to 15s hang when Prometheus down) → parallelized with tokio::join! (bounded ≤3s).
22. Acceptance scripts must query mgmt-net services VM-side (static IPs unreachable from Mac); Grafana is on 172.30.3.22:3000 internally, LAN 10.10.10.41:3001.
