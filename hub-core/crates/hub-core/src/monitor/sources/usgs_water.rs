//! USGS Water Services — real-time stream-gauge instantaneous values
//! (IV endpoint, https://waterservices.usgs.gov/docs/instantaneous-values/).
//! US public-domain data, no key required, no registration. Phase 1.2 of
//! the public-API integration roadmap (`docs/superpowers/roadmaps/
//! 2026-09-27-public-api-integration-roadmap.md`).
//!
//! Strategy: poll a curated watchlist of 25 strategic US stream gauges
//! (Mississippi / Missouri / Ohio / Tennessee / Columbia / Colorado / Rio
//! Grande / Sacramento / Susquehanna / Potomac / Delaware / Trinity / etc.),
//! parse the latest gage-height reading, compare against NWS-published
//! flood stages, and emit a single Signal per gauge per sweep (most
//! severe crossing wins — no duplicate-flood-noise on the same event).
//!
//! Single HTTP call returns all 25 gauges (`sites=01646500,07032000,...`)
//! so the per-sweep wall time is one ~500ms request regardless of
//! watchlist size. The IV response is wrapped in a SOAP-style envelope
//! (root.value.timeSeries) — see `parse_response` for the unwrap path.
//!
//! Severity ladder (matches USGS/NWS flood category conventions):
//!   level >= major_flood    → priority + extreme_type=flood_major
//!   level >= moderate_flood → priority + extreme_type=flood_moderate
//!   level >= minor_flood    → priority + extreme_type=flood_minor
//!   level >= action_stage   → routine  + extreme_type=flood_action
//!   otherwise                → no signal
//!
//! external_id = `"{site_no}:{forecast_date}:{extreme_type}"` — fixed
//! per (site, date, kind). Idempotent re-ingest across the 60-min
//! cadence: same flood event, same id, geo_events silently dedups.
//!
//! Keyless (no env var). USGS Water Services is a US federal public
//! service under the public domain — no ToS concerns. Flood stages
//! are hardcoded from NWS / AHPS gauge metadata tables; a T2 follow-up
//! can swap to a dynamically-fetched thresholds table if the table ever
//! drifts (USGS publishes these at
//! https://waterwatch.usgs.gov/ww_apps/flood/gauges.html but parsing
//! that page is HTML-scraping territory, not Phase 1.2 scope).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::collections::HashMap;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

/// USGS Instantaneous Values endpoint. One base URL; per-fetch query
/// string carries the sites CSV. `format=json` returns a SOAP-style
/// envelope (root.value.timeSeries); we unwrap it in `parse_response`.
const URL: &str = "https://waterservices.usgs.gov/nwis/iv/";

/// USGS NWIS parameter code for gage height (feet). 00060 is discharge
/// (cfs); 00065 is gage height (ft) — flood stages are expressed in
/// feet, so 00065 is the correct match.
const PARAM_GAGE_HEIGHT: &str = "00065";

/// Sentinel for "no data" readings in USGS responses. Per NWIS docs
/// (https://help.waterdata.usgs.gov/code/ndno), `noDataValue` may be
/// returned per-variable; we also hardcode -999999 as a defensive
/// fallback for older responses that omit the field.
const NO_DATA_VALUE: f64 = -999999.0;

/// USGS Water Services' standard "no data" string they put in the
/// `value` field when a sensor is offline. NWIS contract: this string
/// appears in lieu of a numeric reading (the `noDataValue` numeric
/// sentinel is sometimes also present in tandem).
const NO_DATA_STR: &str = "-999999";

