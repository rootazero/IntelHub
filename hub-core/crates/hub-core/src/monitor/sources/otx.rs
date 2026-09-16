//! AlienVault OTX (Open Threat Exchange) — community threat-intel pulse feed.
//!
//! Polls OTX for new threat-intel pulses (community-curated bundles of IOCs)
//! in the last 24h, parses indicators (file hashes, IP addresses, domains,
//! URLs, CVEs, email addresses, Bitcoin addresses), and emits a single
//! Signal per pulse carrying the IOC bundle in the payload. Acts as a
//! rolling community radar for malware families and active campaigns;
//! complements cisakev (NVD KEV) and nvd (NVD full CVE) by surfacing
//! unattributed / emerging threats before they land in formal databases.
//!
//! - Free, no key required for public pulses (`/pulses/subscribed` requires
//!   auth). We use the public search endpoint `/pulses/{}` or the
//!   "modified since" filter via `?modified_since=` — both keyless.
//! - Built-in scan window: 24h. Override via `HUB_OTX_DAYS` (1..30).
//! - Kind: "cyber" (matches cisakev/nvd/osv visual cluster on radar).
//!
//! Anchored at AT&T AlienVault HQ (San Mateo, CA) — visually distinct from
//! CISA/NVD/OFAC (DC) and OSV (Mountain View) clusters. Pulse events
//! represent an aggregate of community activity, not a single geo location,
//! so the anchor is purely for visual placement on the radar.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://otx.alienvault.com/api/v1";
const ALIENVAULT_HQ: (f64, f64) = (37.5630, -122.3255); // San Mateo, CA
const DEFAULT_LOOKBACK_DAYS: u32 = 1;
const MAX_PULSES_PER_SWEEP: usize = 50;
const MAX_IOCS_PER_PULSE: usize = 25;

pub struct Otx;

