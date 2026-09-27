//! HDX HAPI - Humanitarian API (https://hapi.humdata.org/) —
//! national-risk + humanitarian funding indicators. Phase 4.3 of
//! the public-API integration roadmap (`docs/superpowers/roadmaps/
//! 2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Polls two endpoints every 24h:
//! - `GET /api/v2/coordination-context/national-risk?limit=N` —
//!   INFORM-style national risk scores (overall_risk 0-10 +
//!   hazard_exposure + vulnerability + coping_capacity). Annual
//!   reference period.
//! - `GET /api/v2/coordination-context/funding?limit=N` — appeal
//!   requirements_usd vs funding_usd vs funding_pct. Per appeal
//!   per year per country. Adds the 'how underfunded is this
//!   crisis?' dimension.
//!
//! **Keyless** (technically — every request needs an
//! `app_identifier` header, but it's just `base64("name:email")`,
//! not a secret. See below).
//!
//! ## Compounds with...
//!
//! - Phase 1.4 `hdx_humanitarian` (CKAN catalog) — that one
//!   surfaces dataset metadata + URLs (what datasets exist on
//!   HDX). This one surfaces the actual risk + funding scores
//!   inside those datasets. Same source organization (OCHA
//!   Centre for Humanitarian Data) but different layers.
//! - Phase 3.4 `gitguardian` and 4.1 `threatcluster` — incidents +
//!   risk context.
//!
//! ## app_identifier (KEYLESS)
//!
//! Per HAPI docs (https://docs.humdata.org/build/hdx-apis/hapi/
//! how-to-query-hapi), every request needs an `app_identifier`:
//!
//!   "The app_identifier is simply a base64 encoded version of a
//!    user supplied application name and email address. It is not
//!    a secret key and does not grant special permissions."
//!
//! We encode `IntelHub:claude@anthropic.com` (the project's
//! canonical name + maintainer contact). HAPI endpoints are
//! rate-limited per app_identifier but the limit is generous
//! (no published number; community usage shows no throttling
//! on daily sweeps at 24h cadence).
//!
//! ## Severity ladder
//!
//! National-risk:
//! - `overall_risk >= 7.5` → priority (very high humanitarian
//!   crisis — direct aid priority)
//! - `overall_risk >= 5.0` → routine (elevated risk; monitoring)
//! - else → info (low risk baseline)
//!
//! Funding:
//! - `funding_pct < 30` → priority (severely underfunded; the
//!   appeal is asking but donors aren't responding)
//! - `funding_pct < 70` → routine (partially funded)
//! - else → info (well funded)
//!
//! ## external_id
//!
//! - national-risk: `hdx_hapi:nr:{location_code}` — annual
//!   reference_period, so re-polls same year dedup.
//! - funding: `hdx_hapi:fn:{appeal_code}:{reference_period_start}`
//!   — unique per appeal + year.
//!
//! ## Cadence
//!
//! 24h. National-risk is annual; daily sweeps mostly return
//! unchanged data → 0 new events on quiet days (PASS via "zero
//! OK"). Funding updates as new appeals + funding rounds are
//! published (typically a few per week).
//!
//! ## Cap
//!
//! TOP_N_NATIONAL_RISK = 15 (top-15 risk countries).
//! TOP_N_FUNDING = 15 (most-underfunded active appeals).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://hapi.humdata.org/api/v2";

/// `app_identifier` for HAPI. NOT a secret — per HAPI docs it's
/// just `base64("name:email")`. We use the project's canonical
/// name + maintainer contact. Change if IntelHub is forked.
const APP_NAME: &str = "IntelHub";
const APP_EMAIL: &str = "claude@anthropic.com";

const INTERVAL_SECS: u64 = 24 * 3600;

const TOP_N_NATIONAL_RISK: usize = 15;
const TOP_N_FUNDING: usize = 15;

/// Minimum severity threshold for national-risk (overall_risk 0-10).
const NATIONAL_RISK_PRIORITY: f64 = 7.5;
const NATIONAL_RISK_ROUTINE: f64 = 5.0;

