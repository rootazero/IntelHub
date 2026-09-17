//! GEV P3 T4: military installations REST endpoint.
//!
//! `GET /api/v1/gev/installations?south&west&north&east[&exact=1]` serves the
//! PG `military_installations` catalog (T3 collector, migration 0020) as an
//! Overpass-shaped element envelope — the console adapter's path rewrite maps
//! the engine's `/api/military-installations` onto this route, and the engine
//! normalizes the elements client-side (contracts.md §4). The table stores
//! normalized rows (one center lat/lon + optional bounds/geometry/tags), so
//! this layer recomposes the original element shape: nodes carry `lat`/`lon`,
//! ways/relations carry `bounds` + `geometry` when harvested.
//!
//! Envelope contract (§4): `{elements, retrievedAt, status, saturated,
//! elementCap}`; non-2xx bodies are `{error, reason}` with reason ∈
//! {rate_limited, timeout, query_failed, unavailable}. Invalid bboxes are a
//! client programming error → 400 `query_failed` (the engine validates too,
//! source.js:19-30, so this only fires on hand-rolled callers).

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::AppState;

/// Contracts.md §4: the engine infers saturation as
/// `elements.length >= elementCap` when the explicit flag is absent
/// (source.js:4-10), so the cap must always be advertised.
pub const ELEMENT_CAP: usize = 2000;
/// 24h harvest cadence (T3) + 6h grace. Older than this and the client chip
/// switches to "Serving cached mapped context" (ingestion.js:139-148).
pub const STALE_AFTER_HOURS: i64 = 30;

#[derive(Deserialize)]
pub struct InstallationsQuery {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
    /// `exact=1` is the engine's re-ask after a saturated snapped-tile
    /// response. Our catalog is harvested globally and always queried at the
    /// precise bbox, so the flag is accepted for contract compatibility and
    /// intentionally changes nothing.
    pub exact: Option<String>,
}

/// One PG row of `military_installations`, in the SELECT column order below.
pub struct InstallationRow {
    pub osm_type: String,
    pub osm_id: i64,
    pub name: Option<String>,
    pub mil_class: String,
    pub lat: f64,
    pub lon: f64,
    pub minlat: Option<f64>,
    pub minlon: Option<f64>,
    pub maxlat: Option<f64>,
    pub maxlon: Option<f64>,
    pub geometry: Option<Value>,
    pub tags: Value,
}

/// bbox validation, contract §4 / engine source.js:19-30 mirrored: finite,
/// ±90/±180, north>south, east>west, ≤10°×10°. (No antimeridian bboxes:
/// east>west plus the 10° cap keeps `lon BETWEEN west AND east` correct.)
pub fn validate_bbox(south: f64, west: f64, north: f64, east: f64) -> Result<(), &'static str> {
    if ![south, west, north, east].iter().all(|v| v.is_finite()) {
        return Err("bbox values must be finite");
    }
    if !(-90.0..=90.0).contains(&south) || !(-90.0..=90.0).contains(&north) {
        return Err("latitude out of range");
    }
    if !(-180.0..=180.0).contains(&west) || !(-180.0..=180.0).contains(&east) {
        return Err("longitude out of range");
    }
    if north <= south {
        return Err("north must be greater than south");
    }
    if east <= west {
        return Err("east must be greater than west");
    }
    if north - south > 10.0 {
        return Err("bbox latitude span exceeds 10 degrees");
    }
    if east - west > 10.0 {
        return Err("bbox longitude span exceeds 10 degrees");
    }
    Ok(())
}

/// Merge the harvested original Overpass `tags` jsonb (priority — T3 stores
/// them verbatim) with the derived `mil_class` / `name` columns. The original
/// tags always carry the military signal for rows the T3 classifier kept, so
/// synthesis is a defensive fallback for a truncated/missing jsonb, not a
/// re-derivation path.
pub fn merged_tags(row: &InstallationRow) -> Value {
    let mut tags = match &row.tags {
        Value::Object(_) => row.tags.clone(),
        _ => json!({}),
    };
    let has_military_signal = tags.get("military").is_some()
        || tags.get("landuse").and_then(Value::as_str) == Some("military");
    if !has_military_signal {
        if row.mil_class == "military_land" {
            tags["landuse"] = json!("military");
        } else {
            tags["military"] = json!(row.mil_class);
        }
    }
    if tags.get("name").is_none() && tags.get("name:en").is_none() {
        if let Some(name) = &row.name {
            tags["name"] = json!(name);
        }
    }
    tags
}

