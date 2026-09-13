//! Entity seeder (e2e audit Sept 2026) — without this the graph is
//! essentially empty (3 test entities from acceptance probes), so
//! `find_path`, `query_relationship`, and `create_relationship` have
//! nothing real to connect.
//!
//! Strategy: idempotent seed run on hub-core startup. Pulls distinct
//! source / series / geo prefixes from existing data and a hand-curated
//! OSINT rosters list, calling the same `create_entity` path the MCP
//! tool uses (ON CONFLICT kind+name DO UPDATE → safe to re-run).
//!
//! Triggers:
//!   * server.rs `serve()` calls `seed_all(state)` once at boot
//!   * `HUB_SEED_ENABLED` env (default true) gates the whole thing for
//!     clean-test deployments
//!
//! The roster is deliberately small and conservative: only entities the
//! hub already knows about (collectors, series publishers, common OSINT
//! orgs). Anything NLP-derived should live in a separate, opt-in
//! extractor — out of scope for the audit fix.

use std::collections::BTreeSet;
use std::time::Instant;

use serde_json::{json, Value};

use crate::error::Result;
use crate::state::AppState;

const SEED_ACTOR: &str = "system:entity_seeder";

/// Seed entities from existing collectors, signal series, geo sources,
/// and a curated OSINT roster. Idempotent — safe to re-run on every boot.
pub async fn seed_all(state: &AppState) -> Result<usize> {
    if !state.config.seed_enabled {
        tracing::info!("entity seeder disabled (HUB_SEED_ENABLED != true)");
        return Ok(0);
    }

    let started = Instant::now();
    let mut inserted = 0usize;
    let mut skipped = 0usize;

    // ---- 1. collector / source roster -----------------------------------
    // Monitor collector names (from registry) → org entities. These are
    // the publishers of the signals we ingest, so attaching aliases helps
    // future NLP linkers find them in document text.
    for collector in collector_orgs() {
        match upsert(state, "org", collector.name, Some(collector.aliases.iter().map(|s| s.to_string()).collect()), Some(collector.attributes)).await {
            Ok(true) => inserted += 1,
            Ok(false) => skipped += 1,
            Err(e) => tracing::warn!(error = %e, name = collector.name, "seed: collector org failed"),
        }
    }

    // ---- 2. distinct signal_observations prefixes -----------------------
    // Already-running signal series imply their publishers: fred:* → St.
    // Louis Fed, eia:* → US EIA, noaa:* → NOAA GML, nasa:* → NASA GISS,
    // gscpi:* → Caldara & Iacoviello, comtrade:* → UN Comtrade, quote:*
    // → exchange, sentiment:* → news/reddit mix.
    let prefixes: Vec<(&str, &str, &[&str])> = vec![
        ("fred", "Federal Reserve Bank of St. Louis (FRED)", &["FRED", "St. Louis Fed", "fred.stlouisfed.org"]),
        ("eia",  "U.S. Energy Information Administration", &["EIA", "eia.gov"]),
        ("noaa", "NOAA Global Monitoring Laboratory", &["NOAA GML", "gml.noaa.gov"]),
        ("nasa", "NASA Goddard Institute for Space Studies", &["NASA GISS", "data.giss.nasa.gov"]),
        ("gscpi","Geopolitical Risk index (Caldara & Iacoviello)", &["GSCPI", "matteoiacoviello.com"]),
        ("comtrade", "UN Comtrade Database", &["UN Comtrade", "comtradeplus.un.org"]),
        ("quote", "Market data aggregator (FMP / Finnhub / Stooq)", &["FMP", "Finnhub", "Stooq"]),
        ("sentiment", "News + social sentiment aggregator", &["sentiment"]),
    ];
    for (prefix, name, aliases) in prefixes {
        match upsert(state, "org", name, Some(aliases.iter().map(|s| s.to_string()).collect()), Some(json!({
            "source_prefix": prefix,
            "seed_origin": "signal_observations",
            "description": format!("Publisher inferred from signal_observations prefix '{}:*'", prefix),
        }))).await {
            Ok(true) => inserted += 1,
            Ok(false) => skipped += 1,
            Err(e) => tracing::warn!(error = %e, name, "seed: signal-prefix org failed"),
        }
    }

    // ---- 3. distinct geo_events.source values ---------------------------
    // Each monitor source that emits geo events becomes an entity so
    // `query_entity(source=...)` and `find_path` can use them as hubs.
    let geo_sources: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT source FROM geo_events WHERE source IS NOT NULL ORDER BY source",
    )
    .fetch_all(&state.pg)
    .await
    .unwrap_or_default();
    for (src,) in geo_sources {
        let kind = source_kind_hint(&src);
        match upsert(state, kind, &src, Some(vec![]), Some(json!({
            "seed_origin": "geo_events.source",
        }))).await {
            Ok(true) => inserted += 1,
            Ok(false) => skipped += 1,
            Err(e) => tracing::warn!(error = %e, name = src, "seed: geo source entity failed"),
        }
    }
    // Some series encode their institution in the name (e.g. WHO, IEA,
    // OPEC, EIA, NASA). Only seed the institution hint when we can
    // confidently name it; otherwise leave it.
    let series_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT series FROM signal_observations ORDER BY series",
    )
    .fetch_all(&state.pg)
    .await
    .unwrap_or_default();
    for (series,) in series_rows {
        if let Some((name, aliases)) = institution_hint(&series) {
            match upsert(state, "org", name, Some(aliases), Some(json!({
                "seed_origin": "signal_observations.series",
                "source_series": series,
            }))).await {
                Ok(true) => inserted += 1,
                Ok(false) => skipped += 1,
                Err(e) => tracing::warn!(error = %e, name, "seed: institution hint failed"),
            }
        }
    }

    // ---- 5. hand-curated OSINT roster -----------------------------------
    // Common entities the operator regularly investigates. Conservative:
    // these are widely-cited and unlikely to be controversial to insert.
    for entry in curated_roster() {
        match upsert(state, entry.kind, entry.name, Some(entry.aliases), Some(entry.attributes)).await {
            Ok(true) => inserted += 1,
            Ok(false) => skipped += 1,
            Err(e) => tracing::warn!(error = %e, name = entry.name, "seed: curated entity failed"),
        }
    }

    tracing::info!(
        inserted,
        skipped,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "entity seeder: done"
    );
    Ok(inserted)
}

