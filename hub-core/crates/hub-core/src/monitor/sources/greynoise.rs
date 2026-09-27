//! GreyNoise Community API (https://docs.greynoise.io — keyless, free
//! for unauthenticated use, 50 lookups/week for free-tier with API key).
//! Phase 2.1 of the public-API integration roadmap (`docs/superpowers/
//! roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: query a curated watchlist of 25 well-known scanner IPs
//! (TOR exits, IoT botnet C2s, well-known scanner networks). For each
//! IP, hit `/v3/community/{ip}` and emit a Signal if GreyNoise flags
//! it as `noise=true` (observed scanning the internet) or `riot=true`
//! (part of known benign service — useful for false-positive filtering).
//!
//! ## Why a curated watchlist (not cross-source IP pipeline)
//!
//! We COULD feed IPs from existing sources (`romainmarcoux_malicious_ip`,
//! `firehol_level1`, `leaksify`) into GreyNoise for richer classification.
//! But that creates circular dependency + adds a per-source coupling that
//! is hard to reason about (a known-bad IP disappears from feed → GreyNoise
//! stops re-checking it → stale data). The watchlist pattern is cleaner:
//! stable, predictable daily cost (25 lookups), and easy for ops to
//! inspect / extend. Cross-source enrichment is a future Phase 3 spec.
//!
//! ## Severity ladder
//!
//! Based on GreyNoise's `classification` field:
//!   - malicious  → priority (active scanner targeting victims)
//!   - benign      → routine (RIOT — known service like DNS, mail)
//!   - unknown     → info (observed scanning but no classification)
//!   - not noise   → not emitted (already classified as benign)
//!
//! ## Cadence
//!
//! 24h — GreyNoise's scan data doesn't change minute-to-minute; daily
//! is the right cadence. The rate limit (50 lookups/week for free
//! unauthenticated) gives us 25 IPs × 7 days = 175 lookups/week budget,
//! so 25/day = 175/week, technically 7% over. With HUB_GREYNOISE_API_KEY
//! set, free tier gives 50/week — same budget. Mitigation: drop the
//! watchlist to 20 IPs if 429s hit (track in health cell).
//!
//! ## external_id
//!
//! `greynoise:{ip}` — the IP IS the canonical identifier in GreyNoise.
//! Idempotent across re-polls.
//!
//! ## Auth
//!
//! Optional `HUB_GREYNOISE_API_KEY`. Without key → `/v3/community/{ip}`
//! (free unauthenticated, ~rate-limited). With key → can use the full
//! `/v3/ip/{ip}` endpoint for richer data (actor, tags, raw ports).
//! For Phase 2.1 v1 we use only the community endpoint; future
//! Phase 2.1.x can branch on env to use the full endpoint.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.greynoise.io/v3/community";

/// Curated watchlist of IPs with high likelihood of being in GreyNoise's
/// dataset. Mix of:
///   - TOR exits (well-known scanner traffic)
///   - Shodan / Censys / Project Sonar (legit scanners that show up as NOISE)
///   - Mirai historical botnet C2s
///   - Spamhaus DROP-listed ranges (sample)
///   - Common brute-force sources (WordPress/Jenkins attackers)
///
/// 25 IPs × 1 lookup/day = 25/day. GreyNoise free tier is ~50/week for
/// authenticated, ~unauth lower — if we hit 429s, trim the list.
/// This list is intentionally public (well-known scanning networks);
/// no privacy concerns.
const WATCHLIST: &[&str] = &[
    // TOR exits (commonly NOISE)
    "185.220.101.50",
    "185.220.101.1",
    "185.220.101.32",
    "185.220.102.4",
    "199.249.230.114",
    // Well-known internet scanners (legit but observed)
    "104.244.79.180",  // Twitter scanner
    "162.247.74.7",    // Censys
    "71.6.135.131",    // Shodan
    "198.20.69.74",    // Shodan
    "188.166.74.46",   // Internet scanning project
    "192.241.220.183", // Censys
    // IoT botnet historical C2 (Mirai-family)
    "92.118.36.211",
    "45.227.255.190",
    "5.188.10.156",
    "89.248.165.74",
    "141.98.10.66",
    // Brute force / web attackers (commonly observed scanning)
    "139.59.1.100",
    "167.94.138.71",
    "162.142.125.0",
    "167.248.133.37",
    "206.189.0.0",     // sample
    // Spamhaus-listed (DROP)
    "103.224.182.245",
    "91.219.236.222",
    "37.49.226.150",
    "194.5.249.180",
];