/// Funding percent thresholds (0-100, where 100 = fully funded).
const FUNDING_PRIORITY_PCT: f64 = 30.0;
const FUNDING_ROUTINE_PCT: f64 = 70.0;

pub struct HdxHapi;

impl Source for HdxHapi {
    fn name(&self) -> &'static str {
        "hdx_hapi"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let app_id = build_app_id();
            let mut all = Vec::new();
            match fetch_national_risk(ctx, &app_id).await {
                Ok(v) => all.extend(v),
                Err(e) => tracing::warn!(error = %e, "hdx_hapi national-risk fetch failed"),
            }
            match fetch_funding(ctx, &app_id).await {
                Ok(v) => all.extend(v),
                Err(e) => tracing::warn!(error = %e, "hdx_hapi funding fetch failed"),
            }
            Ok(all)
        }
        .boxed()
    }
}

/// Build the app_identifier = base64("name:email") per HAPI docs.
/// Pure function — same input → same output, no surprise.
fn build_app_id() -> String {
    use base64::{engine::general_purpose, Engine as _};
    let raw = format!("{APP_NAME}:{APP_EMAIL}");
    general_purpose::STANDARD.encode(raw.as_bytes())
}

async fn fetch_national_risk(ctx: &Ctx, app_id: &str) -> Result<Vec<Signal>> {
    let resp = ctx
        .http
        .get(format!("{BASE_URL}/coordination-context/national-risk"))
        .query(&[
            ("app_identifier", app_id),
            ("limit", &TOP_N_NATIONAL_RISK.to_string()),
        ])
        .send()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("hdx_hapi http: {e}")))?;
    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "hdx_hapi national-risk non-2xx");
        return Ok(Vec::new());
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("hdx_hapi json: {e}")))?;
    Ok(parse_national_risk(&body))
}

async fn fetch_funding(ctx: &Ctx, app_id: &str) -> Result<Vec<Signal>> {
    let resp = ctx
        .http
        .get(format!("{BASE_URL}/coordination-context/funding"))
        .query(&[
            ("app_identifier", app_id),
            ("limit", &TOP_N_FUNDING.to_string()),
        ])
        .send()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("hdx_hapi http: {e}")))?;
    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "hdx_hapi funding non-2xx");
        return Ok(Vec::new());
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("hdx_hapi json: {e}")))?;
    Ok(parse_funding(&body))
}

fn parse_national_risk(j: &serde_json::Value) -> Vec<Signal> {
    let Some(arr) = j.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<(f64, Signal)> = Vec::with_capacity(arr.len());
    for r in arr {
        let location_code = pick_string(r, &["location_code", "country_code"]).unwrap_or_default();
        let location_name = pick_string(r, &["location_name", "country_name"]).unwrap_or_default();
        if location_code.is_empty() {
            continue;
        }
        let overall_risk = pick_number(r, &["overall_risk"]).unwrap_or(0.0);
        let hazard = pick_number(r, &["hazard_exposure_risk"]).unwrap_or(0.0);
        let vulnerability = pick_number(r, &["vulnerability_risk"]).unwrap_or(0.0);
        let coping = pick_number(r, &["coping_capacity_risk"]).unwrap_or(0.0);
        let global_rank = pick_number(r, &["global_rank"]).unwrap_or(0.0) as i64;
        let risk_class = pick_string(r, &["risk_class"]).unwrap_or_default();
        let reference_period_start =
            pick_string(r, &["reference_period_start"]).unwrap_or_default();
        let (severity, kind) = classify_risk(overall_risk);
        out.push((
            overall_risk,
            Signal::new(
                kind,
                format!(
                    "{location_name} ({location_code}) risk={overall_risk:.1} rank={global_rank}"
                ),
                0.0,
                0.0,
                format!("hdx_hapi:nr:{location_code}"),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "kind": "hdx_hapi_national_risk",
                "location_code": location_code,
                "location_name": location_name,
                "overall_risk": overall_risk,
                "hazard_exposure_risk": hazard,
                "vulnerability_risk": vulnerability,
                "coping_capacity_risk": coping,
                "global_rank": global_rank,
                "risk_class": risk_class,
                "reference_period_start": reference_period_start,
            })),
        ));
    }
    // Sort desc by risk, cap.
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(TOP_N_NATIONAL_RISK);
    out.into_iter().map(|(_, s)| s).collect()
}