/// Strategic US stream-gauge watchlist. Each entry pairs a USGS site_no
/// with NWS/AHPS-published flood stages (action / minor / moderate /
/// major, all in feet). Stages are sourced from the AHPS gauge-info
/// pages for each site (https://water.weather.gov/ahps/) and rounded
/// to whole feet where AHPS publishes fractions.
///
/// Consequence: a flood event at a gauge where we've drifted from AHPS
/// by ≥0.5 ft will be misclassified (e.g., a 27.4 ft reading on a 28 ft
/// action stage would NOT emit, even though AHPS considers it action).
/// The thresholds are deliberately conservative — Phase 1.2 ships
/// minimal and a T2 follow-up can re-pull AHPS dynamically.
///
/// Schema: (site_no, name, lat, lon, action_ft, minor_ft, moderate_ft, major_ft).
const GAUGES: &[(&str, &str, f64, f64, f64, f64, f64, f64)] = &[
    // ── Mississippi River basin ──
    ("07032000", "Mississippi River at Memphis, TN",
     35.1494, -90.0583, 28.0, 34.0, 40.0, 46.0),
    ("05420500", "Mississippi River at Clinton, IA",
     41.7811, -90.2500, 13.0, 17.0, 20.0, 23.0),
    // ── Missouri River basin ──
    ("06610000", "Missouri River at Omaha, NE",
     41.2906, -95.9247, 25.0, 27.0, 30.0, 32.0),
    ("06935965", "Missouri River at St. Charles, MO",
     38.7878, -90.4792, 25.0, 30.0, 35.0, 37.0),
    // ── Ohio River basin ──
    ("03254540", "Ohio River at Cincinnati, OH",
     39.0972, -84.5103, 40.0, 52.0, 56.0, 65.0),
    ("03294500", "Ohio River at Louisville, KY",
     38.2756, -85.7933, 23.0, 38.0, 49.0, 55.0),
    // ── Tennessee/Cumberland (TVA — nuclear fleet watch) ──
    ("03572000", "Tennessee River at Chattanooga, TN",
     35.0833, -85.2789, 16.0, 30.0, 36.0, 45.0),
    ("03431500", "Cumberland River at Nashville, TN",
     36.1692, -86.7753, 30.0, 35.0, 40.0, 45.0),
    // ── Arkansas / Red River ──
    ("07109500", "Arkansas River at Pueblo, CO",
     38.2553, -104.6169, 6.0, 8.0, 10.0, 14.0),
    ("07024000", "Arkansas River at Tulsa, OK",
     36.1461, -95.9775, 11.0, 18.0, 22.0, 26.0),
    ("07374000", "Red River at Shreveport, LA",
     32.5181, -93.7408, 25.5, 30.0, 32.0, 34.0),
    // ── Mid-Atlantic / DC corridor ──
    ("01646500", "Potomac River near Wash, DC Little Falls",
     38.9498, -77.1276, 5.0, 7.0, 10.0, 12.5),
    ("01463500", "Delaware River at Trenton, NJ",
     40.2211, -74.7778, 8.0, 9.0, 10.5, 12.0),
    ("01570500", "Susquehanna River at Harrisburg, PA",
     40.2511, -76.8861, 11.0, 17.0, 20.0, 24.0),
    // ── West Coast ──
    ("11425500", "Sacramento River at Verona, CA",
     38.7761, -121.5931, 26.5, 30.0, 38.0, 45.7),
    ("14105700", "Columbia River at The Dalles, OR",
     45.6050, -121.1800, 16.0, 18.0, 22.0, 27.0),
    ("11467000", "Russian River near Guerneville, CA",
     38.4981, -122.9872, 21.0, 32.0, 35.0, 41.0),
    ("11510700", "Klamath River near Klamath, CA",
     41.5272, -123.9869, 8.0, 12.0, 16.0, 20.0),
    ("11274500", "San Joaquin River near Vernalis, CA",
     37.6772, -121.2669, 16.5, 19.0, 24.0, 30.0),
    // ── Texas / Gulf ──
    ("08057000", "Trinity River at Dallas, TX",
     32.7764, -96.8231, 25.0, 30.0, 33.0, 38.0),
    ("08466310", "Rio Grande at Laredo, TX",
     27.4928, -99.4850, 7.0, 12.0, 18.0, 25.0),
    // ── Plains / Mountain ──
    ("06770500", "Platte River at Grand Island, NE",
     40.8858, -98.3514, 6.0, 7.0, 8.5, 10.0),
    ("06214500", "Yellowstone River at Billings, MT",
     45.7950, -108.4656, 8.0, 10.0, 12.0, 14.0),
    ("05568500", "Illinois River at Peoria, IL",
     40.6936, -89.5864, 18.0, 23.0, 27.0, 32.0),
    ("10343500", "Humboldt River near Comus, NV",
     40.0117, -118.3853, 7.0, 8.5, 9.5, 10.5),
];

