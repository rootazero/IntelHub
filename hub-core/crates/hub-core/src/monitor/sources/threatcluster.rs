//! ThreatCluster Public API (https://threatcluster.io/api/public/v1) —
//! incident clustering + CVE exploitation evidence. Phase 4.1 of the
//! public-API integration roadmap (`docs/superpowers/roadmaps/
//! 2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Polls two complementary endpoints every 24h:
//! - `GET /api/public/v1/threats?time_filter=24h` — recent threat
//!   clusters (incidents grouped across news, dark-web, telemetry).
//! - `GET /api/public/v1/vulnerabilities?kev=true&exploited=true`
//!   — CVEs actively exploited in the wild (CISA KEV listed OR
//!   public exploit code observed).
//!
//! Compounds with the existing `nvd` + `osv` + `cisa_kev`
//! collectors by adding **incident context** (which actors hit
//! which vendors this week) + **exploit timeline** (when the
//! exploit went public). A `priority` hit here means "this CVE
//! is being weaponized by ransomware groups NOW" — much higher
//! signal than "CVSS 9.8 unpatched CVE in 2021".
//!
//! ## Auth
//!
//! `HUB_THREATCLUSTER_API_KEY` (env-gated, mirrors Currents +
//! CompliAPI). ThreatCluster offers a free tier at
//! https://threatcluster.io (100 req/day, no card). Without key →
//! source NOT registered; sp6 reports `shelved-by-design` until
//! signup. Re-reads env at each sweep so a fresh key takes effect
//! on the next interval (no restart needed for rotation).
//!
//! ## Severity ladder
//!
//! Threats:
//! - `severity == "critical"` → priority (live exploit, ransomware
//!   in-progress)
//! - `severity == "high"` OR `confidence >= 0.8` → routine
//! - else → info
//!
//! Vulnerabilities:
//! - `kev == true` AND `exploited_in_wild == true` → priority
//!   (CISA-mandated remediation, active weaponization)
//! - `kev == true` OR `exploited_in_wild == true` → routine
//! - else → info (background CVE pressure, ambient baseline)
//!
//! ## external_id
//!
//! Threats: `threatcluster:{cluster_id}` — ThreatCluster IDs are
//! stable cluster identifiers (`TC-2026-XXXX` style).
//!
//! Vulnerabilities: `threatcluster:cve:{cve_id}` — joins on CVE ID
//! so dedups vs NVD/OSV/CISA_KEV collectors naturally.
//!
//! ## Cadence
//!
//! 24h — matches the existing NVD/OSV/CISA_KEV cadence so dedup
//! windows align. Free tier allows 100 req/day; we use 2 per day
//! (threats + vulnerabilities) = well under the throttle.
//!
//! ## Cap
//!
//! TOP_N_THREATS = 15 (most recent 24h clusters).
//! TOP_N_VULNS = 25 (most active exploited CVEs).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://threatcluster.io/api/public/v1";

const ENV_KEY: &str = "HUB_THREATCLUSTER_API_KEY";

const INTERVAL_SECS: u64 = 24 * 3600;

const TOP_N_THREATS: usize = 15;
const TOP_N_VULNS: usize = 25;

pub struct ThreatCluster;

impl Source for ThreatCluster {
    fn name(&self) -> &'static str {
        "threatcluster"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(api_key) = api_key() else {
                tracing::warn!(
                    "threatcluster: no {ENV_KEY} — collector not registered; \
                     sp6 will report 'shelved-by-design' until signup at \
                     https://threatcluster.io (free tier, no card)"
                );
                return Ok(Vec::new());
            };
            let mut all = Vec::new();
            // 1) Recent threat clusters
            match fetch_threats(ctx, &api_key).await {
                Ok(v) => all.extend(v),
                Err(e) => tracing::warn!(error = %e, "threatcluster threats fetch failed"),
            }
            // 2) Active exploited CVEs
            match fetch_vulnerabilities(ctx, &api_key).await {
                Ok(v) => all.extend(v),
                Err(e) => tracing::warn!(error = %e, "threatcluster vulnerabilities fetch failed"),
            }
            Ok(all)
        }
        .boxed()
    }
}

