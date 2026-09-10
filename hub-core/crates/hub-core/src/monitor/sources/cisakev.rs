//! CISA Known Exploited Vulnerabilities (KEV) — the US government's
//! "patch NOW" catalog. Free JSON feed, no key. Cyber threat intelligence
//! has no natural coordinates → anchored at CISA HQ (same honesty pattern
//! as the RSS feed-HQ fallbacks); dedup on CVE id means the map only gains
//! a point when a NEW vulnerability lands.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const FEED: &str =
    "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json";
const CISA_HQ: (f64, f64) = (38.8977, -77.0365); // Washington DC

pub struct CisaKev;

impl Source for CisaKev {
    fn name(&self) -> &'static str {
        "cisa-kev"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let resp = ctx.http.get(FEED).send().await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("CISA KEV HTTP {}", resp.status())));
            }
            let j: serde_json::Value = resp.json().await?;
            let mut vulns: Vec<serde_json::Value> = j
                .get("vulnerabilities")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            // Newest by dateAdded, keep a dozen per sweep.
            vulns.sort_by_key(|v| {
                std::cmp::Reverse(v.get("dateAdded").and_then(|d| d.as_str()).map(String::from))
            });
            let mut out = Vec::new();
            for v in vulns.into_iter().take(12) {
                let cve = v.get("cveID").and_then(|c| c.as_str()).unwrap_or("").to_string();
                if cve.is_empty() {
                    continue;
                }
                let name = v.get("vulnerabilityName").and_then(|n| n.as_str()).unwrap_or("");
                let vendor = v.get("vendorProject").and_then(|n| n.as_str()).unwrap_or("");
                let product = v.get("product").and_then(|n| n.as_str()).unwrap_or("");
                let title = {
                    let t = format!("{cve} {vendor} {product}: {name}");
                    t.chars().take(180).collect::<String>()
                };
                let occurred_at = v
                    .get("dateAdded")
                    .and_then(|d| d.as_str())
                    .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|d| chrono::DateTime::from_naive_utc_and_offset(d, chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);
                out.push(
                    Signal::new("cyber", title, CISA_HQ.0, CISA_HQ.1, format!("cve:{cve}"))
                        .severity("priority")
                        .occurred(occurred_at)
                        .payload(serde_json::json!({
                            "cve": cve,
                            "vendor": vendor,
                            "product": product,
                            "due_date": v.get("dueDate").and_then(|d| d.as_str()),
                            "ransomware": v.get("knownRansomwareCampaignUse").and_then(|r| r.as_str()),
                        })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}