pub struct UsgsWater;

impl Source for UsgsWater {
    fn name(&self) -> &'static str {
        "usgs_water"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(3600) // 60 min — roadmap §2.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let sites: Vec<&str> = GAUGES.iter().map(|g| g.0).collect();
            let url = format!(
                "{URL}?format=json&sites={}&parameterCd={PARAM_GAGE_HEIGHT}&period=PT1H",
                sites.join(",")
            );
            let resp = match ctx.http.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "usgs_water fetch failed");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "usgs_water non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "usgs_water parse failed");
                    return Ok(Vec::new());
                }
            };
            let readings = parse_response(&body);
            Ok(evaluate_thresholds(&readings))
        }
        .boxed()
    }
}

/// Latest reading for one gauge, post-parse. Empty string in `date_time`
/// means the upstream omitted the field; downstream consumers can still
/// use the value, the date is just missing.
struct Reading {
    site_no: String,
    gage_height_ft: f64,
    lat: f64,
    lon: f64,
    date_time: String,
}

/// Pure parser: unwrap the SOAP envelope and extract the latest reading
/// per site. Defensive against missing fields; returns whatever subset
/// of sites the upstream actually delivered (a partial response is
/// still valuable).
fn parse_response(j: &serde_json::Value) -> HashMap<String, Reading> {
    let mut out = HashMap::new();
    let Some(time_series) = j.pointer("/value/timeSeries").and_then(|v| v.as_array()) else {
        return out;
    };
    for ts in time_series {
        let Some(site_no) = ts
            .pointer("/sourceInfo/siteCode/0/value")
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let Some(lat) = ts
            .pointer("/sourceInfo/geoLocation/geogLocation/latitude")
            .and_then(|v| v.as_f64())
        else {
            continue;
        };
        let Some(lon) = ts
            .pointer("/sourceInfo/geoLocation/geogLocation/longitude")
            .and_then(|v| v.as_f64())
        else {
            continue;
        };
        let Some(values) = ts.pointer("/values/0/value").and_then(|v| v.as_array()) else {
            continue;
        };
        let Some(latest) = values.last() else {
            continue;
        };
        let Some(value_str) = latest.get("value").and_then(|v| v.as_str()) else {
            continue;
        };
        if value_str == NO_DATA_STR {
            continue;
        }
        let Ok(height) = value_str.parse::<f64>() else {
            continue;
        };
        if height == NO_DATA_VALUE {
            continue;
        }
        let date_time = latest
            .get("dateTime")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        out.insert(
            site_no.to_string(),
            Reading {
                site_no: site_no.to_string(),
                gage_height_ft: height,
                lat,
                lon,
                date_time,
            },
        );
    }
    out
}

