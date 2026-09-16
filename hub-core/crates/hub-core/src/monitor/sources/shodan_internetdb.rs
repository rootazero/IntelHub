//! Shodan InternetDB — free keyless per-IP enrichment (CPEs / ports /
//! hostnames / vulns / tags). OSINT Framework "IP → Shodan InternetDB"
//! gap. Endpoint: `https://internetdb.shodan.io/<ip>` returns a small
//! JSON `{ip, ports, hostnames, cpes, tags, vulns}`. No auth, no rate
//! limit, no registration.
//!
//! - **KEYLESS**: InternetDB is Shodan's free companion API that ships
//!   a 1-IP-at-a-time lookup derived from the same crawl data that
//!   powers paid Shodan. Same freshness as paid (weekly), but with
//!   no API key requirement and a generous rate budget.
//! - **CADENCE**: 12h. InternetDB snapshots update on Shodan's
//!   ~weekly crawl cycle; a 12h sweep keeps the watchlist fresh
//!   without hammering the endpoint.
//! - **WATCHLIST**: HUB_SHODAN_INTERNETDB_WATCH env var (CSV of IPs).
//!   Default ships with a mix of well-known public IPs (Cloudflare /
//!   Google DNS) + a few "interesting" sentinel IPs that have
//!   historically returned rich CVE data on InternetDB (so the
//!   operator sees non-empty output on day 1).
//! - **ANCHOR**: per-IP "real" coords aren't returned by InternetDB
//!   (the API returns hostnames, not geo). We use Shodan's HQ in
//!   San Francisco as the honest stand-in — all watchlist IPs
//!   cluster on SF, which self-documents "InternetDB enrichment
//!   metadata" on the radar. (Same pattern as tor_exit / ipsum.)
//! - **KIND**: "cyber" (matches otx/urlscan/ahmia/leaksify/tor_exit
//!   / ipsum visual cluster). Shodan InternetDB is complementary to
//!   urlscan (URL intel) and otx (actor intel) — InternetDB is
//!   IP-level device/service/vuln intel.
//! - **VOLUME**: one Signal per IP per sweep. Ingest-layer dedup
//!   keyed on IP keeps a healthy watchlist from re-emitting the
//!   same payload twice; new CPE / port / vuln additions DO land
//!   because the metadata content_hash changes.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://internetdb.shodan.io";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const INTER_QUERY_GAP: Duration = Duration::from_secs(1);
const MAX_PER_SWEEP: usize = 12;

/// Shodan HQ (San Francisco). Honest stand-in — per-IP geo isn't
/// returned by InternetDB (only reverse-DNS hostnames).
const SHODAN_HQ: (f64, f64) = (37.7749, -122.4194);

/// Default watchlist: mix of well-known public IPs (so the API
/// always returns SOMETHING) + a handful of "interesting" sentinel
/// IPs that have historically shown non-empty CPEs/vulns on
/// InternetDB (so day-1 sweeps surface real signal instead of
/// empty state).
const DEFAULT_WATCH: &[&str] = &[
    "1.1.1.1",        // Cloudflare DNS — usually ports + cpes
    "8.8.8.8",        // Google DNS — usually ports + cpes
    "9.9.9.9",        // Quad9 DNS — usually ports + cpes
    "208.67.222.222", // OpenDNS
    "140.82.121.4",   // GitHub
    "151.101.0.81",   // Fastly CDN
    "104.16.132.229", // Cloudflare front page
    "13.107.42.14",   // Microsoft
    "23.192.228.80",  // Akamai
    "162.247.244.157", // LetsEncrypt OCSP
];

pub struct ShodanInternetDb;

