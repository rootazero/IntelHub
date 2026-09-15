//! Series catalog — single source of truth for every signal series identity.
//!
//! Every collector that writes to `signal_observations` and every reader
//! (frontend, MCP agent, CLI) that queries by series string goes through
//! this catalog. The `Source` enum replaces raw `&str` source prefixes;
//! `Unit` replaces hand-rolled suffix conventions. Adding a new series
//! is a 3-line change: declare the descriptor here, emit from collector,
//! reference from frontend (or read `/api/v1/series`).
//!
//! The catalog is intentionally a `const` slice so:
//! - invariant tests run at compile time and `cargo test`
//! - the `dump-series-catalog` CLI emits a stable JSON for build-time
//!   TypeScript consumption (no runtime FFI, no duplicate catalog)

use serde::Serialize;
use std::fmt;
use std::sync::OnceLock;

/// Upstream data source. Stable across renames — the enum variant
/// stringifies to the same prefix used in storage keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Source {
    Fred,
    Eia,
    Treasury,
    Comtrade,
    Gscpi,
    Nasa,
    Noaa,
    Quote,
    Sentiment,
}

impl Source {
    /// Storage-key prefix (lowercase). Matches `format!("{prefix}:{raw_id}_{suffix}")`.
    pub fn prefix(self) -> &'static str {
        match self {
            Source::Fred => "fred",
            Source::Eia => "eia",
            Source::Treasury => "treasury",
            Source::Comtrade => "comtrade",
            Source::Gscpi => "gscpi",
            Source::Nasa => "nasa",
            Source::Noaa => "noaa",
            Source::Quote => "quote",
            Source::Sentiment => "sentiment",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.prefix())
    }
}

/// What the value represents. Used by readers to choose formatting
/// (decimal places, suffix symbol) and to validate that emit-side
/// didn't accidentally drop the unit suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Unit {
    /// Percent (e.g. 4.95 = 4.95%). Stored as bare number, no /100.
    Percent,
    /// US Dollars.
    Usd,
    /// US Dollars per barrel (oil).
    Bbl,
    /// Thousands (e.g. PAYEMS_K = nonfarm payrolls in thousands).
    K,
    /// Year-over-year percent change.
    YoyPct,
    /// Spot US Dollars per barrel (compound unit for EIA oil).
    SpotUsdBbl,
    /// Degrees Celsius (NASA temperature anomaly).
    AnomC,
    /// Parts per million (NOAA CO2).
    Ppm,
    /// Generic index level (no unit; reader interprets context).
    Index,
    /// Counts (claims, etc.).
    Count,
    /// Plain string identifier (e.g. ticker symbols).
    Symbol,
}

impl Unit {
    /// Suffix appended to `raw_id` in the normalized storage key.
    /// Compound units concatenate multiple segments with `_`.
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Percent => "_PCT",
            Unit::Usd => "_USD",
            Unit::Bbl => "_BBL",
            Unit::K => "_K",
            Unit::YoyPct => "_YOY",
            Unit::SpotUsdBbl => "_SPOT_USD_BBL",
            Unit::AnomC => "_ANOM_C",
            Unit::Ppm => "_MLO_PPM",
            Unit::Index => "",
            Unit::Count => "",
            Unit::Symbol => "",
        }
    }
}

/// Canonical descriptor for one signal series. The combination
/// `(source, normalized_id)` is globally unique across the catalog
/// (enforced by `series_catalog_invariant::catalog_no_duplicate_normalized_id`).
#[derive(Debug, Clone, Serialize)]
pub struct SeriesDescriptor {
    pub source: Source,
    pub upstream_id: &'static str,
    pub normalized_id: &'static str,
    pub display_name: &'static str,
    pub unit: Unit,
    pub description: &'static str,
}

