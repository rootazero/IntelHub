# Series Catalog — Single Source of Truth for Signal Naming

**Date**: 2026-09-15
**Author**: Claude (after HUD gauge series ID bug on Monitor Command Deck)
**Status**: Draft — pending review
**Scope**: SP6B finance collectors + SP7 Monitor HUD + future quant-readability PR2/PR3

## 1. Problem

The Monitor Command Deck (SP7) renders a risk-gauge panel with five metrics:

| Label | Code query (frontend) | DB stored (backend) | Status |
|---|---|---|---|
| VIX | `fred:VIXCLS` | `fred:VIXCLS` | ✅ matches |
| 10Y % | `fred:DGS10` | `fred:DGS10_PCT` | ❌ silent miss |
| 2s10s | `fred:T10Y2Y` | `fred:T10Y2Y` | ✅ matches |
| HY OAS | `fred:BAMLH0A0HYM2` | `fred:HY_OAS_PCT` | ❌ silent miss |
| WTI $ | `eia:WTI` | `eia:WTI_SPOT_USD_BBL` | ❌ silent miss |

The user observed three blank gauges and asked "is this a bug?" — it was. The frontend queried raw FRED/EIA series IDs while the SP6B collectors emit namespaced + unit-suffixed names. The mismatch is invisible to type-checkers (both sides are `string`), invisible to tests (no integration test cross-references frontend queries against backend emits), and silent at runtime (frontend renders `null` and the gauge shows an empty cell — no error toast, no console warning).

## 2. Root cause

Two separate decisions converged badly:

1. **SP6B emit-time normalization**: To prevent collision between sources that share raw IDs (e.g., both FRED and a third party might publish `GDP`), SP6B collectors build the storage key as `format!("{source}:{id}_{unit}", ...)`. The unit suffix is required for unit-aware downstream queries (e.g., distinguishing `BAMLH0A0HYM2` percent from the same series reported in basis points).
2. **SP7 frontend lookup**: The Monitor.tsx `GAUGES` constant was written before SP6B's emit contract was finalized, using raw upstream IDs (`DGS10`, `BAMLH0A0HYM2`, `WTI`). No contract was propagated forward — the frontend author had to reverse-engineer the storage shape from curl output.

There is **no machine-readable contract** between the two sides. The naming convention is tribal knowledge in three places: SP6B design spec (`docs/superpowers/specs/2026-09-10-intelhub-sp6b-finance-sources-design.md`), the SIGINT mirror log, and the SP6B emit code. Any future frontend author, MCP tool, or quant agent must re-derive the canonical naming from one of these sources.

## 3. Inventory (as of 2026-09-15)

`GET /api/v1/signals/latest` currently returns **70 distinct series** across 9 sources:

| Source | Count | Example |
|---|---|---|
| `comtrade:` | 6 | `comtrade:CN.exp.semiconductors_usd` |
| `eia:` | 2 | `eia:WTI_SPOT_USD_BBL`, `eia:BRENT_SPOT_USD_BBL` |
| `fred:` | 11 | `fred:DGS10_PCT`, `fred:VIXCLS`, `fred:T10Y2Y` |
| `gscpi:` | 1 | `gscpi:index` |
| `nasa:` | 1 | `nasa:GISTEMP_ANOM_C` |
| `noaa:` | 1 | `noaa:CO2_MLO_PPM` |
| `quote:` | 23 | `quote:AAPL`, `quote:BTCUSD` (one per watchlist symbol) |
| `sentiment:` | 23 | `sentiment:AAPL` (mirrors quote set) |
| `treasury:` | 2 | `treasury:TOTAL_DEBT_USD`, `treasury:AVG_RATE_MARKETABLE_PCT` |

Unit suffixes observed: `_PCT`, `_USD`, `_BBL`, `_K`, `_YOY`, `_SPOT_USD_BBL` (compound), `_ANOM_C`, `_MLO_PPM`.

## 4. Goals

1. **Single source of truth** — one Rust module defines every series identity. Both emit-side collectors and read-side consumers import from the same place.
2. **Compile-time safety** — frontend cannot reference a series that doesn't exist in the catalog. Type errors at build, not silent runtime misses.
3. **Discoverability** — `GET /api/v1/series` returns the full catalog so MCP agents, frontend authors, and external tools can browse without reading source.
4. **Refactorability** — when a collector's emit contract changes (e.g., a unit suffix is added), all read-side consumers get a compile error pointing at the stale reference.

## 5. Non-goals

1. **Backfill of historical `signal_observations` rows** — the storage keys stay as-is. Catalog is a metadata layer over existing data.
2. **Migrating SP1/SP2/SP3/SP4/SP5 series** — climate, GDELT, ACLED, OFAC, etc. emit shapes are not covered by this spec. They can be added in a followup.
3. **Quant readability PR2/PR3 history table** — the catalog is the lookup table that quant_history rows reference, but the quant history backfill itself is a separate PR (per existing quant-readability spec §6.4).

