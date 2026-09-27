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

pub fn api_key() -> Option<String> {
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
        let urgency = pick_string(r, &["urgency_level", "severity", "priority"]).unwrap_or_default();
        let threat_score = pick_number(r, &["threat_score", "confidence", "confidence_score"])
            .unwrap_or(0.0);
        let category = pick_string(r, &["category", "type", "threat_type"]).unwrap_or_default();
        let actor = pick_string(r, &["actor", "threat_actor", "group"]).unwrap_or_default();
        let target_sector = pick_string(r, &["target_sector", "sector", "industry"]).unwrap_or_default();
        let target_country = pick_string(r, &["target_country", "country"]).unwrap_or_default();
        let article_count = pick_number(r, &["article_count", "source_count", "sources", "mention_count"])
            .unwrap_or(0.0) as i64;
        let summary = pick_string(r, &["summary", "description"]).unwrap_or_default();
        let created_at = pick_string(r, &["created_at", "first_seen", "timestamp"])
            .unwrap_or_default();
        let url = pick_string(r, &["url", "source_url"]).unwrap_or_default();
        let (sev, kind) = classify_threat(&urgency, threat_score);
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
                    "urgency_level": urgency,
                    "threat_score": threat_score,
                    "category": category,
                    "actor": actor,
                    "target_sector": target_sector,
                    "target_country": target_country,
                    "article_count": article_count,
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
        let in_kev = pick_bool(r, &["in_kev", "kev", "is_kev"]).unwrap_or(false);
        let has_exploit = pick_bool(r, &["has_exploit", "exploited_in_wild", "exploited"])
            .unwrap_or(false);
        let cvss = pick_number(r, &["cvss_v3_score", "cvss", "cvss_score"]).unwrap_or(0.0);
        let exploit_count =
            pick_number(r, &["exploit_count"]).unwrap_or(0.0) as i64;
        let severity = pick_string(r, &["cvss_v3_severity", "severity"]).unwrap_or_default();
        let description = pick_string(r, &["description", "summary"]).unwrap_or_default();
        let published_date = pick_string(r, &["published_date", "published"]).unwrap_or_default();
        let last_modified = pick_string(r, &["last_modified", "modified"]).unwrap_or_default();
        let (sev, kind) = classify_vuln(in_kev, has_exploit, &severity, cvss);
        out.push(
            Signal::new(kind, title, 0.0, 0.0, format!("threatcluster:cve:{cve_id}"))
                .severity(sev)
                .payload(serde_json::json!({
                    "kind": "threatcluster_vuln",
                    "cve_id": cve_id,
                    "in_kev": in_kev,
                    "has_exploit": has_exploit,
                    "cvss_v3_score": cvss,
                    "cvss_v3_severity": severity,
                    "exploit_count": exploit_count,
                    "description": description,
                    "published_date": published_date,
                    "last_modified": last_modified,
                })),
        );
    }
    out
}

fn classify_threat(urgency: &str, threat_score: f64) -> (&'static str, &'static str) {
    let u = urgency.to_lowercase();
    if u == "critical" || threat_score >= 80.0 {
        return ("priority", "threatcluster_threat_critical");
    }
    if u == "high" || threat_score >= 50.0 {
        return ("routine", "threatcluster_threat_high");
    }
    ("info", "threatcluster_threat_low")
}