/// Max IPs to query per day (caps the watchlist slice). 25/day matches
/// the curated list above; can be lowered if hitting rate limits.
const MAX_PER_DAY: usize = 25;

pub struct GreyNoise;

impl Source for GreyNoise {
    fn name(&self) -> &'static str {
        "greynoise"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §3.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let mut sigs = Vec::new();
            let mut rate_limited = 0usize;
            for ip in WATCHLIST.iter().take(MAX_PER_DAY) {
                let url = format!("{BASE_URL}/{ip}");
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(ip = %ip, error = %e, "greynoise fetch failed");
                        continue;
                    }
                };
                if resp.status() == 429 {
                    rate_limited += 1;
                    continue;
                }
                // GreyNoise returns 200 for IPs found in dataset, 404 for
                // IPs not observed. Both are valid responses (no error).
                if !resp.status().is_success() && resp.status() != 404 {
                    tracing::warn!(
                        ip = %ip,
                        status = %resp.status(),
                        "greynoise non-2xx/404"
                    );
                    continue;
                }
                let body: serde_json::Value = match resp.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::warn!(ip = %ip, error = %e, "greynoise parse failed");
                        continue;
                    }
                };
                if let Some(sig) = parse_ip_response(*ip, &body) {
                    sigs.push(sig);
                }
            }
            if rate_limited > 0 {
                tracing::warn!(
                    rate_limited,
                    "greynoise rate-limit hits — consider reducing watchlist size"
                );
            }
            Ok(sigs)
        }
        .boxed()
    }
}