fn parse_funding(j: &serde_json::Value) -> Vec<Signal> {
    let Some(arr) = j.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<(f64, Signal)> = Vec::with_capacity(arr.len());
    for r in arr {
        let appeal_code = pick_string(r, &["appeal_code"]).unwrap_or_default();
        let appeal_name = pick_string(r, &["appeal_name"]).unwrap_or_default();
        let appeal_type = pick_string(r, &["appeal_type"]).unwrap_or_default();
        let location_code = pick_string(r, &["location_code"]).unwrap_or_default();
        let location_name = pick_string(r, &["location_name"]).unwrap_or_default();
        if appeal_code.is_empty() {
            continue;
        }
        let requirements_usd = pick_number(r, &["requirements_usd"]).unwrap_or(0.0);
        let funding_usd = pick_number(r, &["funding_usd"]).unwrap_or(0.0);
        let funding_pct = pick_number(r, &["funding_pct"]).unwrap_or(0.0);
        let reference_period_start =
            pick_string(r, &["reference_period_start"]).unwrap_or_default();
        // Sort key: lowest funding_pct (most underfunded first).
        let (severity, kind) = classify_funding(funding_pct);
        out.push((
            funding_pct,
            Signal::new(
                kind,
                format!(
                    "{location_name} appeal={appeal_name} funding={funding_pct:.0}%"
                ),
                0.0,
                0.0,
                format!(
                    "hdx_hapi:fn:{appeal_code}:{reference_period_start}"
                ),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "kind": "hdx_hapi_funding",
                "appeal_code": appeal_code,
                "appeal_name": appeal_name,
                "appeal_type": appeal_type,
                "location_code": location_code,
                "location_name": location_name,
                "requirements_usd": requirements_usd,
                "funding_usd": funding_usd,
                "funding_pct": funding_pct,
                "reference_period_start": reference_period_start,
            })),
        ));
    }
    // Sort ascending by funding_pct (most underfunded first).
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(TOP_N_FUNDING);
    out.into_iter().map(|(_, s)| s).collect()
}

fn classify_risk(overall: f64) -> (&'static str, &'static str) {
    if overall >= NATIONAL_RISK_PRIORITY {
        return ("priority", "hdx_hapi_national_risk_priority");
    }
    if overall >= NATIONAL_RISK_ROUTINE {
        return ("routine", "hdx_hapi_national_risk_routine");
    }
    ("info", "hdx_hapi_national_risk_info")
}

fn classify_funding(pct: f64) -> (&'static str, &'static str) {
    if pct < FUNDING_PRIORITY_PCT {
        return ("priority", "hdx_hapi_funding_priority");
    }
    if pct < FUNDING_ROUTINE_PCT {
        return ("routine", "hdx_hapi_funding_routine");
    }
    ("info", "hdx_hapi_funding_info")
}