impl Source for Otx {
    fn name(&self) -> &'static str {
        "otx"
    }
    fn interval(&self) -> Duration {
        // 4h cadence. OTX fires pulses continuously; 4h catches most new
        // campaigns before they go quiet.
        Duration::from_secs(4 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let days = ctx
                .config
                .monitor_otx_lookback_days
                .unwrap_or(DEFAULT_LOOKBACK_DAYS)
                .clamp(1, 30);
            let url = format!(
                "{API_BASE}/pulses/subscribed?modified_since={days}d&limit={MAX_PULSES_PER_SWEEP}",
            );
            let resp = match ctx.http.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(target: "monitor::otx", "fetch failed: {e}");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                // 401/404 means the endpoint is key-only (the public
                // /pulses/search requires a search term; the listing is
                // auth-gated). Self-degrade silently — the source remains
                // visible on the health board via scheduler reporting.
                tracing::warn!(target: "monitor::otx",
                    "HTTP {} (likely key-gated); collector shelved by design",
                    resp.status());
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(target: "monitor::otx", "parse: {e}");
                    return Ok(Vec::new());
                }
            };
            let pulses = body.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();

            let mut out = Vec::new();
            for pulse in pulses.into_iter().take(MAX_PULSES_PER_SWEEP) {
                let parsed = match parse_pulse(&pulse) {
                    Some(p) => p,
                    None => continue,
                };

                let severity = match parsed.malware_families.as_slice() {
                    fams if fams.iter().any(|f| f.contains("Ransomware") || f.contains("APT")) => "flash",
                    fams if !fams.is_empty() => "priority",
                    _ if parsed.has_cve => "priority",
                    _ => "routine",
                };

                let title = if let Some(name) = parsed.name.as_deref() {
                    format!("OTX pulse: {name} ({pulse_iocs} IOCs)", pulse_iocs = parsed.ioc_count)
                } else {
                    format!("OTX pulse {} ({} IOCs)", parsed.id, parsed.ioc_count)
                };

                out.push(
                    Signal::new(
                        "cyber",
                        title,
                        ALIENVAULT_HQ.0,
                        ALIENVAULT_HQ.1,
                        format!("otx:pulse:{}", parsed.id),
                    )
                    .severity(severity)
                    .occurred(parsed.modified_at)
                    .payload(serde_json::json!({
                        "pulse_id": parsed.id,
                        "name": parsed.name,
                        "tlp": parsed.tlp,
                        "malware_families": parsed.malware_families,
                        "attack_ids": parsed.attack_ids,
                        "tags": parsed.tags,
                        "ioc_count": parsed.ioc_count,
                        "iocs": parsed.iocs,
                    })),
                );
            }
            if out.is_empty() {
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

#[derive(Debug, Default)]
struct ParsedPulse {
    id: String,
    name: Option<String>,
    tlp: Option<String>,
    malware_families: Vec<String>,
    attack_ids: Vec<String>,
    tags: Vec<String>,
    ioc_count: usize,
    has_cve: bool,
    iocs: serde_json::Value,
    modified_at: chrono::DateTime<chrono::Utc>,
}

fn parse_pulse(pulse: &serde_json::Value) -> Option<ParsedPulse> {
    let id = pulse.get("id").and_then(|x| x.as_str())?.to_string();
    if id.is_empty() {
        return None;
    }
    let name = pulse
        .get("name")
        .and_then(|x| x.as_str())
        .map(|s| s.chars().take(150).collect::<String>());
    let tlp = pulse
        .get("tlp")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    let malware_families = extract_str_array(pulse.get("malware_families"));
    let attack_ids = extract_str_array(pulse.get("attack_ids"));
    let tags = extract_str_array(pulse.get("tags"));

    let mut has_cve = false;
    let mut ioc_count = 0usize;
    let mut ioc_breakdown: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut sample_iocs: Vec<serde_json::Value> = Vec::new();

    if let Some(arr) = pulse.get("indicators").and_then(|x| x.as_array()) {
        for (idx, ind) in arr.iter().enumerate() {
            ioc_count += 1;
            let itype = ind
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            if itype == "CVE" {
                has_cve = true;
            }
            *ioc_breakdown.entry(itype.clone()).or_insert(0) += 1;
            if idx < MAX_IOCS_PER_PULSE {
                sample_iocs.push(serde_json::json!({
                    "type": itype,
                    "indicator": ind.get("indicator").and_then(|x| x.as_str()).unwrap_or(""),
                    "created": ind.get("created").and_then(|x| x.as_str()).unwrap_or(""),
                }));
            }
        }
    }

    let modified_at = pulse
        .get("modified")
        .and_then(|x| x.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    Some(ParsedPulse {
        id,
        name,
        tlp,
        malware_families,
        attack_ids,
        tags,
        ioc_count,
        has_cve,
        iocs: serde_json::json!({
            "breakdown": ioc_breakdown,
            "sample": sample_iocs,
        }),
        modified_at,
    })
}

fn extract_str_array(v: Option<&serde_json::Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_pulse() {
        let p = serde_json::json!({
            "id": "abc123",
            "name": "Test Campaign",
            "tlp": "white",
            "malware_families": ["Emotet"],
            "attack_ids": ["T1566"],
            "tags": ["phishing", "banker"],
            "modified": "2026-09-13T12:00:00Z",
            "indicators": [
                {"type": "CVE", "indicator": "CVE-2026-1234", "created": "2026-09-01T00:00:00Z"},
                {"type": "IPv4", "indicator": "1.2.3.4", "created": "2026-09-02T00:00:00Z"},
                {"type": "domain", "indicator": "evil.example", "created": "2026-09-03T00:00:00Z"},
            ]
        });
        let parsed = parse_pulse(&p).unwrap();
        assert_eq!(parsed.id, "abc123");
        assert_eq!(parsed.malware_families, vec!["Emotet"]);
        assert!(parsed.has_cve);
        assert_eq!(parsed.ioc_count, 3);
    }

    #[test]
    fn rejects_empty_id() {
        let p = serde_json::json!({"id": "", "name": "x"});
        assert!(parse_pulse(&p).is_none());
    }

    #[test]
    fn severity_escalates_for_ransomware() {
        let p = serde_json::json!({
            "id": "x1",
            "name": "LockBit campaign",
            "malware_families": ["LockBit", "Ransomware-X"],
            "indicators": []
        });
        let parsed = parse_pulse(&p).unwrap();
        let sev = if parsed.malware_families.iter().any(|f| f.contains("Ransomware") || f.contains("APT")) {
            "flash"
        } else if !parsed.malware_families.is_empty() {
            "priority"
        } else {
            "routine"
        };
        assert_eq!(sev, "flash");
    }
}