fn api_key() -> Option<String> {
    std::env::var(ENV_KEY)
        .ok()
        .filter(|s| !s.is_empty())
}

async fn fetch_threats(ctx: &Ctx, api_key: &str) -> Result<Vec<Signal>> {
    let resp = ctx
        .http
        .get(format!("{BASE_URL}/threats"))
        .header("X-API-Key", api_key)
        .query(&[("time_filter", "24h")])
        .send()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("threatcluster http: {e}")))?;
    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "threatcluster threats non-2xx");
        return Ok(Vec::new());
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("threatcluster json: {e}")))?;
    Ok(parse_threats(&body))
}

async fn fetch_vulnerabilities(ctx: &Ctx, api_key: &str) -> Result<Vec<Signal>> {
    let resp = ctx
        .http
        .get(format!("{BASE_URL}/vulnerabilities"))
        .header("X-API-Key", api_key)
        .query(&[
            ("kev", "true"),
            ("exploited", "true"),
            ("limit", &TOP_N_VULNS.to_string()),
        ])
        .send()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("threatcluster http: {e}")))?;
    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "threatcluster vulnerabilities non-2xx");
        return Ok(Vec::new());
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("threatcluster json: {e}")))?;
    Ok(parse_vulnerabilities(&body))
}

/// Parse threats response. Expected shape per docs: list of cluster
/// objects under `data[]` (or root array). Defensive: tries root
/// array → `data[]` → `items[]` → `results[]` keys, returns [] on
/// miss.
fn parse_threats(j: &serde_json::Value) -> Vec<Signal> {
    let arr = first_array(j).unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len().min(TOP_N_THREATS));
    for r in arr.iter().take(TOP_N_THREATS) {
        let cluster_id = pick_string(r, &["cluster_id", "id", "identifier"]).unwrap_or_default();
        let title = pick_string(r, &["title", "name", "headline"])
            .unwrap_or_else(|| "Untitled threat cluster".to_string());
        let severity = pick_string(r, &["severity", "priority"]).unwrap_or_default();
        let confidence = pick_number(r, &["confidence", "confidence_score"]).unwrap_or(0.0);
        let category = pick_string(r, &["category", "type", "threat_type"]).unwrap_or_default();
        let actor = pick_string(r, &["actor", "threat_actor", "group"]).unwrap_or_default();
        let target_sector = pick_string(r, &["target_sector", "sector", "industry"]).unwrap_or_default();
        let target_country = pick_string(r, &["target_country", "country"]).unwrap_or_default();
        let source_count = pick_number(r, &["source_count", "sources", "mention_count"])
            .unwrap_or(0.0) as i64;
        let summary = pick_string(r, &["summary", "description"]).unwrap_or_default();
        let created_at = pick_string(r, &["created_at", "first_seen", "timestamp"])
            .unwrap_or_default();
        let url = pick_string(r, &["url", "source_url"]).unwrap_or_default();
        let (sev, kind) = classify_threat(&severity, confidence);
        let ext_id = if !cluster_id.is_empty() {
            format!("threatcluster:{cluster_id}")
        } else {
            format!("threatcluster:t:{}", short_hash(&title))
        };
        out.push(
            Signal::new(kind, title, 0.0, 0.0, ext_id)
                .severity(sev)
                .payload(serde_json::json!({
                    "kind": "threatcluster_threat",
                    "cluster_id": cluster_id,
                    "severity": severity,
                    "confidence": confidence,
                    "category": category,
                    "actor": actor,
                    "target_sector": target_sector,
                    "target_country": target_country,
                    "source_count": source_count,
                    "summary": summary,
                    "created_at": created_at,
                    "url": url,
                })),
        );
    }
    out
}