## 6. Design

### 6.1 `SeriesDescriptor` struct

`hub-core/src/series.rs`:

```rust
use std::sync::OnceLock;

pub enum Unit { Percent, Usd, Bbl, Kelvin, Ppm, Index, Count, BasisPoints, String }
pub enum Source { Fred, Eia, Treasury, Comtrade, Gscpi, Nasa, Noaa, Quote, Sentiment }

pub struct SeriesDescriptor {
    /// Source namespace. Stable across renames.
    pub source: Source,
    /// Upstream raw ID (e.g. FRED's `DGS10`, EIA's `DCOILWTICO`).
    pub raw_id: &'static str,
    /// Storage key written to signal_observations.series.
    /// Built as `{source}:{raw_id}_{unit_suffix}` per the convention.
    pub normalized_id: &'static str,
    /// Display label shown to humans (e.g., "10Y %").
    pub display_name: &'static str,
    /// What the value represents.
    pub unit: Unit,
    /// Optional human-readable description for the catalog endpoint.
    pub description: &'static str,
}

/// Static catalog. All entries verified at compile time.
pub fn catalog() -> &'static [SeriesDescriptor] {
    static CATALOG: OnceLock<Vec<SeriesDescriptor>> = OnceLock::new();
    CATALOG.get_or_init(|| vec![
        // === FRED (11 series) ===
        SeriesDescriptor { source: Source::Fred, raw_id: "DGS10",
            normalized_id: "fred:DGS10_PCT", display_name: "10Y %",
            unit: Unit::Percent, description: "10-year Treasury constant maturity rate" },
        SeriesDescriptor { source: Source::Fred, raw_id: "DGS2",
            normalized_id: "fred:DGS2_PCT", display_name: "2Y %",
            unit: Unit::Percent, description: "2-year Treasury constant maturity rate" },
        // ... 9 more
        // === EIA (2 series) ===
        SeriesDescriptor { source: Source::Eia, raw_id: "DCOILWTICO",
            normalized_id: "eia:WTI_SPOT_USD_BBL", display_name: "WTI $",
            unit: Unit::Usd, description: "WTI spot price USD per barrel" },
        // ...
    ])
}

/// Lookup helper — collectors use this to convert (source, raw_id) into the storage key.
pub fn normalize(source: Source, raw_id: &str) -> Option<&'static str> {
    catalog().iter()
        .find(|d| d.source == source && d.raw_id == raw_id)
        .map(|d| d.normalized_id)
}
```

### 6.2 Collector refactor

Each SP6B collector replaces inline string construction with `series::normalize()`:

```rust
// Before (current):
series: format!("fred:{}", raw_id),  // OR format!("fred:{}_{}", raw_id, unit_suffix)

// After:
series: series::normalize(Source::Fred, raw_id)
    .ok_or_else(|| HubError::monitor(format!("unknown FRED series: {raw_id}")))?
    .to_string(),
```

Adding a new series to a collector becomes a 3-line change:
1. Add entry to `series::catalog()`
2. Reference raw_id in the collector
3. Emit path uses `normalize()`

Unknown raw IDs fail loudly at emit time, not silently as orphan rows.

### 6.3 Frontend consumption

Two delivery mechanisms:

**a) Build-time JSON export**

`hub-core/build.rs` exports `series_catalog.json` at compile time. Vite imports it as a TypeScript module:

```ts
import CATALOG from "../hub-core/build/series_catalog.json" with { type: "json" };

// Compile-time-typed lookup:
const dgs10 = CATALOG.series.find(s => s.display_name === "10Y %")!;
// dgs10.normalized_id === "fred:DGS10_PCT"
```

The Monitor.tsx `GAUGES` array becomes:

```ts
const GAUGES = [
  { series: CATALOG.series.find(s => s.normalized_id === "fred:VIXCLS")!.normalized_id, label: "VIX", warnAbove: 30 },
  { series: CATALOG.series.find(s => s.display_name === "10Y %")!.normalized_id, label: "10Y %" },
  // ...
];
```

If a display name is mistyped or renamed upstream, the build fails.

**b) Runtime endpoint**

`GET /api/v1/series` returns `{series: SeriesDescriptor[]}` for MCP agents, debugging UIs, and external tools. Cache in Redis with TTL=600s (catalog rarely changes).

### 6.4 Compile-time invariant test

`crates/hub-core/tests/series_catalog_invariant.rs`:

```rust
#[test]
fn catalog_no_duplicate_normalized_id() {
    let mut seen = HashSet::new();
    for d in series::catalog() {
        assert!(seen.insert(d.normalized_id), "duplicate normalized_id: {}", d.normalized_id);
    }
}

#[test]
fn catalog_normalized_id_format() {
    for d in series::catalog() {
        let prefix = format!("{:?}", d.source).to_lowercase() + ":";
        assert!(d.normalized_id.starts_with(&prefix),
            "{} missing source prefix", d.normalized_id);
    }
}
```