/// Recompose a normalized table row back into the Overpass element shape the
/// client's normalize consumes (contracts.md §4): node → `lat`/`lon`;
/// way/relation → `bounds`/`geometry` when harvested (absent → field omitted,
/// the client's bounds-midpoint/center fallbacks then apply or drop).
pub fn row_to_element(row: &InstallationRow) -> Value {
    let mut el = json!({ "type": row.osm_type, "id": row.osm_id });
    if row.osm_type == "node" {
        el["lat"] = json!(row.lat);
        el["lon"] = json!(row.lon);
    } else {
        if let (Some(a), Some(o), Some(c), Some(d)) =
            (row.minlat, row.minlon, row.maxlat, row.maxlon)
        {
            el["bounds"] = json!({ "minlat": a, "minlon": o, "maxlat": c, "maxlon": d });
        }
        if let Some(geometry) = &row.geometry {
            el["geometry"] = geometry.clone();
        }
    }
    el["tags"] = merged_tags(row);
    el
}

/// Staleness is judged on the OLDEST row of the table (task contract: "表空或
/// 最老 fetched_at >30h"): a partial T3 round keeps the failed quadrant's old
/// rows mixed with fresh ones, and min() is what surfaces that decay. (A fully
/// successful round rewrites every row to the same round_ts, so min == max
/// there and both readings agree.)
pub fn is_stale(oldest: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match oldest {
        None => true, // empty table — nothing to serve
        Some(t) => now.signed_duration_since(t) > ChronoDuration::hours(STALE_AFTER_HOURS),
    }
}

/// Truncate to the element cap. Fetch `cap + 1` upstream so an exactly-full
/// page is distinguishable from a truncated one (`saturated` semantics §4:
/// true → the client re-asks with `exact=1`).
pub fn cap_elements(elements: Vec<Value>, cap: usize) -> (Vec<Value>, bool) {
    if elements.len() > cap {
        (elements.into_iter().take(cap).collect(), true)
    } else {
        (elements, false)
    }
}

fn install_err(status: StatusCode, error: &str, reason: &str) -> Response {
    (status, Json(json!({ "error": error, "reason": reason }))).into_response()
}