/// Compare each reading against the watchlist's flood thresholds and
/// emit at most one Signal per gauge (most-severe crossing wins).
fn evaluate_thresholds(readings: &HashMap<String, Reading>) -> Vec<Signal> {
    let mut out = Vec::new();
    for &(site_no, name, lat, lon, action, minor, moderate, major) in GAUGES {
        let Some(r) = readings.get(site_no) else {
            continue; // upstream didn't return this site (sensor offline, etc.)
        };
        let level = r.gage_height_ft;
        let (severity, kind, threshold) = if level >= major {
            ("priority", "flood_major", major)
        } else if level >= moderate {
            ("priority", "flood_moderate", moderate)
        } else if level >= minor {
            ("priority", "flood_minor", minor)
        } else if level >= action {
            ("routine", "flood_action", action)
        } else {
            continue;
        };
        // Forecast date for external_id stability: USGS doesn't expose a
        // "flood event start" date, so we use the reading's date — same
        // reading re-poll = same external_id = geo_events dedups.
        let date = r.date_time.get(..10).unwrap_or("0000-00-00");
        out.push(
            Signal::new(
                "flood",
                format!("{name} — {kind} ({level:.1} ft ≥ {threshold:.1} ft)"),
                r.lat,
                r.lon,
                format!("{site_no}:{date}:{kind}"),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "site_no": site_no,
                "site_name": name,
                "gage_height_ft": level,
                "threshold_ft": threshold,
                "extreme_type": kind,
                "reading_date": date,
                "reading_datetime": r.date_time,
                "stage_action_ft": action,
                "stage_minor_ft": minor,
                "stage_moderate_ft": moderate,
                "stage_major_ft": major,
            })),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "value": {
                "timeSeries": [
                    // Above action only — Mississippi at Memphis, 30 ft
                    {
                        "sourceInfo": {
                            "siteName": "MISSISSIPPI RIVER AT MEMPHIS, TN",
                            "siteCode": [{"value": "07032000"}],
                            "geoLocation": {
                                "geogLocation": {
                                    "latitude": 35.1494,
                                    "longitude": -90.0583
                                }
                            }
                        },
                        "variable": {"variableName": "Gage height, ft", "noDataValue": -999999},
                        "values": [{
                            "value": [
                                {"value": "30.5", "dateTime": "2026-09-27T10:00:00.000-05:00"}
                            ]
                        }]
                    },
                    // Above minor — Potomac DC, 8.0 ft
                    {
                        "sourceInfo": {
                            "siteName": "POTOMAC RIVER NEAR WASH, DC LITTLE FALLS PUMP STA",
                            "siteCode": [{"value": "01646500"}],
                            "geoLocation": {
                                "geogLocation": {
                                    "latitude": 38.9498,
                                    "longitude": -77.1276
                                }
                            }
                        },
                        "variable": {"variableName": "Gage height, ft", "noDataValue": -999999},
                        "values": [{
                            "value": [
                                {"value": "8.0", "dateTime": "2026-09-27T10:00:00.000-04:00"}
                            ]
                        }]
                    },
                    // Above major — hypothetical Sacramento, 50 ft
                    {
                        "sourceInfo": {
                            "siteName": "SACRAMENTO RIVER AT VERONA, CA",
                            "siteCode": [{"value": "11425500"}],
                            "geoLocation": {
                                "geogLocation": {"latitude": 38.7761, "longitude": -121.5931}
                            }
                        },
                        "variable": {"variableName": "Gage height, ft", "noDataValue": -999999},
                        "values": [{
                            "value": [
                                {"value": "50.0", "dateTime": "2026-09-27T10:00:00.000-07:00"}
                            ]
                        }]
                    },
                    // Below action — quiet gauge, 4.0 ft (Missouri Omaha action=25)
                    {
                        "sourceInfo": {
                            "siteName": "MISSOURI RIVER AT OMAHA, NE",
                            "siteCode": [{"value": "06610000"}],
                            "geoLocation": {
                                "geogLocation": {"latitude": 41.2906, "longitude": -95.9247}
                            }
                        },
                        "variable": {"variableName": "Gage height, ft", "noDataValue": -999999},
                        "values": [{
                            "value": [
                                {"value": "4.0", "dateTime": "2026-09-27T10:00:00.000-05:00"}
                            ]
                        }]
                    },
                    // No-data gauge — should be skipped
                    {
                        "sourceInfo": {
                            "siteName": "DOWN GAUGE",
                            "siteCode": [{"value": "99999999"}],
                            "geoLocation": {
                                "geogLocation": {"latitude": 0.0, "longitude": 0.0}
                            }
                        },
                        "variable": {"variableName": "Gage height, ft", "noDataValue": -999999},
                        "values": [{
                            "value": [
                                {"value": "-999999", "dateTime": "2026-09-27T10:00:00.000-05:00"}
                            ]
                        }]
                    }
                ]
            }
        })
    }

    /// Happy path: 4 valid gauges produce 3 signals (one is below
    /// action), and the no-data gauge is silently dropped.
    #[test]
    fn detects_flood_stages_correctly() {
        let readings = parse_response(&sample_response());
        assert_eq!(readings.len(), 4, "no-data gauge should be skipped");
        let sigs = evaluate_thresholds(&readings);
        // 07032000 (30.5 ft ≥ action 28) → routine, flood_action
        // 01646500 (8.0 ft ≥ minor 7)    → priority, flood_minor
        // 11425500 (50.0 ft ≥ major 45.7) → priority, flood_major
        // 06610000 (4.0 ft < action 25)   → no signal
        assert_eq!(sigs.len(), 3, "expected 3 signals, got len={}", sigs.len());
        // severity distribution
        let routine = sigs.iter().filter(|s| s.severity == "routine").count();
        let priority = sigs.iter().filter(|s| s.severity == "priority").count();
        assert_eq!(routine, 1);
        assert_eq!(priority, 2);
    }

    /// external_id format = `{site_no}:{date}:{extreme_type}`.
    #[test]
    fn external_id_shape() {
        let readings = parse_response(&sample_response());
        let sigs = evaluate_thresholds(&readings);
        let by_site = |s: &str| sigs.iter().find(|x| x.external_id.starts_with(s)).unwrap();
        let memphis = by_site("07032000");
        assert!(memphis.external_id.ends_with(":flood_action"));
        let potomac = by_site("01646500");
        assert!(potomac.external_id.ends_with(":flood_minor"));
        let sacramento = by_site("11425500");
        assert!(sacramento.external_id.ends_with(":flood_major"));
    }

    /// Below action → no signal at all (the "calm weather" case).
    #[test]
    fn below_action_emits_nothing() {
        let j = serde_json::json!({
            "value": {"timeSeries": [{
                "sourceInfo": {
                    "siteName": "MISSOURI RIVER AT OMAHA, NE",
                    "siteCode": [{"value": "06610000"}],
                    "geoLocation": {"geogLocation": {"latitude": 41.29, "longitude": -95.92}}
                },
                "variable": {"noDataValue": -999999},
                "values": [{"value": [{"value": "4.0", "dateTime": "2026-09-27T10:00:00.000Z"}]}]
            }]}
        });
        let readings = parse_response(&j);
        assert!(evaluate_thresholds(&readings).is_empty());
    }

    /// Threshold ladder precision: a reading exactly AT a threshold fires
    /// (inclusive), like Open-Meteo precedent.
    #[test]
    fn threshold_inclusive() {
        // Potomac action=5.0, exact 5.0 → flood_action (routine)
        let j = serde_json::json!({
            "value": {"timeSeries": [{
                "sourceInfo": {
                    "siteName": "POTOMAC RIVER NEAR WASH, DC",
                    "siteCode": [{"value": "01646500"}],
                    "geoLocation": {"geogLocation": {"latitude": 38.95, "longitude": -77.13}}
                },
                "variable": {"noDataValue": -999999},
                "values": [{"value": [{"value": "5.0", "dateTime": "2026-09-27T10:00:00.000Z"}]}]
            }]}
        });
        let readings = parse_response(&j);
        let sigs = evaluate_thresholds(&readings);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "routine");
    }

    /// Threshold exclusivity: 0.01 ft below action → no signal.
    #[test]
    fn threshold_exclusive_below() {
        let j = serde_json::json!({
            "value": {"timeSeries": [{
                "sourceInfo": {
                    "siteName": "POTOMAC RIVER NEAR WASH, DC",
                    "siteCode": [{"value": "01646500"}],
                    "geoLocation": {"geogLocation": {"latitude": 38.95, "longitude": -77.13}}
                },
                "variable": {"noDataValue": -999999},
                "values": [{"value": [{"value": "4.99", "dateTime": "2026-09-27T10:00:00.000Z"}]}]
            }]}
        });
        let readings = parse_response(&j);
        assert!(evaluate_thresholds(&readings).is_empty());
    }

    /// Defensive: missing SOAP envelope → empty HashMap.
    #[test]
    fn empty_envelope_returns_empty_map() {
        assert!(parse_response(&serde_json::json!({})).is_empty());
        assert!(parse_response(&serde_json::json!({"value": {}})).is_empty());
        assert!(parse_response(&serde_json::json!({"value": {"timeSeries": []}})).is_empty());
    }

    /// Defensive: timeSeries item missing required fields → silently skipped.
    #[test]
    fn malformed_time_series_item_skipped() {
        let j = serde_json::json!({
            "value": {"timeSeries": [
                {"sourceInfo": null}, // null sourceInfo
                {"sourceInfo": {"siteCode": []}}, // missing geoLocation
                {"sourceInfo": {"siteCode": [{"value": "01646500"}], "geoLocation": {"geogLocation": {"latitude": 38.95, "longitude": -77.13}}},
                 "variable": {"noDataValue": -999999},
                 "values": [{"value": []}]}, // empty values array
                {"sourceInfo": {"siteCode": [{"value": "01646500"}], "geoLocation": {"geogLocation": {"latitude": 38.95, "longitude": -77.13}}},
                 "variable": {"noDataValue": -999999},
                 "values": [{"value": [{"value": "not-a-number", "dateTime": "2026-09-27T10:00:00.000Z"}]}]}, // unparseable
                {"sourceInfo": {"siteCode": [{"value": "01646500"}], "geoLocation": {"geogLocation": {"latitude": 38.95, "longitude": -77.13}}},
                 "variable": {"noDataValue": -999999},
                 "values": [{"value": [{"value": "-999999", "dateTime": "2026-09-27T10:00:00.000Z"}]}]} // no-data string
            ]}
        });
        let readings = parse_response(&j);
        assert!(readings.is_empty(), "all items should be silently skipped");
    }

    /// "Most severe wins" — a reading above major emits ONLY flood_major,
    /// not a stack of flood_action + flood_minor + flood_moderate +
    /// flood_major for the same gauge.
    #[test]
    fn only_most_severe_crossing_fires() {
        let j = serde_json::json!({
            "value": {"timeSeries": [{
                "sourceInfo": {
                    "siteName": "SACRAMENTO RIVER AT VERONA, CA",
                    "siteCode": [{"value": "11425500"}],
                    "geoLocation": {"geogLocation": {"latitude": 38.78, "longitude": -121.59}}
                },
                "variable": {"noDataValue": -999999},
                "values": [{"value": [{"value": "60.0", "dateTime": "2026-09-27T10:00:00.000Z"}]}]
            }]}
        });
        let readings = parse_response(&j);
        let sigs = evaluate_thresholds(&readings);
        assert_eq!(sigs.len(), 1, "expected 1 signal, got len={}", sigs.len());
        assert!(sigs[0].external_id.ends_with(":flood_major"));
        assert_eq!(sigs[0].severity, "priority");
    }

    /// external_id stability across re-poll (idempotent ingest).
    #[test]
    fn external_id_stable_across_calls() {
        let readings = parse_response(&sample_response());
        let a = evaluate_thresholds(&readings);
        let b = evaluate_thresholds(&readings);
        let ids_a: Vec<&str> = a.iter().map(|s| s.external_id.as_str()).collect();
        let ids_b: Vec<&str> = b.iter().map(|s| s.external_id.as_str()).collect();
        assert_eq!(ids_a, ids_b);
    }

    /// Watchlist size guard: drift below 20 or above 40 is a regression.
    #[test]
    fn gauge_watchlist_size() {
        assert!(GAUGES.len() >= 20, "watchlist too small: {}", GAUGES.len());
        assert!(GAUGES.len() <= 40, "watchlist too large: {}", GAUGES.len());
    }
}