async fn upsert(
    state: &AppState,
    kind: &str,
    name: &str,
    aliases: Option<Vec<String>>,
    attributes: Option<Value>,
) -> Result<bool> {
    let intent = crate::graphw::EntityIntent {
        kind: kind.to_string(),
        name: name.to_string(),
        aliases,
        attributes,
    };
    let out = crate::graphw::create_entity(state, SEED_ACTOR, intent).await?;
    let id = out.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
    // create_entity returns the id whether insert or update happened;
    // for the "inserted" count we ask PG directly.
    let existed: (bool,) = sqlx::query_as(
        "SELECT (created_at = updated_at) AS fresh FROM entities WHERE entity_id::text = $1",
    )
    .bind(id)
    .fetch_one(&state.pg)
    .await
    .unwrap_or((false,));
    Ok(existed.0)
}

// ---- roster helpers (data, no IO) ---------------------------------------

struct Org<'a> {
    name: &'a str,
    aliases: &'a [&'a str],
    attributes: Value,
}

fn collector_orgs() -> Vec<Org<'static>> {
    // Static list: derived from registry() at compile time would be nicer,
    // but the seeder runs in hub-core which doesn't depend on monitor
    // modules' private types. Hard-coding the always-on orgs is fine; if
    // a new collector is added the maintainer can extend this list.
    vec![
        Org { name: "NASA EONET",        aliases: &["EONET", "eonet.gsfc.nasa.gov"],           attributes: json!({"domain": "earth observation", "seed_origin": "collector"}) },
        Org { name: "NOAA",              aliases: &["National Oceanic and Atmospheric Administration", "noaa.gov"], attributes: json!({"domain": "weather", "seed_origin": "collector"}) },
        Org { name: "USGS",              aliases: &["U.S. Geological Survey", "earthquake.usgs.gov"], attributes: json!({"domain": "earthquakes", "seed_origin": "collector"}) },
        Org { name: "NASA FIRMS",        aliases: &["Fire Information for Resource Management System", "firms.modaps.eosdis.nasa.gov"], attributes: json!({"domain": "fires", "seed_origin": "collector"}) },
        Org { name: "OpenSky Network",   aliases: &["OpenSky", "opensky-network.org"],         attributes: json!({"domain": "aviation", "seed_origin": "collector"}) },
        Org { name: "WHO",               aliases: &["World Health Organization", "who.int"],   attributes: json!({"domain": "health", "seed_origin": "collector"}) },
        Org { name: "CISA",              aliases: &["Cybersecurity and Infrastructure Security Agency", "cisa.gov"], attributes: json!({"domain": "cyber", "seed_origin": "collector"}) },
        Org { name: "OFAC",              aliases: &["Office of Foreign Assets Control", "ofac.treasury.gov"], attributes: json!({"domain": "sanctions", "seed_origin": "collector"}) },
        Org { name: "USAspending",       aliases: &["USAspending.gov"],                        attributes: json!({"domain": "finance", "seed_origin": "collector"}) },
        Org { name: "EPA RadNet",        aliases: &["EPA RadNet", "epa.gov/radnet"],           attributes: json!({"domain": "radiation", "seed_origin": "collector"}) },
        Org { name: "KiwiSDR",           aliases: &["kiwisdr.com"],                            attributes: json!({"domain": "rf", "seed_origin": "collector"}) },
        Org { name: "GDELT Project",     aliases: &["GDELT", "gdeltproject.org"],              attributes: json!({"domain": "events", "seed_origin": "collector"}) },
        Org { name: "ReliefWeb",         aliases: &["reliefweb.int"],                          attributes: json!({"domain": "humanitarian", "seed_origin": "collector"}) },
        Org { name: "ACLED",             aliases: &["Armed Conflict Location & Event Data Project", "acleddata.com"], attributes: json!({"domain": "conflict", "seed_origin": "collector"}) },
        Org { name: "BLS",               aliases: &["U.S. Bureau of Labor Statistics", "bls.gov"], attributes: json!({"domain": "labor", "seed_origin": "collector"}) },
        Org { name: "U.S. Treasury",     aliases: &["home.treasury.gov"],                      attributes: json!({"domain": "rates", "seed_origin": "collector"}) },
        Org { name: "EIA",               aliases: &["U.S. Energy Information Administration", "eia.gov"], attributes: json!({"domain": "energy", "seed_origin": "collector"}) },
        Org { name: "FRED",              aliases: &["Federal Reserve Economic Data", "fred.stlouisfed.org"], attributes: json!({"domain": "macro", "seed_origin": "collector"}) },
    ]
}

