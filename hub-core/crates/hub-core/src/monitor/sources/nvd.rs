//! NVD (NIST National Vulnerability Database) CVE feed 2.0 — the
//! authoritative US government catalog of every CVE with CVSS v3 scores.
//! Complements cisakev (which only tracks actively-exploited entries):
//! NVD gives the full corpus so the Radar map can surface "all recent
//! HIGH+ CRITICAL CVEs", not just KEV-curated ones. Free, no key
//! required (5 req/30s); with HUB_NVD_API_KEY the cap rises to 50 req/30s.
//!
//! Anchor at NIST Gaithersburg HQ (different point from CISA HQ used by
//! cisakev — both agencies live in the DC metro but the visual separation
//! keeps two CVEs that appear in BOTH feeds readable on the map).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const FEED_BASE: &str = "https://services.nvd.nist.gov/rest/json/cves/2.0";
const NIST_HQ: (f64, f64) = (39.1003, -77.2210); // Gaithersburg, MD
const LOOKBACK_HOURS: i64 = 24;
const MAX_PER_SWEEP: usize = 20;

pub struct Nvd;

impl Source for Nvd {
    fn name(&self) -> &'static str {
        "nvd"
    }
    fn interval(&self) -> Duration {
        // 6h cadence: HIGH/CRITICAL CVEs land faster than that, so a 6h
        // sweep captures every meaningful change without spamming signals.
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // NVD API 2.0 uses lastModStartDate / lastModEndDate (ISO 8601
            // UTC). 24h lookback keeps the response bounded even on busy
            // days (~30-60 HIGH+CRITICAL per 24h in our observation).
            let end = chrono::Utc::now();
            let start = end - chrono::Duration::hours(LOOKBACK_HOURS);
            let start_str = start.format("%Y-%m-%dT%H:%M:%S.000").to_string();
            let end_str = end.format("%Y-%m-%dT%H:%M:%S.000").to_string();

            // cvssV3Severity=HIGH and CRITICAL are independent query params
            // (NVD accepts the same param repeated). resultsPerPage caps
            // response size to keep memory bounded.
            let url = format!(
                "{FEED_BASE}?lastModStartDate={start_str}&lastModEndDate={end_str}\
                 &cvssV3Severity=HIGH&cvssV3Severity=CRITICAL&resultsPerPage=50"
            );

            let mut req = ctx.http.get(&url);
            if let Some(key) = &ctx.config.monitor_nvd_api_key {
                req = req.header("apiKey", key);
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("NVD HTTP {}", resp.status())));
            }
            let j: serde_json::Value = resp.json().await?;

            let vulns = j
                .get("vulnerabilities")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut out = Vec::new();
            for item in vulns.into_iter().take(MAX_PER_SWEEP) {
                let Some(cve) = item.get("cve") else { continue };
                let cve_id = cve.get("id").and_then(|x| x.as_str()).unwrap_or("");
                if cve_id.is_empty() {
                    continue;
                }

                // English description preferred; falls back to first available.
                let description = cve
                    .get("descriptions")
                    .and_then(|d| d.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .find(|d| {
                                d.get("lang").and_then(|l| l.as_str()) == Some("en")
                            })
                            .or_else(|| arr.first())
                    })
                    .and_then(|d| d.get("value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // CVSS v3.1 score + severity. Older CVEs may only carry
                // v2 — we keep them at "routine" rather than dropping.
                let (cvss_score, cvss_severity) = cve
                    .get("metrics")
                    .and_then(|m| m.get("cvssMetricV31"))
                    .and_then(|arr| arr.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|m| m.get("cvssData"))
                    .map(|d| {
                        let score = d
                            .get("baseScore")
                            .and_then(|s| s.as_f64())
                            .unwrap_or(0.0);
                        let sev = d
                            .get("baseSeverity")
                            .and_then(|s| s.as_str())
                            .unwrap_or("UNKNOWN")
                            .to_string();
                        (score, sev)
                    })
                    .unwrap_or((0.0, "UNKNOWN".to_string()));

                // NVD's `cvssV3Severity=HIGH` query param is a server-side
                // hint, not a strict gate — some CVEs return with v3.1
                // baseSeverity=MEDIUM because NVD also surfaces CVSS v2 /
                // supplemental ADP v3 metrics in the response. Drop those
                // client-side so the geo_events contract (only HIGH /
                // CRITICAL NVD signals) stays clean.
                if !matches!(cvss_severity.as_str(), "HIGH" | "CRITICAL") {
                    continue;
                }

                let severity = match cvss_severity.as_str() {
                    "CRITICAL" => "flash",
                    "HIGH" => "priority",
                    _ => "routine",
                };

                let published_at = cve
                    .get("published")
                    .and_then(|p| p.as_str())
                    .and_then(|p| chrono::DateTime::parse_from_rfc3339(p).ok())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);

                // Title format: "{CVE-ID} ({SEVERITY} {score}): {first 120 chars of desc}"
                let title = {
                    let desc_short: String = description.chars().take(120).collect();
                    format!("{cve_id} ({cvss_severity} {cvss_score:.1}): {desc_short}")
                };

                let references: Vec<String> = cve
                    .get("references")
                    .and_then(|r| r.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|r| {
                                r.get("url")
                                    .and_then(|u| u.as_str())
                                    .map(String::from)
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let cwe_ids: Vec<String> = cve
                    .get("weaknesses")
                    .and_then(|w| w.as_array())
                    .map(|arr| {
                        arr.iter()
                            .flat_map(|w| {
                                w.get("description")
                                    .and_then(|d| d.as_array())
                                    .into_iter()
                                    .flat_map(|ds| {
                                        ds.iter().filter_map(|d| {
                                            d.get("value").and_then(|v| v.as_str()).map(String::from)
                                        })
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                // External_id uses nvd-cve: prefix to stay independent from
                // cisakev's cve: prefix — both can coexist for CVEs that
                // land in KEV *and* NVD, surfacing two distinct payloads.
                out.push(
                    Signal::new(
                        "cyber",
                        title,
                        NIST_HQ.0,
                        NIST_HQ.1,
                        format!("nvd-cve:{cve_id}"),
                    )
                    .severity(match severity {
                        "flash" => "flash",
                        "priority" => "priority",
                        _ => "routine",
                    })
                    .occurred(published_at)
                    .payload(serde_json::json!({
                        "cve": cve_id,
                        "cvss_score": cvss_score,
                        "cvss_severity": cvss_severity,
                        "references": references,
                        "cwe": cwe_ids,
                        "description": description,
                    })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}
