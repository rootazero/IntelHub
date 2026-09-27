# Public-API Integration Roadmap

**Date**: 2026-09-27
**Status**: Draft → Approved 2026-09-27
**Author**: Claude (after surveying https://github.com/public-apis/public-apis vs IntelHub's 75+ existing monitors)
**Scope**: Hub-core `monitor::registry()` + `series_registry()` expansion over the next ~2 months
**Strategy**: Small-step shipping (one source per PR, ≤ 1-2 dev-days each), but planned at scale so we don't double back

---

## 1. Strategic Framework

### 1.1 Why a roadmap before a single PR

IntelHub already covers 75+ collectors (see `hub-core/crates/hub-core/src/monitor/sources/`). Adding public APIs one-at-a-time without a plan guarantees two failure modes:

1. **Duplicate coverage**: e.g., IP-geolocation collectors (we have `ipapi_co`, `ip_api_com`; public-apis lists 12 more). Adding a 14th IP-geo source for marginal gain wastes the sp6 acceptance budget.
2. **No compounding value**: each isolated source is a standalone dashboard blip. The roadmap aims for **cross-source compounding** — e.g., `ArcNautical` sanctions × `AIS` positions × `OpenSanctions` PEP rows creates a "vessel-ownership intelligence" product that no single source can deliver.

### 1.2 Value-scoring rubric

Every candidate API scored 1-5 on five axes; final `V` = sum, max 25. Higher = ship sooner.

| Axis | Weight | What 5 means | What 1 means |
|---|---|---|---|
| **V₁ Gap-fill** | ×1.0 | Fills a core OSINT dimension IntelHub has zero coverage of | Already covered by equivalent/stronger source |
| **V₂ Deploy cost** (inverted) | ×1.0 | No key, free tier, no signup friction | Paid key, credit card, manual approval |
| **V₃ Data stability** | ×1.0 | Government / 10y-old project / explicit uptime SLA | Hobby project, last commit 2y ago |
| **V₄ Multi-use** | ×1.0 | Used in 3+ downstream seams (Radar / Alerts / Enrich / Entity / Cockpit) | Single-purpose, one widget |
| **V₅ ToS clarity** (inverted) | ×1.0 | Public-domain, ODbL, CC0, explicit commercial-OK | NC-only, "research only", ambiguous |

**Tiebreakers** (when scores are equal):
- Adds a **new geospatial layer** > adds a non-geo series
- Compounds with existing sources > stands alone
- Lower CPU/network cost per sweep (because of the 74-source stampede constraint documented in `AGENTS.md`)

### 1.3 Phase boundaries

| Phase | Theme | Cadence per PR | Approx count | Exit criteria |
|---|---|---|---|---|
| **1. Keyless Coverage Sprint** | Maximize gain, zero friction | 1 source / 0.5-1 day | 8 sources | All 8 green on 315 + 410; sp6 +N |
| **2. Free APIKey Expansion** | Apply for the free keys | 1 source / 1-2 days | 5 sources | All 5 green; sp6 +M |
| **3. Geopolitical Hardening** | High-value paid / specialized | 1 source / 2-4 days | 5 sources | All 5 green + entity/cockpit wiring |
| **4. Specialized / Watchlist** | Keep watching, ship opportunistically | ad-hoc | 5+ | None — opportunistic |

---

## 2. Phase 1 — Keyless Coverage Sprint (≈ 5-8 dev-days total)

> All Phase 1 sources are **no key required** and fill a clear OSINT gap. Each ships as a single PR + sp6 acceptance bump.

### 2.1 Inventory

| # | Source | Tier | V₁ | V₂ | V₃ | V₄ | V₅ | **Σ** | Sweep cadence | Trait | sp6 check shape |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1.1 | **Open-Meteo** (`open-meteo.com`) — global weather forecast | geo | 5 | 5 | 5 | 4 | 5 | **24** | 30 min | `Source` → Signals | `open_meteo_rows>0` + bbox coverage |
| 1.2 | **Nager.Date** (`date.nager.at`) — 90+ countries public holidays | geo | 4 | 5 | 5 | 3 | 5 | **22** | 24 h | `Source` → Signals (priority=routine) | `nager_holidays_rows>0` |
| 1.3 | **Sunrise-Sunset** (`sunrise-sunset.org`) — daylight window by lat/lng | enrich | 3 | 5 | 5 | 4 | 5 | **22** | 24 h | `Source` → Signals (event-time anchor) | `sunrise_sunset_rows>0` |
| 1.4 | **USGS Water Services** (`waterservices.usgs.gov`) — US rivers/water | geo | 5 | 5 | 5 | 3 | 5 | **23** | 60 min | `Source` → Signals (flood/drought severity) | `usgs_water_rows>0` + flood-stage filter |
| 1.5 | **Queimadas INPE** (`queimadas.dgi.inpe.br`) — Brazil wildfire hotspots | geo | 5 | 5 | 4 | 3 | 5 | **22** | 60 min | `Source` → Signals (parity with FIRMS) | `queimadas_rows>0` + bbox |
| 1.6 | **HDX Humanitarian** (`data.humdata.org`) — UN crisis datasets | entity | 5 | 5 | 5 | 3 | 5 | **23** | 24 h | `SeriesCollector` → `humanitarian_*` rows | `hdx_datasets>5` + sample entity bridge |
| 1.7 | **OpenAQ** (`docs.openaq.org`) — global air quality | geo | 5 | 4 | 5 | 4 | 5 | **23** | 30 min | `Source` → Signals (env layer parity with EPA) | `openaq_rows>0` + city coverage > 5 continents |
| 1.8 | **Helium News** (`heliumtrades.com/mcp`) — bias-scored news + market data | entity | 4 | 5 | 3 | 3 | 5 | **20** | 15 min | `SeriesCollector` → news-feed rows | `helium_news_rows>0` + bias distribution |

> **Note**: 1.7 (OpenAQ) is technically free-tier apiKey but signup is instant and never requires payment. Grouped into Phase 1 because the friction cost is < 5 minutes and is the only way to access global air quality.

### 2.2 Per-PR contract

Each Phase 1 PR follows the existing pattern (`usgs.rs` / `noaa.rs` / `firms.rs` as templates):

1. New file `hub-core/crates/hub-core/src/monitor/sources/<name>.rs`
2. `pub mod <name>;` in `monitor/sources/mod.rs`
3. `Box::new(sources::<name>::<Name>)` in `monitor::registry()` (or `series_registry()`)
4. Env key check using the existing `select()` helper pattern (`HUB_<KEY>` wins, bare `<KEY>` fallback, missing → graceful degrade with one-line `tracing::info!`)
5. Unit tests in-file: happy-path + empty-response + shape-validation
6. Bump `scripts/accept-sp6.py` with new check(s) (mandatory per `AGENTS.md`)
7. **Bump baseline after 315 + 410 both pass**, never one-sided

### 2.3 Stampede budget

Current scheduler stagger = `1.5s × index` across 74 sources (`monitor/scheduler.rs:48`). Adding 8 sources → 82, stagger unchanged at 1.5s (now ~123s total). **No new sequential-within-source loop** unless absolutely required. Reference vendor "cctv-refresh" 11-provider sequential (≤103s) is the upper bound — nothing Phase 1 touches that file.

### 2.4 Phase 1 exit gate

- 315 + 410 both `sp6 passed >= 56, failed == 0` (current baseline 51+5/1 — see `AGENTS.md`)
- New sources visible on `monitor` command deck (SP7)
- One radar-layer demo for at least one of {Open-Meteo, OpenAQ, USGS Water} — proves geo seam wiring

---

## 3. Phase 2 — Free APIKey Expansion (≈ 7-12 dev-days)

> These require a free signup but no payment. The 5-minute sign-up is the only cost.

| # | Source | V₁ | V₂ | V₃ | V₄ | V₅ | **Σ** | Phase 1 layer it compounds with |
|---|---|---|---|---|---|---|---|---|
| 2.1 | **GreyNoise Community** (`docs.greynoise.io`) — IP context, IoT vs scanner | 5 | 4 | 5 | 4 | 5 | **23** | `shodan_internetdb`, `romainmarcoux_malicious_ip` |
| 2.2 | **Currents API** (`currentsapi.services`) — multi-lang news as **GDELT 429 fallback** | 5 | 4 | 4 | 4 | 5 | **22** | `gdelt`, `reliefweb`, `rss` (AGENTS notes GDELT shared-IP rate limit) |
| 2.3 | **Semantic Scholar** (`api.semanticscholar.org`) — academic citations | 3 | 5 | 5 | 4 | 5 | **22** | `embed.rs`, `enrich` stage |
| 2.4 | **GBIF** (`gbif.org/developer`) — biodiversity occurrences | 4 | 5 | 5 | 2 | 5 | **21** | Standalone (new dimension: ecological/animal-disease) |
| 2.5 | **FOFA** (`en.fofa.info`) — Chinese cyberspace asset mapping | 5 | 4 | 4 | 3 | 4 | **20** | `shodan_internetdb`, `ipapi_co` |

### 3.1 Phase-2-specific concerns

- **Currents API** specifically addresses the **GDELT 429 trap** documented in `AGENTS.md`. Wire it as fallback: when `gdelt` health cell reports rate-limit in last 10 min, raise Currents weight; otherwise stay quiet. Implement as `rss`-style multi-source aggregator, not a duplicate signal producer.
- **FOFA** requires China-network routing. IntelHub VM is on PVE40 → openclash → likely already routable, but verify during dev. Document the test in PR description.
- **Semantic Scholar** integrates into `enrich.rs` — citations appear as `enriched.evidence[].source_kind="semantic_scholar"`. This is the first Phase-2 source that's **not a monitor** — design first, write code second.

### 3.2 Phase 2 exit gate

- 5 sources shipped, sp6 passing +N
- New **`enrich`** integration test covers Semantic Scholar citation path
- GDELT-fallback wiring demonstrably reduces "GDELT empty" rate

---

## 4. Phase 3 — Geopolitical Hardening (≈ 10-16 dev-days)

> Highest-value, but each requires either a paid key OR a special-purpose integration. **User decision required before Phase 3 starts**: which of these is worth the cost/approval?

| # | Source | V₁ | V₂ | V₃ | V₄ | V₅ | **Σ** | What makes this Phase 3 |
|---|---|---|---|---|---|---|---|---|
| 3.1 | **ArcNautical** (`arcnautical.com`) — vessel IMO + sanctions + ownership opacity | 5 | 3 | 4 | 5 | 5 | **22** | Paid; compounds directly with `AIS` for **vessel-ownership intelligence** |
| 3.2 | **Strait of Hormuz Ship Monitor** (`hormuz.data-tracking.net`) — live chokepoint AIS | 5 | 5 | 3 | 4 | 4 | **21** | Keyless BUT requires custom routing logic + alert-rule integration |
| 3.3 | **CompliAPI / Vett** (`docs.compliapi.com`) — multi-source sanctions (OFAC/EU/UK/PEP) | 5 | 3 | 5 | 4 | 4 | **21** | Paid; cross-validates existing `opensanctions` + `ofac` |
| 3.4 | **GitGuardian** (`api.gitguardian.com`) — leaked-secret detection | 4 | 3 | 5 | 3 | 4 | **19** | Paid; integrates into `embed`/`crawl` flow, not a monitor |
| 3.5 | **CongressInvests** (`congressinvests.com`) — US politician trades | 4 | 3 | 4 | 3 | 4 | **18** | Paid; entity-graph compounding with `sec_edgar` + `treasury` |

### 4.1 Sequencing within Phase 3

1. **3.2 first** (keyless, pure routing logic) — proves the chokepoint-alert pattern, then 3.1/3.3 plug into the same alert-rule scaffolding.
2. **3.1 + 3.3** in parallel after 3.2 lands — both compound with vessel/sanctions entity graph.
3. **3.4 + 3.5** last — both are non-monitor integrations, ship after the OSINT core is solid.

### 4.2 User-decision checkpoints (mandatory before each PR)

- **3.1**: Approve ArcNautical subscription cost (tier?). Without approval, source file ships in **shelved** mode (compiles, registers with `tracing::info!`, REST reports `status="missing-key"`).
- **3.3**: Approve CompliAPI tier OR decide to keep current `opensanctions` only.
- **3.4**: Decide if GitGuardian leaks should appear in `claim.evidence` (privacy/legal review).
- **3.5**: Decide if politician-trade signals should auto-alert or only enrich entities on demand.

### 4.3 Phase 3 exit gate

- 5 sources shipped (or shelved with explicit user sign-off)
- Vessel-ownership intelligence product demonstrable: ArcNautical + AIS + OFAC → single Vessel entity node
- New **Cockpit TCAS-style "compliance panel"** for ship owners (if 3.1+3.3 land)

---

## 5. Phase 4 — Watchlist / Opportunistic (no schedule)

These stay on the watchlist. Ship opportunistically when (a) IntelHub's roadmap has bandwidth, (b) a real signal-demand surfaces, or (c) the upstream improves.

| Source | Reason for watchlist |
|---|---|
| **Helium (IoT Hotspots)** | Edge OSINT; very niche; no clear dashboard demand yet |
| **ThreatCluster** | Competes with `nvd` + `osv`; evaluate only after Phase 2 done |
| **Open Disease** | Health dimension; evaluate after Humanitarian (1.6) lands and shows value |
| **HDX deep integration** (post-1.6) | Per-dataset parsers, only if Phase 1.6 actually surfaces useful entities |
| **Festival Public Holidays** | Strict upgrade of Nager.Date; ship if Nager proves insufficient |
| **Aether-X Port Congestion** | Compounds with `AIS` once Phase 3.1 lands |
| **CongressInvests (lite)** | Free-tier of 3.5 if it exists |

---

## 6. Cross-Phase Concerns

### 6.1 Enrich + Entity-graph integration

Phase 2.3 (Semantic Scholar) and Phase 3.4 (GitGuardian) are **not monitors** — they enrich. Their wiring is a separate design pass that should land **before** Phase 2 starts:

- New `enrich.rs` extension points: `enrich_with_semantic_scholar(claim)`, `enrich_with_gitguardian(url)`
- Both should be **opt-in** via env flag, not always-on, to keep cost predictable
- Citations become `enriched.evidence[].source_kind = "semantic_scholar" | "gitguardian"` so the trace is auditable

### 6.2 Radar / Cockpit integration

Phase 1.1 (Open-Meteo), 1.4 (USGS Water), 1.5 (INPE), 1.7 (OpenAQ), and Phase 3.2 (Hormuz) all add new **geospatial layers** that should appear on the Radar tab and the Cockpit SV-style overlay. After Phase 1 lands, a separate UI PR should expose a `◇ LAYERS` switcher (mirroring `HudCockpitElementSwitch` P17 pattern).

### 6.3 Source-contracts test discipline

Each new source ships with a `source-contracts.test.ts` analog (`hub-core/crates/hub-core/tests/contracts_<name>.rs`) covering:
- Env-key fallback (`HUB_<KEY>` wins, bare `<KEY>` fallback, empty → graceful)
- 25s timeout respected (no hung task)
- Empty upstream → zero signals, not crash
- Restart-safety (idempotent sweep — no PG duplicates on restart-mid-sweep)
- Health cell writes (Redis `monitor:health:<name>` HSET)

This is enforced by `accept-sp6.py` already; the new check pattern is added in the same PR as the source.

### 6.4 Cost + key management

- `HUB_<KEY>` env vars live in `core/hub.env` on the VM only (already in `.gitignore`)
- `scripts/resolve-versions.sh` already detects stale `$(...)` templates (per AGENTS.md)
- Phase 2+ keys are added via the existing `step_secrets` flow, no new plumbing

---

## 7. Explicitly Out of Scope

| Candidate | Why excluded |
|---|---|
| Aviation Stack (paid flight status) | Already covered by `opensky` + `adsb` + `adsbexchange` |
| Aviation Safety Data | Niche; not a primary OSINT signal |
| Mapbox / Geoapify / LocationIQ | `nominatim` + `overpass` already cover this |
| Etherscan (alt-flavor) | Already have `etherscan.rs` |
| Bitquery / Covalent / Block.io | `defillama` + `etherscan` already cover crypto |
| IPstack / BigDataCloud / IPinfo etc. | `ipapi_co` + `ip_api_com` already cover; per 1.1 we don't add a 14th IP-geo source |
| Wikidata SPARQL variants | Already covered by `wikidata.rs` |
| Hunter.io / Apollo | Email-finder; not currently a signal demand |
| OpenCorporates variants | Already on `ROADMAP.md` as wontfix-by-design |
| 4chan / Discord / Slack / Twitter variants | `bluesky` + `x` already in; adding more social isn't the OSINT differentiation |

---

## 8. Acceptance / Verification Strategy

### 8.1 sp6 bumps per phase

| Phase | New sp6 checks | Approx total additions |
|---|---|---|
| Phase 1 | 16 (2 per source × 8) | +16 |
| Phase 2 | 10 (2 per source × 5) | +26 |
| Phase 3 | 10 (2 per source × 5) | +36 |
| Phase 4 | opportunistic | — |

### 8.2 Rollout per PR

1. Worktree (`feat/api-<name>`) on Mac
2. rsync → 315 → build → restart → wait 5 min (per AGENTS.md deploy-stampede rule)
3. 315 acceptance: `python3 scripts/accept-sp6.py "$KEY"` — must pass
4. Merge to main, rsync → 410 → same sequence
5. 410 acceptance — green
7. **5 minutes idle** before next restart (AGENTS.md)
8. `git push origin main`
9. Update `ROADMAP.md` Phase status table

### 8.3 Rollback discipline

- If 410 acceptance regresses (even 1 fail), revert on 410 immediately; don't try to fix forward
- Source remains in code but gets `HUB_MONITOR_SOURCES` exclude in `core/hub.env` until root-caused

---

## 9. Decision Log

| Date | Decision | Rationale |
|---|---|---|
| 2026-09-27 | Adopt this roadmap | Cross-source compounding beats one-at-a-time |
| 2026-09-27 | Phase 1 first, no skipping | Stampede budget is tight; ship 8 keyless sources before adding key friction |
| 2026-09-27 | Semantic Scholar is **not** a monitor | Put it in `enrich.rs` instead — keeps sweep costs predictable |
| TBD | ArcNautical + CompliAPI tiers | User decision required (cost) |
| TBD | GitGuardian privacy posture | Legal review required |