/// Static catalog. All entries verified at compile time + invariant tests.
///
/// To add a new series:
/// 1. Pick the existing `Source` or add a new variant + `prefix()` arm.
/// 2. Add the `SeriesDescriptor` below.
/// 3. Reference the upstream ID in your collector's HTTP call.
/// 4. Use `normalize(source, upstream_id)` to get the storage key.
///
/// Run `cargo test --lib --test series_catalog_invariant` to verify
/// uniqueness + format invariants.
pub const CATALOG: &[SeriesDescriptor] = &[
    // === FRED (11 series) ===
    // Daily Treasury / yield curve
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "DGS10",
        normalized_id: "fred:DGS10_PCT",
        display_name: "10Y %",
        unit: Unit::Percent,
        description: "10-year Treasury constant maturity rate",
    },
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "DGS2",
        normalized_id: "fred:DGS2_PCT",
        display_name: "2Y %",
        unit: Unit::Percent,
        description: "2-year Treasury constant maturity rate",
    },
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "T10Y2Y",
        normalized_id: "fred:T10Y2Y",
        display_name: "2s10s",
        unit: Unit::Percent,
        description: "10-year minus 2-year Treasury spread",
    },
    // Fed policy + employment
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "FEDFUNDS",
        normalized_id: "fred:FEDFUNDS_PCT",
        display_name: "Fed Funds",
        unit: Unit::Percent,
        description: "Effective federal funds rate",
    },
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "UNRATE",
        normalized_id: "fred:UNRATE_PCT",
        display_name: "Unemployment",
        unit: Unit::Percent,
        description: "Civilian unemployment rate",
    },
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "PAYEMS",
        normalized_id: "fred:PAYEMS_K",
        display_name: "NFP",
        unit: Unit::K,
        description: "All employees, total nonfarm (thousands)",
    },
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "ICSA",
        normalized_id: "fred:ICSA",
        display_name: "IC Claims",
        unit: Unit::Count,
        description: "Initial claims, seasonally adjusted",
    },
    // Inflation
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "CPIAUCSL",
        normalized_id: "fred:CPIAUCSL_YOY",
        display_name: "CPI YoY",
        unit: Unit::YoyPct,
        description: "CPI all urban consumers, year-over-year percent change",
    },
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "AWHE",
        normalized_id: "fred:AWHE_YOY_PCT",
        display_name: "Wages YoY",
        unit: Unit::YoyPct,
        description: "Average hourly earnings, year-over-year percent change",
    },
    // Volatility / credit
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "VIXCLS",
        normalized_id: "fred:VIXCLS",
        display_name: "VIX",
        unit: Unit::Index,
        description: "CBOE Volatility Index (VIX close)",
    },
    SeriesDescriptor {
        source: Source::Fred,
        upstream_id: "BAMLH0A0HYM2",
        normalized_id: "fred:HY_OAS_PCT",
        display_name: "HY OAS",
        unit: Unit::Percent,
        description: "ICE BofA US High Yield Index option-adjusted spread",
    },

    // === EIA (2 series) ===
    // upstream_id is EIA's own facet ID (used in API queries). The same
    // commodity is also published by FRED under DCOILWTICO / DCOILBRENTEU
    // but those are NOT upstream_ids for the EIA collector — cross-source
    // dedup is left to readers (e.g. quant agents can join on display_name).
    SeriesDescriptor {
        source: Source::Eia,
        upstream_id: "RWTC",
        normalized_id: "eia:WTI_SPOT_USD_BBL",
        display_name: "WTI $",
        unit: Unit::SpotUsdBbl,
        description: "WTI spot price, USD per barrel (EIA facet RWTC)",
    },
    SeriesDescriptor {
        source: Source::Eia,
        upstream_id: "RBRTE",
        normalized_id: "eia:BRENT_SPOT_USD_BBL",
        display_name: "Brent $",
        unit: Unit::SpotUsdBbl,
        description: "Brent spot price, USD per barrel (EIA facet RBRTE)",
    },

    // === Treasury (2 series) ===
    SeriesDescriptor {
        source: Source::Treasury,
        upstream_id: "TOTAL_DEBT",
        normalized_id: "treasury:TOTAL_DEBT_USD",
        display_name: "Total Debt",
        unit: Unit::Usd,
        description: "Total US public debt outstanding, USD",
    },
    SeriesDescriptor {
        source: Source::Treasury,
        upstream_id: "AVG_RATE_MARKETABLE",
        normalized_id: "treasury:AVG_RATE_MARKETABLE_PCT",
        display_name: "Avg Rate",
        unit: Unit::Percent,
        description: "Average interest rate on marketable Treasury debt",
    },

    // === GSCPI (1 series) ===
    SeriesDescriptor {
        source: Source::Gscpi,
        upstream_id: "index",
        normalized_id: "gscpi:index",
        display_name: "GSCPI",
        unit: Unit::Index,
        description: "Geopolitical Supply Chain Pressure Index",
    },

    // === NASA (1 series) ===
    SeriesDescriptor {
        source: Source::Nasa,
        upstream_id: "GISTEMP",
        normalized_id: "nasa:GISTEMP_ANOM_C",
        display_name: "Temp Anomaly",
        unit: Unit::AnomC,
        description: "GISTEMP global temperature anomaly, Celsius",
    },

    // === NOAA (1 series) ===
    SeriesDescriptor {
        source: Source::Noaa,
        upstream_id: "CO2_MLO",
        normalized_id: "noaa:CO2_MLO_PPM",
        display_name: "CO2 ppm",
        unit: Unit::Ppm,
        description: "Mauna Loa CO2 concentration, ppm",
    },

    // === Quote / Sentiment — watchlist-driven (dynamic) ===
    // No static catalog entries. Collectors use `series::build_dynamic(Source::Quote, symbol, Unit::Usd)`
    // to construct storage keys like `quote:AAPL` / `sentiment:BTCUSD`.
    // The catalog exposes the Source enum and prefix() so the format
    // is locked even though individual IDs are not.

    // === Comtrade — query-driven but enumerated ===
    // Each entry is a fixed (reporter, partner, commodity, direction) flow.
    // The upstream_id encodes all four so the catalog entry is self-contained.
    // Adding a new flow is a 1-line catalog edit + 1-tuple edit in comtrade.rs.
    // Storage suffix `_usd` is literal (lower-case, no Unit::Usd suffix);
    // we use Unit::Symbol so build_dynamic() doesn't add an extra `_USD` on top.
    SeriesDescriptor {
        source: Source::Comtrade,
        upstream_id: "CN.exp.semiconductors_usd",
        normalized_id: "comtrade:CN.exp.semiconductors_usd",
        display_name: "CN semi exports",
        unit: Unit::Symbol,
        description: "China semiconductor exports, USD",
    },
    SeriesDescriptor {
        source: Source::Comtrade,
        upstream_id: "TW.exp.semiconductors_usd",
        normalized_id: "comtrade:TW.exp.semiconductors_usd",
        display_name: "TW semi exports",
        unit: Unit::Symbol,
        description: "Taiwan semiconductor exports, USD",
    },
    SeriesDescriptor {
        source: Source::Comtrade,
        upstream_id: "KR.exp.semiconductors_usd",
        normalized_id: "comtrade:KR.exp.semiconductors_usd",
        display_name: "KR semi exports",
        unit: Unit::Symbol,
        description: "Korea semiconductor exports, USD",
    },
    SeriesDescriptor {
        source: Source::Comtrade,
        upstream_id: "US.imp.crude_usd",
        normalized_id: "comtrade:US.imp.crude_usd",
        display_name: "US crude imports",
        unit: Unit::Symbol,
        description: "US crude oil imports, USD",
    },
    SeriesDescriptor {
        source: Source::Comtrade,
        upstream_id: "CN.imp.gold_usd",
        normalized_id: "comtrade:CN.imp.gold_usd",
        display_name: "CN gold imports",
        unit: Unit::Symbol,
        description: "China gold imports, USD",
    },
    SeriesDescriptor {
        source: Source::Comtrade,
        upstream_id: "DE.exp.arms_usd",
        normalized_id: "comtrade:DE.exp.arms_usd",
        display_name: "DE arms exports",
        unit: Unit::Symbol,
        description: "Germany arms exports, USD",
    },
];