fn source_kind_hint(src: &str) -> &'static str {
    // Heuristic: most monitor sources are publishers → org. Geo locations
    // (lat, lon) tagged via source naming (e.g. "usgs_eq") should be the
    // source org, not a location. Locations are handled separately by
    // investigators creating `location` entities from geo_events.
    if src.starts_with("monitor:") { "event" } else { "org" }
}

fn institution_hint(series: &str) -> Option<(&'static str, Vec<String>)> {
    // Series-name → institution hint. Conservative: only insert when the
    // mapping is unambiguous and the entity doesn't already exist with
    // (kind=org, name=<institution>). The upsert is idempotent so it's
    // safe; this filter just keeps the seeder lean.
    let s = series.to_ascii_lowercase();
    if s.starts_with("fred:") {
        Some(("Federal Reserve System", vec!["Fed".into(), "Federal Reserve".into(), "federalreserve.gov".into()]))
    } else if s.starts_with("eia:") {
        Some(("U.S. Energy Information Administration", vec!["EIA".into(), "eia.gov".into()]))
    } else if s.starts_with("noaa:") {
        Some(("NOAA Global Monitoring Laboratory", vec!["NOAA GML".into(), "gml.noaa.gov".into()]))
    } else if s.starts_with("nasa:") {
        Some(("NASA Goddard Institute for Space Studies", vec!["NASA GISS".into(), "data.giss.nasa.gov".into()]))
    } else if s.starts_with("gscpi:") {
        Some(("Geopolitical Risk Index (Caldara & Iacoviello)", vec!["GSCPI".into()]))
    } else if s.starts_with("comtrade:") {
        Some(("United Nations Comtrade", vec!["UN Comtrade".into(), "comtradeplus.un.org".into()]))
    } else {
        None
    }
}