pub async fn gev_installations(
    State(state): State<Arc<AppState>>,
    params: Result<Query<InstallationsQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<Value>, Response> {
    let Query(q) = params.map_err(|_| {
        install_err(
            StatusCode::BAD_REQUEST,
            "missing or malformed south/west/north/east parameters",
            "query_failed",
        )
    })?;
    validate_bbox(q.south, q.west, q.north, q.east).map_err(|msg| {
        install_err(StatusCode::BAD_REQUEST, msg, "query_failed")
    })?;

    type Row = (
        String,
        i64,
        Option<String>,
        String,
        f64,
        f64,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<Value>,
        Value,
    );
    // Center-in-bbox OR bounds-overlap: a way whose footprint crosses the
    // viewport but whose center sits outside must still be listed — the
    // client applies its own precise viewport filter (viewport.js).
    let rows = sqlx::query_as::<_, Row>(
        r#"SELECT osm_type, osm_id, name, mil_class, lat, lon,
                  minlat, minlon, maxlat, maxlon, geometry, tags
             FROM military_installations
            WHERE (lat >= $1 AND lat <= $2 AND lon >= $3 AND lon <= $4)
               OR (minlat IS NOT NULL AND minlon IS NOT NULL
                   AND maxlat IS NOT NULL AND maxlon IS NOT NULL
                   AND minlat <= $2 AND maxlat >= $1
                   AND minlon <= $4 AND maxlon >= $3)
            ORDER BY osm_type, osm_id
            LIMIT $5"#,
    )
    .bind(q.south)
    .bind(q.north)
    .bind(q.west)
    .bind(q.east)
    .bind(ELEMENT_CAP as i64 + 1)
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "gev installations query failed");
        install_err(
            StatusCode::SERVICE_UNAVAILABLE,
            "installations catalog unavailable",
            "unavailable",
        )
    })?;

    let rows = rows
        .iter()
        .map(|r| InstallationRow {
            osm_type: r.0.clone(),
            osm_id: r.1,
            name: r.2.clone(),
            mil_class: r.3.clone(),
            lat: r.4,
            lon: r.5,
            minlat: r.6,
            minlon: r.7,
            maxlat: r.8,
            maxlon: r.9,
            geometry: r.10.clone(),
            tags: r.11.clone(),
        })
        .collect::<Vec<_>>();
    let (elements, saturated) =
        cap_elements(rows.iter().map(row_to_element).collect(), ELEMENT_CAP);

    let (oldest, newest): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT min(fetched_at), max(fetched_at) FROM military_installations")
            .fetch_one(&state.pg)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "gev installations freshness probe failed");
                install_err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "installations catalog unavailable",
                    "unavailable",
                )
            })?;
    let now = Utc::now();

    Ok(Json(json!({
        "elements": elements,
        "retrievedAt": newest
            .map(|t| t.to_rfc3339_opts(SecondsFormat::Micros, true))
            .unwrap_or_else(|| now.to_rfc3339_opts(SecondsFormat::Micros, true)),
        "status": if is_stale(oldest, now) { "stale" } else { "ok" },
        "saturated": saturated,
        "elementCap": ELEMENT_CAP,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        osm_type: &str,
        osm_id: i64,
        name: Option<&str>,
        mil_class: &str,
        lat: f64,
        lon: f64,
        bounds: Option<(f64, f64, f64, f64)>,
        geometry: Option<Value>,
        tags: Value,
    ) -> InstallationRow {
        InstallationRow {
            osm_type: osm_type.to_string(),
            osm_id,
            name: name.map(str::to_string),
            mil_class: mil_class.to_string(),
            lat,
            lon,
            minlat: bounds.map(|b| b.0),
            minlon: bounds.map(|b| b.1),
            maxlat: bounds.map(|b| b.2),
            maxlon: bounds.map(|b| b.3),
            geometry,
            tags,
        }
    }

    // ---- bbox validation: every contract branch ----

    #[test]
    fn bbox_accepts_valid_viewports() {
        assert!(validate_bbox(37.5, -122.5, 38.5, -121.5).is_ok());
        assert!(validate_bbox(-90.0, -180.0, -80.0, -170.0).is_ok());
        assert!(validate_bbox(80.0, 170.0, 90.0, 180.0).is_ok());
        // exactly 10° spans are allowed (engine uses > 10)
        assert!(validate_bbox(0.0, 0.0, 10.0, 10.0).is_ok());
    }

    #[test]
    fn bbox_rejects_non_finite() {
        assert!(validate_bbox(f64::NAN, 0.0, 1.0, 1.0).is_err());
        assert!(validate_bbox(0.0, f64::INFINITY, 1.0, 1.0).is_err());
        assert!(validate_bbox(0.0, 0.0, f64::NEG_INFINITY, 1.0).is_err());
    }

    #[test]
    fn bbox_rejects_out_of_range() {
        assert!(validate_bbox(-91.0, 0.0, 0.0, 1.0).is_err()); // south < -90
        assert!(validate_bbox(0.0, 0.0, 91.0, 1.0).is_err()); // north > 90
        assert!(validate_bbox(0.0, -181.0, 1.0, 1.0).is_err()); // west < -180
        assert!(validate_bbox(0.0, 0.0, 1.0, 181.0).is_err()); // east > 180
    }

    #[test]
    fn bbox_rejects_inverted_and_degenerate() {
        assert!(validate_bbox(38.0, -122.0, 37.0, -121.0).is_err()); // north <= south
        assert!(validate_bbox(37.0, -121.0, 38.0, -122.0).is_err()); // east <= west
        assert!(validate_bbox(37.0, -122.0, 37.0, -121.0).is_err()); // zero height
        assert!(validate_bbox(37.0, -122.0, 38.0, -122.0).is_err()); // zero width
    }

    #[test]
    fn bbox_rejects_oversized_spans() {
        assert!(validate_bbox(0.0, 0.0, 10.1, 5.0).is_err()); // >10° lat
        assert!(validate_bbox(0.0, 0.0, 5.0, 10.1).is_err()); // >10° lon
        assert!(validate_bbox(-90.0, -180.0, 90.0, 180.0).is_err()); // whole globe
    }

    // ---- element recombination ----

    #[test]
    fn node_element_carries_lat_lon_only() {
        let el = row_to_element(&row(
            "node",
            123,
            Some("CFB Esquimalt"),
            "naval_base",
            48.37,
            -124.9,
            None,
            None,
            json!({"military": "naval_base", "name": "CFB Esquimalt"}),
        ));
        assert_eq!(el["type"], json!("node"));
        assert_eq!(el["id"], json!(123));
        assert_eq!(el["lat"], json!(48.37));
        assert_eq!(el["lon"], json!(-124.9));
        assert_eq!(el["tags"]["name"], json!("CFB Esquimalt"));
        // nodes have no footprint — bounds/geometry keys must be absent
        assert!(el.get("bounds").is_none());
        assert!(el.get("geometry").is_none());
    }

    #[test]
    fn way_element_recombines_bounds_and_geometry() {
        let el = row_to_element(&row(
            "way",
            456,
            None,
            "range",
            35.0,
            -116.0,
            Some((34.9, -116.1, 35.1, -115.9)),
            Some(json!([{"lat": 34.9, "lon": -116.1}, {"lat": 35.1, "lon": -115.9}])),
            json!({"military": "range"}),
        ));
        assert_eq!(el["type"], json!("way"));
        assert_eq!(
            el["bounds"],
            json!({"minlat": 34.9, "minlon": -116.1, "maxlat": 35.1, "maxlon": -115.9})
        );
        assert_eq!(el["geometry"].as_array().unwrap().len(), 2);
        assert_eq!(el["tags"]["military"], json!("range"));
        // way center is client-derived (bounds midpoint / center) — not a field
        assert!(el.get("lat").is_none());
        assert!(el.get("lon").is_none());
    }

    #[test]
    fn way_element_omits_absent_bounds_and_geometry() {
        let el = row_to_element(&row(
            "relation",
            789,
            None,
            "base",
            11.0,
            22.0,
            None,
            None,
            json!({"military": "base"}),
        ));
        assert_eq!(el["type"], json!("relation"));
        assert!(el.get("bounds").is_none());
        assert!(el.get("geometry").is_none());
        assert_eq!(el["tags"]["military"], json!("base"));
    }

    // ---- tags merge: original jsonb priority, mil_class/name fallback ----

    #[test]
    fn tags_keep_original_values_over_columns() {
        // name column was derived from tags originally — original wins,
        // including the name:en-only case (no synthesized `name` key).
        let r = row(
            "node",
            1,
            Some("Derived Name"),
            "airfield",
            0.0,
            0.0,
            None,
            None,
            json!({"military": "airfield", "name:en": "Original En Name"}),
        );
        let tags = merged_tags(&r);
        assert_eq!(tags["name:en"], json!("Original En Name"));
        assert!(tags.get("name").is_none());
        assert_eq!(tags["military"], json!("airfield"));
    }

    #[test]
    fn tags_synthesize_military_signal_from_mil_class() {
        // defensive path: a row whose stored tags jsonb lost the signal
        let airfield = merged_tags(&row(
            "node", 1, Some("X"), "airfield", 0.0, 0.0, None, None, json!({}),
        ));
        assert_eq!(airfield["military"], json!("airfield"));
        let land = merged_tags(&row(
            "node", 2, Some("Y"), "military_land", 0.0, 0.0, None, None, json!({}),
        ));
        assert_eq!(land["landuse"], json!("military"));
        assert!(land.get("military").is_none());
        // original signal is never overwritten
        let kept = merged_tags(&row(
            "node", 3, Some("Z"), "airfield", 0.0, 0.0, None, None,
            json!({"military": "naval_base"}),
        ));
        assert_eq!(kept["military"], json!("naval_base"));
    }

    #[test]
    fn tags_fill_name_only_when_both_name_keys_missing() {
        let r = row(
            "node", 1, Some("Column Name"), "base", 0.0, 0.0, None, None, json!({"military": "base"}),
        );
        assert_eq!(merged_tags(&r)["name"], json!("Column Name"));
        let with_name = row(
            "node", 2, Some("Column Name"), "base", 0.0, 0.0, None, None,
            json!({"military": "base", "name": "Original"}),
        );
        assert_eq!(merged_tags(&with_name)["name"], json!("Original"));
    }

    #[test]
    fn tags_tolerate_non_object_jsonb() {
        let mut r = row(
            "node", 1, Some("X"), "barracks", 0.0, 0.0, None, None, json!("garbage"),
        );
        let tags = merged_tags(&r);
        assert_eq!(tags["military"], json!("barracks"));
        assert_eq!(tags["name"], json!("X"));
        r.tags = json!(null);
        let tags = merged_tags(&r);
        assert_eq!(tags["military"], json!("barracks"));
    }

    // ---- staleness: oldest-row rule, 30h threshold ----

    #[test]
    fn stale_when_table_empty() {
        assert!(is_stale(None, Utc::now()));
    }

    #[test]
    fn stale_when_oldest_row_older_than_30h() {
        let now = Utc::now();
        assert!(is_stale(Some(now - ChronoDuration::hours(31)), now));
        assert!(is_stale(Some(now - ChronoDuration::days(3)), now));
    }

    #[test]
    fn fresh_when_oldest_row_within_30h() {
        let now = Utc::now();
        assert!(!is_stale(Some(now - ChronoDuration::hours(29)), now));
        // exactly 30h is the boundary — NOT stale (strictly greater)
        assert!(!is_stale(Some(now - ChronoDuration::hours(STALE_AFTER_HOURS)), now));
    }

    // ---- saturation / cap truncation ----

    #[test]
    fn saturated_only_when_rows_exceed_cap() {
        let (els, sat) = cap_elements(vec![json!(1), json!(2)], 2);
        assert_eq!(els.len(), 2);
        assert!(!sat); // exactly cap: not saturated
        let (els, sat) = cap_elements(vec![json!(1), json!(2), json!(3)], 2);
        assert_eq!(els.len(), 2); // truncated to cap
        assert!(sat);
        let (_, sat) = cap_elements(vec![], 2000);
        assert!(!sat);
    }

    #[test]
    fn envelope_constants_match_contract() {
        assert_eq!(ELEMENT_CAP, 2000);
        assert_eq!(STALE_AFTER_HOURS, 30);
    }
}