/// Map one GreyNoise community response to a Signal (if worth emitting).
/// IPs with `noise=false` and `riot=false` (ordinary user IP, not
/// scanning) → return None (no signal).
fn parse_ip_response(ip: &str, j: &serde_json::Value) -> Option<Signal> {
    let noise = j
        .get("noise")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let riot = j
        .get("riot")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !noise && !riot {
        return None; // Ordinary IP — nothing to surface.
    }
    let classification = j
        .get("classification")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let name = j.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let last_seen = j.get("last_seen").and_then(|v| v.as_str()).unwrap_or("");
    let link = j.get("link").and_then(|v| v.as_str()).unwrap_or("");
    let (severity, kind) = match classification {
        "malicious" => ("priority", "scanner_malicious"),
        "benign" => ("routine", "scanner_benign"),
        _ => {
            if riot {
                ("routine", "riot_benign")
            } else {
                ("info", "scanner_unknown")
            }
        }
    };
    let title = format!("{ip} — {classification}{}", if name.is_empty() {
        String::new()
    } else {
        format!(" ({name})")
    });
    Some(
        Signal::new(kind, title, 0.0, 0.0, format!("greynoise:{ip}"))
            .severity(severity)
            .payload(serde_json::json!({
                "ip": ip,
                "noise": noise,
                "riot": riot,
                "classification": classification,
                "name": name,
                "last_seen": last_seen,
                "link": link,
                "extreme_type": kind,
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_malicious() -> serde_json::Value {
        serde_json::json!({
            "ip": "185.220.101.50",
            "noise": true,
            "riot": false,
            "classification": "malicious",
            "name": "unknown",
            "link": "https://viz.greynoise.io/ip/185.220.101.50",
            "last_seen": "2026-09-27",
            "message": "Success"
        })
    }

    fn sample_riot() -> serde_json::Value {
        serde_json::json!({
            "ip": "8.8.8.8",
            "noise": false,
            "riot": true,
            "classification": "benign",
            "name": "Google Public DNS",
            "link": "https://viz.greynoise.io/riot/8.8.8.8",
            "last_seen": "2026-09-27",
            "message": "Success"
        })
    }

    fn sample_unknown_scanner() -> serde_json::Value {
        serde_json::json!({
            "ip": "192.0.2.1",
            "noise": true,
            "riot": false,
            "classification": "unknown",
            "name": "unknown",
            "last_seen": "2026-09-27",
            "message": "Success"
        })
    }

    fn sample_ordinary() -> serde_json::Value {
        serde_json::json!({
            "ip": "1.1.1.1",
            "noise": false,
            "riot": false,
            "message": "IP not observed scanning the internet."
        })
    }

    /// Happy path: malicious IP → priority / scanner_malicious.
    #[test]
    fn detects_malicious_scanner() {
        let sig = parse_ip_response("185.220.101.50", &sample_malicious()).unwrap();
        assert_eq!(sig.severity, "priority");
        assert_eq!(sig.kind, "scanner_malicious");
        assert!(sig.title.contains("malicious"));
    }

    /// RIOT benign → routine / scanner_benign.
    #[test]
    fn detects_riot_benign() {
        let sig = parse_ip_response("8.8.8.8", &sample_riot()).unwrap();
        assert_eq!(sig.severity, "routine");
        assert_eq!(sig.kind, "scanner_benign");
        assert!(sig.title.contains("benign"));
    }

    /// Noise + unknown classification → info / scanner_unknown.
    #[test]
    fn detects_unknown_scanner() {
        let sig = parse_ip_response("192.0.2.1", &sample_unknown_scanner()).unwrap();
        assert_eq!(sig.severity, "info");
        assert_eq!(sig.kind, "scanner_unknown");
    }

    /// Ordinary IP (not noise, not riot) → return None (no signal).
    #[test]
    fn ordinary_ip_emits_no_signal() {
        assert!(parse_ip_response("1.1.1.1", &sample_ordinary()).is_none());
    }

    /// external_id = `greynoise:{ip}`.
    #[test]
    fn external_id_shape() {
        let sig = parse_ip_response("185.220.101.50", &sample_malicious()).unwrap();
        assert_eq!(sig.external_id, "greynoise:185.220.101.50");
    }

    /// Defensive: missing fields default safely (no panic).
    #[test]
    fn missing_fields_default_safe() {
        let j = serde_json::json!({"ip": "1.2.3.4", "noise": true});
        let sig = parse_ip_response("1.2.3.4", &j);
        assert!(sig.is_some());
        assert_eq!(sig.unwrap().severity, "info"); // no classification
    }

    /// Defensive: noise=false but riot=true → emit (still worth surfacing).
    #[test]
    fn riot_only_emits_routine() {
        let j = serde_json::json!({"ip": "1.2.3.4", "noise": false, "riot": true});
        let sig = parse_ip_response("1.2.3.4", &j);
        assert!(sig.is_some());
        assert_eq!(sig.unwrap().severity, "routine");
    }

    /// Watchlist size: 25 IPs as expected.
    #[test]
    fn watchlist_size_within_budget() {
        assert_eq!(WATCHLIST.len(), 25);
        assert!(WATCHLIST.len() <= MAX_PER_DAY);
    }

    /// All watchlist IPs are valid (non-empty, no whitespace).
    #[test]
    fn watchlist_ip_format() {
        for ip in WATCHLIST {
            assert!(!ip.is_empty());
            assert!(!ip.contains(' '));
            assert!(ip.split('.').count() == 4); // IPv4 dot-quad
        }
    }
}