struct Curated<'a> {
    kind: &'a str,
    name: &'a str,
    aliases: Vec<String>,
    attributes: Value,
}

fn curated_roster() -> Vec<Curated<'static>> {
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    vec![
        Curated { kind: "org",       name: "United Nations",  aliases: a(&["UN", "un.org"]),
                  attributes: json!({"description": "International intergovernmental organization"}) },
        Curated { kind: "org",       name: "International Court of Justice", aliases: a(&["ICJ", "icj-cij.org"]),
                  attributes: json!({"description": "Principal judicial organ of the UN"}) },
        Curated { kind: "org",       name: "International Criminal Court", aliases: a(&["ICC", "icc-cpi.int"]),
                  attributes: json!({"description": "Permanent international tribunal for war crimes / crimes against humanity"}) },
        Curated { kind: "org",       name: "NATO", aliases: a(&["North Atlantic Treaty Organization", "nato.int"]),
                  attributes: json!({"description": "Military alliance of 32 North-Atlantic and European states"}) },
        Curated { kind: "org",       name: "European Union", aliases: a(&["EU", "europa.eu"]),
                  attributes: json!({"description": "Political and economic union of 27 member states"}) },
        Curated { kind: "org",       name: "BRICS", aliases: a(&["brics-russia-2024", "brics.bz"]),
                  attributes: json!({"description": "Intergovernmental organization of major emerging economies"}) },
        Curated { kind: "org",       name: "International Monetary Fund", aliases: a(&["IMF", "imf.org"]),
                  attributes: json!({"description": "UN specialized agency for global monetary cooperation"}) },
        Curated { kind: "org",       name: "World Bank", aliases: a(&["worldbank.org"]),
                  attributes: json!({"description": "UN specialized agency for development finance"}) },
        Curated { kind: "org",       name: "World Health Organization", aliases: a(&["WHO", "who.int"]),
                  attributes: json!({"description": "UN specialized agency for public health"}) },
        Curated { kind: "org",       name: "International Committee of the Red Cross", aliases: a(&["ICRC", "icrc.org"]),
                  attributes: json!({"description": "Humanitarian organization operating in conflict zones"}) },
        Curated { kind: "org",       name: "Internet Corporation for Assigned Names and Numbers", aliases: a(&["ICANN", "icann.org"]),
                  attributes: json!({"description": "Coordinates DNS / IP allocation globally"}) },
        // Common OSINT locations investigators care about
        Curated { kind: "location", name: "Chernobyl Exclusion Zone", aliases: a(&["Chernobyl", "ChNPP"]),
                  attributes: json!({"country": "UA", "description": "30km exclusion zone around former ChNPP"}) },
        Curated { kind: "location", name: "Fukushima Daiichi Nuclear Plant", aliases: a(&["Fukushima Daiichi", "1F"]),
                  attributes: json!({"country": "JP", "description": "Nuclear plant, 2011 meltdown site"}) },
        Curated { kind: "location", name: "Strait of Hormuz", aliases: a(&["Hormuz"]),
                  attributes: json!({"country": "OM/IR", "description": "Strategic chokepoint between the Persian Gulf and Gulf of Oman"}) },
        Curated { kind: "location", name: "Taiwan Strait", aliases: a(&["Formosa Strait"]),
                  attributes: json!({"description": "Body of water between Taiwan and mainland China"}) },
    ]
}

// Tag the entities we touched this run, so future runs can do an
// incremental diff instead of full upsert (deferred — current upsert is
// cheap given the small roster).
#[allow(dead_code)]
fn touched_set() -> BTreeSet<String> {
    BTreeSet::new()
}