/// Parse vulnerabilities response. Expected shape: `{data: [{cve_id,
/// kev, exploited_in_wild, cvss, epss, severity, ...}]}`.
fn parse_vulnerabilities(j: &serde_json::Value) -> Vec<Signal> {
    let arr = first_array(j).unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len().min(TOP_N_VULNS));
    for r in arr.iter().take(TOP_N_VULNS) {
        let cve_id = pick_string(r, &["cve_id", "id", "cve"]).unwrap_or_default();
        if cve_id.is_empty() {
            continue; // skip rows without CVE ID — can't dedup
        }
        let title = pick_string(r, &["title", "name"])
            .unwrap_or_else(|| format!("{cve_id} active exploit"));
        let kev = pick_bool(r, &["kev", "in_kev", "is_kev"]).unwrap_or(false);
        let exploited_in_wild =
            pick_bool(r, &["exploited_in_wild", "exploited", "wild_exploit"]).unwrap_or(false);
        let cvss = pick_number(r, &["cvss", "cvss_score"]).unwrap_or(0.0);
        let epss = pick_number(r, &["epss", "epss_score"]).unwrap_or(0.0);
        let severity = pick_string(r, &["severity", "cvss_severity"]).unwrap_or_default();
        let vendor = pick_string(r, &["vendor", "affected_vendor"]).unwrap_or_default();
        let product = pick_string(r, &["product", "affected_product"]).unwrap_or_default();
        let exploit_kind =
            pick_string(r, &["exploit_kind", "exploit_type"]).unwrap_or_default();
        let ransomware_use =
            pick_string(r, &["ransomware_use", "ransomware_family"]).unwrap_or_default();
        let first_exploited = pick_string(r, &["first_exploited", "exploit_first_seen"])
            .unwrap_or_default();
        let (sev, kind) = classify_vuln(kev, exploited_in_wild);
        out.push(
            Signal::new(kind, title, 0.0, 0.0, format!("threatcluster:cve:{cve_id}"))
                .severity(sev)
                .payload(serde_json::json!({
                    "kind": "threatcluster_vuln",
                    "cve_id": cve_id,
                    "kev": kev,
                    "exploited_in_wild": exploited_in_wild,
                    "cvss": cvss,
                    "epss": epss,
                    "severity": severity,
                    "vendor": vendor,
                    "product": product,
                    "exploit_kind": exploit_kind,
                    "ransomware_use": ransomware_use,
                    "first_exploited": first_exploited,
                })),
        );
    }
    out
}

fn classify_threat(severity: &str, confidence: f64) -> (&'static str, &'static str) {
    if severity.eq_ignore_ascii_case("critical") {
        return ("priority", "threatcluster_threat_critical");
    }
    if severity.eq_ignore_ascii_case("high") || confidence >= 0.8 {
        return ("routine", "threatcluster_threat_high");
    }
    ("info", "threatcluster_threat_low")
}

fn classify_vuln(kev: bool, exploited_in_wild: bool) -> (&'static str, &'static str) {
    if kev && exploited_in_wild {
        return ("priority", "threatcluster_vuln_kev_exploited");
    }
    if kev || exploited_in_wild {
        return ("routine", "threatcluster_vuln_active_exploit");
    }
    ("info", "threatcluster_vuln_background")
}