/// Normalize an upstream `(source, upstream_id)` pair into the storage
/// key. Returns `None` if the pair isn't catalogued.
///
/// Collectors use this to build the `series` field on `Observation`:
/// ```ignore
/// let series = series::normalize(Source::Fred, "DGS10").unwrap();
/// // → "fred:DGS10_PCT"
/// ```
pub fn normalize(source: Source, upstream_id: &str) -> Option<&'static str> {
    CATALOG
        .iter()
        .find(|d| d.source == source && d.upstream_id == upstream_id)
        .map(|d| d.normalized_id)
}

/// Build a normalized key from parts (for dynamic sources like Comtrade
/// where the catalog doesn't enumerate every triple).
///
/// Format: `{prefix}:{upstream_id}_{unit_suffix}`.
///
/// Example:
/// `build_dynamic(Source::Comtrade, "CN.exp.semiconductors", Unit::Usd)`
/// → `"comtrade:CN.exp.semiconductors_USD"`
pub fn build_dynamic(source: Source, upstream_id: &str, unit: Unit) -> String {
    format!("{}{}:{}{}", source.prefix(), "", upstream_id, unit.suffix())
    // (the empty "" trick keeps rustfmt happy with the trailing colon)
}

/// All catalog entries for a given source.
pub fn for_source(source: Source) -> impl Iterator<Item = &'static SeriesDescriptor> {
    CATALOG.iter().filter(move |d| d.source == source)
}