fn classify_vuln(
    in_kev: bool,
    has_exploit: bool,
    cvss_severity: &str,
    cvss: f64,
) -> (&'static str, &'static str) {
    let s = cvss_severity.to_uppercase();
    if s == "CRITICAL" || (in_kev && has_exploit) {
        return ("priority", "threatcluster_vuln_critical");
    }
    if in_kev || has_exploit || s == "HIGH" || cvss >= 7.0 {
        return ("routine", "threatcluster_vuln_active");
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
    let h = Sha256::digest(s.as_bytes());
    let hex = format!("{:x}", h);
    hex.chars().take(16).collect()
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
                    "urgency_level": "high",
                    "threat_score": 72.0,
                    "category": "phishing",
                    "actor": "APT29",
                    "target_sector": "government",
                    "target_country": "EU",
                    "article_count": 18
                },
                {
                    "cluster_id": "TC-2026-0003",
                    "title": "Low-confidence DDoS chatter",
                    "urgency_level": "medium",
                    "threat_score": 32.0
                }
            ]
        })
    }

    fn sample_vulns() -> serde_json::Value {
        serde_json::json!({
            "cves": [
                {
                    "cve_id": "CVE-2026-1234",
                    "description": "Windows kernel RCE",
                    "cvss_v3_score": 9.8,
                    "cvss_v3_severity": "CRITICAL",
                    "in_kev": true,
                    "has_exploit": true,
                    "exploit_count": 3,
                    "published_date": "2026-09-15T00:00:00"
                },
                {
                    "cve_id": "CVE-2026-5678",
                    "description": "Apache path traversal",
                    "cvss_v3_score": 7.5,
                    "cvss_v3_severity": "HIGH",
                    "in_kev": true,
                    "has_exploit": false,
                    "exploit_count": 0
                },
                {
                    "cve_id": "CVE-2026-9012",
                    "description": "Linux kernel info leak",
                    "cvss_v3_score": 5.3,
                    "cvss_v3_severity": "MEDIUM",
                    "in_kev": false,
                    "has_exploit": true,
                    "exploit_count": 1
                },
                {
                    "cve_id": "CVE-2026-3456",
                    "description": "Theoretical CVE",
                    "cvss_v3_score": 6.5,
                    "cvss_v3_severity": "MEDIUM",
                    "in_kev": false,
                    "has_exploit": false,
                    "exploit_count": 0
                }
            ]
        })
    }

    /// critical urgency OR threat_score >= 80 → priority
    #[test]
    fn threat_critical_is_priority() {
        let sigs = parse_threats(&sample_threats());
        assert_eq!(sigs.len(), 3);
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "threatcluster_threat_critical");
    }

    /// high urgency OR threat_score >= 50 → routine
    #[test]
    fn threat_high_or_score_is_routine() {
        let sigs = parse_threats(&sample_threats());
        assert_eq!(sigs[1].severity, "routine");
        assert_eq!(sigs[1].kind, "threatcluster_threat_high");
    }

    /// medium urgency + low threat_score → info
    #[test]
    fn threat_low_is_info() {
        let sigs = parse_threats(&sample_threats());
        assert_eq!(sigs[2].severity, "info");
        assert_eq!(sigs[2].kind, "threatcluster_threat_low");
    }

    /// threat_score >= 80 alone → priority even without "critical" urgency
    #[test]
    fn threat_score_drives_priority() {
        let mut j = sample_threats();
        j["threats"].as_array_mut().unwrap().push(serde_json::json!({
            "cluster_id": "TC-score-90", "title": "high score",
            "urgency_level": "medium", "threat_score": 90.0
        }));
        let sigs = parse_threats(&j);
        let high = sigs.iter().find(|s| s.external_id.contains("score-90")).unwrap();
        assert_eq!(high.severity, "priority");
    }

    /// external_id shape = threatcluster:{cluster_id}
    #[test]
    fn threat_external_id() {
        let sigs = parse_threats(&sample_threats());
        assert_eq!(sigs[0].external_id, "threatcluster:TC-2026-0001");
    }

    /// KEV + has_exploit → priority
    #[test]
    fn vuln_kev_and_exploited_is_priority() {
        let sigs = parse_vulnerabilities(&sample_vulns());
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "threatcluster_vuln_critical");
    }

    /// KEV only OR has_exploit only OR HIGH severity → routine
    #[test]
    fn vuln_kev_only_is_routine() {
        let sigs = parse_vulnerabilities(&sample_vulns());
        // CVE-2026-5678: in_kev=true, has_exploit=false, severity=HIGH → routine
        let sig = sigs.iter().find(|s| s.external_id.contains("5678")).unwrap();
        assert_eq!(sig.severity, "routine");
        assert_eq!(sig.kind, "threatcluster_vuln_active");
        // CVE-2026-9012: in_kev=false, has_exploit=true → routine
        let sig = sigs.iter().find(|s| s.external_id.contains("9012")).unwrap();
        assert_eq!(sig.severity, "routine");
    }

    /// No KEV + no exploit + MEDIUM severity → info
    #[test]
    fn vuln_neither_is_info() {
        let sigs = parse_vulnerabilities(&sample_vulns());
        let sig = sigs.iter().find(|s| s.external_id.contains("3456")).unwrap();
        assert_eq!(sig.severity, "info");
        assert_eq!(sig.kind, "threatcluster_vuln_background");
    }

    /// CRITICAL cvss_v3_severity alone → priority
    #[test]
    fn vuln_critical_severity_is_priority() {
        let j = serde_json::json!({"cves": [{
            "cve_id": "CVE-2026-CCCC",
            "cvss_v3_score": 9.0,
            "cvss_v3_severity": "CRITICAL",
            "in_kev": false,
            "has_exploit": false
        }]});
        let sigs = parse_vulnerabilities(&j);
        assert_eq!(sigs[0].severity, "priority");
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
        let j = serde_json::json!({"cves": [{"description": "no cve"}, {"cve_id": "CVE-2026-9999"}]});
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