fn first_array(j: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    if let Some(arr) = j.as_array() {
        return Some(arr.clone());
    }
    for k in &["data", "items", "results", "clusters", "vulnerabilities", "threats"] {
        if let Some(arr) = j.get(k).and_then(|v| v.as_array()) {
            return Some(arr.clone());
        }
    }
    None
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

fn pick_bool(v: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    for k in keys {
        if let Some(b) = v.get(*k).and_then(|x| x.as_bool()) {
            return Some(b);
        }
    }
    None
}

fn short_hash(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let out = h.finalize();
    hex::encode(&out[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_threats() -> serde_json::Value {
        serde_json::json!({
            "data": [
                {
                    "cluster_id": "TC-2026-0001",
                    "title": "Active ransomware campaign targeting healthcare",
                    "severity": "critical",
                    "confidence": 0.95,
                    "category": "ransomware",
                    "actor": "BlackCat",
                    "target_sector": "healthcare",
                    "target_country": "US",
                    "source_count": 42,
                    "summary": "Multi-hospital incident",
                    "created_at": "2026-09-27T08:00:00Z",
                    "url": "https://example.com/cluster/TC-2026-0001"
                },
                {
                    "cluster_id": "TC-2026-0002",
                    "title": "Spear phishing wave against EU diplomats",
                    "severity": "high",
                    "confidence": 0.85,
                    "category": "phishing",
                    "actor": "APT29",
                    "target_sector": "government",
                    "target_country": "EU",
                    "source_count": 18
                },
                {
                    "cluster_id": "TC-2026-0003",
                    "title": "Low-confidence DDoS chatter",
                    "severity": "medium",
                    "confidence": 0.4
                }
            ]
        })
    }

    fn sample_vulns() -> serde_json::Value {
        serde_json::json!({
            "data": [
                {
                    "cve_id": "CVE-2026-1234",
                    "title": "Windows kernel RCE",
                    "kev": true,
                    "exploited_in_wild": true,
                    "cvss": 9.8,
                    "epss": 0.87,
                    "severity": "CRITICAL",
                    "vendor": "Microsoft",
                    "product": "Windows",
                    "exploit_kind": "rce",
                    "ransomware_use": "BlackCat",
                    "first_exploited": "2026-09-15"
                },
                {
                    "cve_id": "CVE-2026-5678",
                    "title": "Apache path traversal",
                    "kev": true,
                    "exploited_in_wild": false,
                    "cvss": 7.5,
                    "vendor": "Apache",
                    "product": "httpd"
                },
                {
                    "cve_id": "CVE-2026-9012",
                    "title": "Linux kernel info leak",
                    "kev": false,
                    "exploited_in_wild": true,
                    "cvss": 5.3,
                    "vendor": "Linux",
                    "product": "kernel"
                },
                {
                    "cve_id": "CVE-2026-3456",
                    "title": "Theoretical CVE",
                    "kev": false,
                    "exploited_in_wild": false,
                    "cvss": 6.5
                }
            ]
        })
    }

    /// critical severity → priority
    #[test]
    fn threat_critical_is_priority() {
        let sigs = parse_threats(&sample_threats());
        assert_eq!(sigs.len(), 3);
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "threatcluster_threat_critical");
    }

    /// high severity or confidence >= 0.8 → routine
    #[test]
    fn threat_high_or_confidence_is_routine() {
        let sigs = parse_threats(&sample_threats());
        assert_eq!(sigs[1].severity, "routine");
        assert_eq!(sigs[1].kind, "threatcluster_threat_high");
    }

    /// medium severity + low confidence → info
    #[test]
    fn threat_low_is_info() {
        let sigs = parse_threats(&sample_threats());
        assert_eq!(sigs[2].severity, "info");
        assert_eq!(sigs[2].kind, "threatcluster_threat_low");
    }

    /// external_id shape = threatcluster:{cluster_id}
    #[test]
    fn threat_external_id() {
        let sigs = parse_threats(&sample_threats());
        assert_eq!(sigs[0].external_id, "threatcluster:TC-2026-0001");
    }

    /// KEV + exploited → priority
    #[test]
    fn vuln_kev_and_exploited_is_priority() {
        let sigs = parse_vulnerabilities(&sample_vulns());
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "threatcluster_vuln_kev_exploited");
    }

    /// KEV only OR exploited only → routine
    #[test]
    fn vuln_kev_only_is_routine() {
        let sigs = parse_vulnerabilities(&sample_vulns());
        // CVE-2026-5678: kev=true, exploited=false → routine
        let sig = sigs.iter().find(|s| s.external_id.contains("5678")).unwrap();
        assert_eq!(sig.severity, "routine");
        assert_eq!(sig.kind, "threatcluster_vuln_active_exploit");
        // CVE-2026-9012: kev=false, exploited=true → routine
        let sig = sigs.iter().find(|s| s.external_id.contains("9012")).unwrap();
        assert_eq!(sig.severity, "routine");
    }

    /// No KEV + no exploit → info
    #[test]
    fn vuln_neither_is_info() {
        let sigs = parse_vulnerabilities(&sample_vulns());
        let sig = sigs.iter().find(|s| s.external_id.contains("3456")).unwrap();
        assert_eq!(sig.severity, "info");
        assert_eq!(sig.kind, "threatcluster_vuln_background");
    }

    /// vulnerability external_id = threatcluster:cve:{cve_id}
    #[test]
    fn vuln_external_id() {
        let sigs = parse_vulnerabilities(&sample_vulns());
        assert_eq!(sigs[0].external_id, "threatcluster:cve:CVE-2026-1234");
    }

    /// Skip rows with empty cve_id (defensive)
    #[test]
    fn vuln_skips_empty_cve() {
        let j = serde_json::json!({"data": [{"title": "no cve"}, {"cve_id": "CVE-2026-9999"}]});
        let sigs = parse_vulnerabilities(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].external_id, "threatcluster:cve:CVE-2026-9999");
    }

    /// Tries root array, then data/items/results/clusters/etc keys.
    #[test]
    fn first_array_handles_all_shapes() {
        assert_eq!(first_array(&serde_json::json!([1, 2])).unwrap().len(), 2);
        assert_eq!(
            first_array(&serde_json::json!({"data": [1, 2]})).unwrap().len(),
            2
        );
        assert_eq!(
            first_array(&serde_json::json!({"items": [1]})).unwrap().len(),
            1
        );
        assert_eq!(
            first_array(&serde_json::json!({"results": [1]})).unwrap().len(),
            1
        );
        assert_eq!(
            first_array(&serde_json::json!({"clusters": [1]})).unwrap().len(),
            1
        );
        assert!(first_array(&serde_json::json!({"x": 1})).is_none());
    }

    /// pick_string tolerates missing keys
    #[test]
    fn pick_string_tries_multiple_keys() {
        let v = serde_json::json!({"foo": "F", "bar": "B"});
        assert_eq!(pick_string(&v, &["a", "foo"]), Some("F".to_string()));
        assert_eq!(pick_string(&v, &["bar", "foo"]), Some("B".to_string()));
        assert_eq!(pick_string(&v, &["zzz"]), None);
    }

    /// pick_number, pick_bool similar
    #[test]
    fn pick_helpers_numeric_bool() {
        let v = serde_json::json!({"a": 1.5, "b": true, "c": false});
        assert_eq!(pick_number(&v, &["a"]), Some(1.5));
        assert_eq!(pick_bool(&v, &["b"]), Some(true));
        assert_eq!(pick_bool(&v, &["c"]), Some(false));
        assert_eq!(pick_bool(&v, &["zzz"]), None);
    }

    /// Threats cap respected (TOP_N_THREATS)
    #[test]
    fn threats_caps() {
        let big: Vec<_> = (0..20)
            .map(|i| {
                serde_json::json!({
                    "cluster_id": format!("TC-{}", i),
                    "title": format!("threat {i}"),
                    "severity": "high",
                    "confidence": 0.9,
                })
            })
            .collect();
        let j = serde_json::json!({"data": big});
        let sigs = parse_threats(&j);
        assert_eq!(sigs.len(), TOP_N_THREATS);
    }

    /// Vulnerabilities cap respected (TOP_N_VULNS)
    #[test]
    fn vulns_caps() {
        let big: Vec<_> = (0..50)
            .map(|i| {
                serde_json::json!({
                    "cve_id": format!("CVE-2026-{:04}", i),
                    "title": format!("vuln {i}"),
                    "kev": true,
                    "exploited_in_wild": true,
                })
            })
            .collect();
        let j = serde_json::json!({"data": big});
        let sigs = parse_vulnerabilities(&j);
        assert_eq!(sigs.len(), TOP_N_VULNS);
    }
}