### 6.5 Discovery endpoint

`GET /api/v1/series` (public, no auth required — catalog is metadata):

```json
{
  "count": 70,
  "series": [
    {"source": "fred", "raw_id": "DGS10", "normalized_id": "fred:DGS10_PCT",
     "display_name": "10Y %", "unit": "Percent",
     "description": "10-year Treasury constant maturity rate"},
    ...
  ]
}
```

Filtering via query params: `?source=fred`, `?unit=Percent`.

## 7. Migration plan

### Phase 1: skeleton (no behavior change)
1. Create `hub-core/src/series.rs` with `SeriesDescriptor` struct + 11 FRED entries.
2. Add `series::catalog()` returning the FRED entries (verified by inventory).
3. Build-time JSON export: `hub-core/build.rs` writes `series_catalog.json`.
4. Compile-time invariant test: 2 tests, must pass.

### Phase 2: collector refactor (FRED + EIA + treasury)
1. Refactor `monitor/sources/fred.rs` to use `series::normalize(Source::Fred, raw_id)`.
2. Same for `eia.rs` + `treasury.rs`.
3. Compare emitted `signal_observations.series` strings before/after — must be byte-identical.
4. Add acceptance: `accept-sp7.py` check that fetches `/api/v1/signals/latest` and asserts every FRED series string matches a catalog entry.

### Phase 3: remaining collectors
1. `comtrade.rs`, `gscpi.rs`, `nasa.rs`, `noaa.rs`, `quote.rs`, `sentiment.rs`.
2. Acceptance: every series in `/api/v1/signals/latest` resolves through `series::catalog()`.

### Phase 4: frontend
1. Add TypeScript import for `series_catalog.json`.
2. Refactor `Monitor.tsx` `GAUGES` to use catalog lookups.
3. Add `accept-sp7.py` check: bundle contains all 5 catalog-resolved HUD gauge series.
4. Add `accept-sp7.py` check: bundle does NOT contain any string literal `fred:` or `eia:` (catches future regressions where someone hardcodes a series ID).

### Phase 5: discovery endpoint
1. `GET /api/v1/series` returns full catalog.
2. Cache in Redis with TTL=600s.
3. Documentation in `docs/api/series.md`.

## 8. Acceptance

- [ ] `cargo test --lib --test series_catalog_invariant` passes (no duplicates, format correct).
- [ ] `cargo test --lib` passes for the entire hub-core crate.
- [ ] `accept-sp5.py` continues to pass — emit contract byte-identical.
- [ ] `accept-sp7.py` extended with 3 new checks:
  - [ ] All 70 series in `/signals/latest` resolve through `series::catalog()`.
  - [ ] Monitor.tsx bundle contains no hardcoded `fred:` / `eia:` strings (except catalog import).
  - [ ] `/api/v1/series` returns at least 70 entries.
- [ ] Visual verification: all 5 Monitor HUD gauges show real values.

## 9. Risks

| Risk | Mitigation |
|---|---|
| Catalog drift (entry added but never wired) | Compile-time invariant test + `accept-sp7.py` cross-reference check |
| Frontend bundle size growth (70 entries × ~200 bytes = 14 KB) | Negligible vs. 1.4 MB JS bundle. Inline only catalog metadata, not values. |
| Adding new series requires PR to update catalog | **This is intentional.** It forces a code review at the point of change. |
| Backward-compat for in-flight emit strings during rollout | Phase 2 verifies byte-identical output before any collector is migrated. |
| Quote/sentiment catalogs grow with watchlist (23+ entries) | Catalog accepts any number of entries. Watchlist manager writes catalog entries on add/remove. |

## 10. Out of scope (followups)

1. **SP1–SP5 series catalogs** — climate, GDELT, ACLED, OFAC, etc. Out of scope for this PR. Could be a follow-up spec adding ~30 more entries.
2. **Quant history table (`quant_history`)** — separate PR per quant-readability spec §6.4. The catalog is the lookup table that history rows reference.
3. **Series quality metadata** — staleness thresholds, confidence intervals, source priority. The catalog carries `description` + `unit`; richer metadata can be a follow-up.
4. **Auto-discovery from FRED** — some collectors (FRED has 800+ series) could auto-enumerate from the upstream API. Catalog entries would be generated. Out of scope to keep catalog author-controlled.

## 11. Estimated size

| File | Lines (estimate) |
|---|---|
| `hub-core/src/series.rs` | 200 (struct + 70 catalog entries) |
| `hub-core/build.rs` | 30 (catalog JSON export) |
| `hub-core/tests/series_catalog_invariant.rs` | 50 |
| `hub-core/src/api/series.rs` (endpoint) | 80 |
| `console/src/lib/catalog.ts` | 40 |
| `console/src/pages/Monitor.tsx` refactor | 20 |
| `scripts/accept-sp7.py` extension | 40 |

Total: ~460 LoC across 6 files. Comparable to a single SP followup PR.