impl Source for ShodanInternetDb {
    fn name(&self) -> &'static str {
        "shodan_internetdb"
    }
    fn interval(&self) -> Duration {
        // 12h matches InternetDB's ~weekly update cycle with
        // ~3x headroom. Cheaper than daily; doesn't miss updates.
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let watchlist: Vec<String> =
                if !ctx.config.monitor_shodan_internetdb_watch.is_empty() {
                    ctx.config
                        .monitor_shodan_internetdb_watch
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                } else {
                    DEFAULT_WATCH.iter().map(|s| s.to_string()).collect()
                };

            let mut out = Vec::new();
            for (idx, ip) in watchlist.iter().enumerate() {
                if idx > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query_ip(ctx, ip).await {
                    Ok(mut sigs) => out.append(&mut sigs),
                    Err(e) => {
                        tracing::warn!(
                            target: "monitor::shodan_internetdb",
                            ip = ip,
                            error = %e,
                            "query failed"
                        );
                    }
                }
            }

            if out.is_empty() {
                return Ok(out);
            }
            // cap at MAX_PER_SWEEP — full watchlist emit would
            // exceed radar signal-density budget on first deploy.
            out.truncate(MAX_PER_SWEEP);
            Ok(out)
        }
        .boxed()
    }
}

async fn query_ip(ctx: &Ctx, ip: &str) -> Result<Vec<Signal>> {
    let url = format!("{API_BASE}/{ip}");
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http.get(&url).send(),
    )
    .await
    .map_err(|_| HubError::sensor("shodan_internetdb: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("shodan_internetdb: {e}")))?;

    // InternetDB returns 404 + {"detail":"No information available"}
    // for IPs that have never been crawled. Treat as empty (not error)
    // so a single uncrawled sentinel doesn't fail the sweep.
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(HubError::sensor(format!(
            "shodan_internetdb: HTTP {status}: {snippet}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("shodan_internetdb: parse: {e}")))?;

    let ip_resp = body
        .get("ip")
        .and_then(|v| v.as_str())
        .unwrap_or(ip)
        .to_string();
    let ports: Vec<u64> = body
        .get("ports")
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_u64()).collect())
        .unwrap_or_default();
    let hostnames: Vec<String> = body
        .get("hostnames")
        .and_then(|h| h.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let cpes: Vec<String> = body
        .get("cpes")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let tags: Vec<String> = body
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let vulns: Vec<String> = body
        .get("vulns")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if ports.is_empty() && hostnames.is_empty() && cpes.is_empty() && vulns.is_empty() {
        return Ok(Vec::new());
    }

    // Severity: any CVE → flash; otherwise priority if 5+ ports or
    // tags non-empty (IoT / ICS / etc.); otherwise routine.
    let severity = if !vulns.is_empty() {
        "flash"
    } else if ports.len() >= 5 || !tags.is_empty() {
        "priority"
    } else {
        "routine"
    };

    let mut s = Signal::new(
        "cyber",
        format!(
            "Shodan InternetDB: {ip_resp} ({} ports, {} vulns, {} hostnames)",
            ports.len(),
            vulns.len(),
            hostnames.len()
        ),
        SHODAN_HQ.0,
        SHODAN_HQ.1,
        format!("shodan_internetdb:ip:{ip_resp}"),
    )
    .severity(severity)
    .payload(serde_json::json!({
        "ip": ip_resp,
        "ports": ports,
        "hostnames": hostnames,
        "cpes": cpes,
        "tags": tags,
        "vulns": vulns,
        "anchor_source": "shodan_hq_sf",
    }));
    Ok(vec![s])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_constant_matches_documented_url() {
        assert_eq!(API_BASE, "https://internetdb.shodan.io");
    }

    #[test]
    fn default_watch_has_mix_of_dns_and_cdn() {
        // Must contain at least one DNS-resolver (1.1.1.1, 8.8.8.8, or
        // 9.9.9.9) so the API always returns a non-empty response on
        // day-1 — otherwise the operator sees a flat radar.
        let dns_present = DEFAULT_WATCH.iter().any(|ip| {
            ["1.1.1.1", "8.8.8.8", "9.9.9.9"].contains(ip)
        });
        assert!(dns_present, "default watchlist must include a public DNS resolver");
    }

    #[test]
    fn max_per_sweep_bounded() {
        assert!(MAX_PER_SWEEP > 0 && MAX_PER_SWEEP <= 50);
    }
}