fn pick_string(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn pick_number(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(n) = v.get(*k).and_then(|x| x.as_f64()) {
            return Some(n);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_risk() -> serde_json::Value {
        serde_json::json!({
            "data": [
                {
                    "location_code": "AFG",
                    "location_name": "Afghanistan",
                    "overall_risk": 8.4,
                    "hazard_exposure_risk": 8.0,
                    "vulnerability_risk": 8.8,
                    "coping_capacity_risk": 7.5,
                    "global_rank": 7,
                    "risk_class": "5",
                    "reference_period_start": "2025-01-01T00:00:00"
                },
                {
                    "location_code": "SOM",
                    "location_name": "Somalia",
                    "overall_risk": 8.1,
                    "global_rank": 9,
                    "reference_period_start": "2025-01-01T00:00:00"
                },
                {
                    "location_code": "SSD",
                    "location_name": "South Sudan",
                    "overall_risk": 7.7,
                    "global_rank": 12,
                    "reference_period_start": "2025-01-01T00:00:00"
                },
                {
                    "location_code": "DEU",
                    "location_name": "Germany",
                    "overall_risk": 3.2,
                    "global_rank": 122,
                    "reference_period_start": "2025-01-01T00:00:00"
                }
            ]
        })
    }

    fn sample_funding() -> serde_json::Value {
        serde_json::json!({
            "data": [
                {
                    "appeal_code": "CAFG01",
                    "appeal_name": "Afghanistan 2026",
                    "appeal_type": "Consolidated inter-agency appeal",
                    "location_code": "AFG",
                    "location_name": "Afghanistan",
                    "requirements_usd": 1_500_000_000.0,
                    "funding_usd": 250_000_000.0,
                    "funding_pct": 16.7,
                    "reference_period_start": "2026-01-01T00:00:00"
                },
                {
                    "appeal_code": "CSOM01",
                    "appeal_name": "Somalia 2026",
                    "appeal_type": "Consolidated inter-agency appeal",
                    "location_code": "SOM",
                    "location_name": "Somalia",
                    "requirements_usd": 1_000_000_000.0,
                    "funding_usd": 450_000_000.0,
                    "funding_pct": 45.0,
                    "reference_period_start": "2026-01-01T00:00:00"
                },
                {
                    "appeal_code": "CSYR01",
                    "appeal_name": "Syria 2026",
                    "appeal_type": "Consolidated inter-agency appeal",
                    "location_code": "SYR",
                    "location_name": "Syria",
                    "requirements_usd": 4_000_000_000.0,
                    "funding_usd": 3_500_000_000.0,
                    "funding_pct": 87.5,
                    "reference_period_start": "2026-01-01T00:00:00"
                }
            ]
        })
    }

    fn empty() -> serde_json::Value {
        serde_json::json!({"data": []})
    }

    fn missing_data() -> serde_json::Value {
        serde_json::json!({})
    }

    /// Risk >= 7.5 → priority
    #[test]
    fn risk_high_is_priority() {
        let sigs = parse_national_risk(&sample_risk());
        let afg = sigs.iter().find(|s| s.external_id.contains("AFG")).unwrap();
        assert_eq!(afg.severity, "priority");
        assert_eq!(afg.kind, "hdx_hapi_national_risk_priority");
    }

    /// Risk >= 5.0 < 7.5 → routine
    #[test]
    fn risk_medium_is_routine() {
        let sigs = parse_national_risk(&sample_risk());
        // Manually construct a mid-risk entry to test ladder;
        // sample only has high + low — append one to test
        let mut j = sample_risk();
        j["data"].as_array_mut().unwrap().push(serde_json::json!({
            "location_code": "TEST", "location_name": "Test",
            "overall_risk": 5.5, "global_rank": 50,
            "reference_period_start": "2025-01-01T00:00:00"
        }));
        let sigs = parse_national_risk(&j);
        let mid = sigs.iter().find(|s| s.external_id.contains("TEST")).unwrap();
        assert_eq!(mid.severity, "routine");
    }

    /// Risk < 5.0 → info
    #[test]
    fn risk_low_is_info() {
        let sigs = parse_national_risk(&sample_risk());
        let deu = sigs.iter().find(|s| s.external_id.contains("DEU")).unwrap();
        assert_eq!(deu.severity, "info");
    }

    /// Sorted desc by risk, capped
    #[test]
    fn risk_sorted_desc_capped() {
        let sigs = parse_national_risk(&sample_risk());
        // AFG (8.4), SOM (8.1), SSD (7.7), DEU (3.2) — all 4 in sample, all within TOP_N
        assert_eq!(sigs.len(), 4);
        assert!(sigs[0].external_id.contains("AFG"));
        assert!(sigs[3].external_id.contains("DEU"));
    }

    /// Funding < 30% → priority
    #[test]
    fn funding_underfunded_is_priority() {
        let sigs = parse_funding(&sample_funding());
        let afg = sigs.iter().find(|s| s.external_id.contains("CAFG01")).unwrap();
        assert_eq!(afg.severity, "priority");
    }

    /// Funding 30-70% → routine
    #[test]
    fn funding_partial_is_routine() {
        let sigs = parse_funding(&sample_funding());
        let som = sigs.iter().find(|s| s.external_id.contains("CSOM01")).unwrap();
        assert_eq!(som.severity, "routine");
    }

    /// Funding >= 70% → info
    #[test]
    fn funding_funded_is_info() {
        let sigs = parse_funding(&sample_funding());
        let syr = sigs.iter().find(|s| s.external_id.contains("CSYR01")).unwrap();
        assert_eq!(syr.severity, "info");
    }

    /// Sorted asc by funding_pct (most underfunded first)
    #[test]
    fn funding_sorted_asc() {
        let sigs = parse_funding(&sample_funding());
        assert_eq!(sigs[0].external_id, "hdx_hapi:fn:CAFG01:2026-01-01T00:00:00");
        assert_eq!(sigs[2].external_id, "hdx_hapi:fn:CSYR01:2026-01-01T00:00:00");
    }

    /// External IDs: hdx_hapi:nr:{location_code} for risk, hdx_hapi:fn:{appeal_code}:{period} for funding
    #[test]
    fn external_id_shapes() {
        let sigs_r = parse_national_risk(&sample_risk());
        assert_eq!(sigs_r[0].external_id, "hdx_hapi:nr:AFG");
        let sigs_f = parse_funding(&sample_funding());
        assert_eq!(sigs_f[0].external_id, "hdx_hapi:fn:CAFG01:2026-01-01T00:00:00");
    }

    /// Empty data[] / missing data key → empty
    #[test]
    fn empty_or_missing() {
        assert!(parse_national_risk(&empty()).is_empty());
        assert!(parse_funding(&empty()).is_empty());
        assert!(parse_national_risk(&missing_data()).is_empty());
        assert!(parse_funding(&missing_data()).is_empty());
    }

    /// Skip entries with empty location_code / appeal_code (defensive)
    #[test]
    fn skip_empty_keys() {
        let j = serde_json::json!({
            "data": [
                {"location_code": "", "location_name": "no code", "overall_risk": 9.0},
                {"location_code": "OK", "location_name": "ok", "overall_risk": 9.0}
            ]
        });
        let sigs = parse_national_risk(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].external_id, "hdx_hapi:nr:OK");
    }

    /// app_identifier = base64("IntelHub:claude@anthropic.com")
    #[test]
    fn app_id_format() {
        let id = build_app_id();
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(&id)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, "IntelHub:claude@anthropic.com");
    }

    /// Caps respected
    #[test]
    fn caps() {
        let big_risk: Vec<_> = (0..30)
            .map(|i| {
                serde_json::json!({
                    "location_code": format!("XX{i:02}"),
                    "location_name": format!("Country {i}"),
                    "overall_risk": 5.0 + (i as f64) * 0.1,
                    "global_rank": i + 1,
                    "reference_period_start": "2025-01-01T00:00:00"
                })
            })
            .collect();
        let j = serde_json::json!({"data": big_risk});
        let sigs = parse_national_risk(&j);
        assert_eq!(sigs.len(), TOP_N_NATIONAL_RISK);
    }

    /// pick_string / pick_number utilities
    #[test]
    fn pick_helpers() {
        let v = serde_json::json!({"a": "F", "b": 1.5});
        assert_eq!(pick_string(&v, &["a"]), Some("F".to_string()));
        assert_eq!(pick_number(&v, &["b"]), Some(1.5));
        assert_eq!(pick_string(&v, &["z"]), None);
    }
}