/// Dump the catalog as a JSON array to stdout. Used by the
/// `core/hub dump-series-catalog` CLI subcommand, which is invoked
/// by `build-console.sh` before `npm run build` to seed
/// `console/src/lib/series_catalog.json`.
pub fn dump_catalog_json() -> anyhow::Result<()> {
    let entries: Vec<_> = CATALOG
        .iter()
        .map(|d| {
            serde_json::json!({
                "source": d.source.prefix(),
                "upstream_id": d.upstream_id,
                "normalized_id": d.normalized_id,
                "display_name": d.display_name,
                "unit": format!("{:?}", d.unit),
                "description": d.description,
            })
        })
        .collect();
    let out = serde_json::json!({
        "count": entries.len(),
        "series": entries,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Process-wide cached count. Avoids repeated scans when the inventory
/// endpoint is hit on every overview render.
pub fn count() -> usize {
    static CACHE: OnceLock<usize> = OnceLock::new();
    *CACHE.get_or_init(|| CATALOG.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_returns_expected_keys() {
        assert_eq!(normalize(Source::Fred, "DGS10"), Some("fred:DGS10_PCT"));
        assert_eq!(normalize(Source::Fred, "VIXCLS"), Some("fred:VIXCLS"));
        assert_eq!(normalize(Source::Eia, "DCOILWTICO"), Some("eia:WTI_SPOT_USD_BBL"));
        assert_eq!(normalize(Source::Fred, "NONEXISTENT"), None);
    }

    #[test]
    fn prefix_matches_storage_format() {
        assert_eq!(Source::Fred.prefix(), "fred");
        assert_eq!(Source::Eia.prefix(), "eia");
        assert_eq!(Source::Quote.prefix(), "quote");
    }

    #[test]
    fn unit_suffix_is_stable() {
        assert_eq!(Unit::Percent.suffix(), "_PCT");
        assert_eq!(Unit::SpotUsdBbl.suffix(), "_SPOT_USD_BBL");
        assert_eq!(Unit::Index.suffix(), "");
    }

    #[test]
    fn build_dynamic_formats_correctly() {
        assert_eq!(
            build_dynamic(Source::Comtrade, "CN.exp.semiconductors", Unit::Usd),
            "comtrade:CN.exp.semiconductors_USD"
        );
        assert_eq!(
            build_dynamic(Source::Quote, "AAPL", Unit::Usd),
            "quote:AAPL_USD"
        );
    }

    #[test]
    fn catalog_count_matches_inventory() {
        // 11 FRED + 2 EIA + 2 Treasury + 1 GSCPI + 1 NASA + 1 NOAA + 6 Comtrade = 24.
        // Quote/Sentiment are dynamic (build_dynamic at emit time).
        assert_eq!(CATALOG.len(), 24, "catalog grew — update spec/inventory check");
    }
}