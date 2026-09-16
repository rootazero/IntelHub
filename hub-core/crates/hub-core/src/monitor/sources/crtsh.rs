//! crt.sh — Certificate Transparency (CT) log search. Free keyless
//! lookup against every publicly observable TLS certificate. OSINT
//! Framework "Domain → Certificate Search" gap. Endpoint:
//! `https://crt.sh/?q=<domain>&output=json&limit=N` returns JSON
//! array of certificate records `{issuer_ca_id, issuer_name,
//! common_name, name_value, id, not_before, not_after, ...}`.
//!
//! - **KEYLESS**: crt.sh is a free CT log aggregator run by
//!   Sectigo. No auth, no rate limit on a per-request basis;
//!   response time is ~3-10s for narrow queries and can climb to
//!   30s+ for broad wildcards.
//! - **CADENCE**: 6h. Certs are issued on demand (new subdomains
//!   get a new cert almost immediately); a 6h sweep catches the
//!   majority of meaningful domain-fronting / typosquat signals
//!   without hammering the upstream.
//! - **WATCHLIST**: HUB_CRTSH_WATCH env var (CSV of domain patterns
//!   per crt.sh query syntax — supports exact match, wildcard
//!   `%`, and issuer filter). Default ships with 5 high-traffic
//!   domains across critical infrastructure so day-1 emits real
//!   signals.
//! - **ANCHOR**: per-cert "real" coords don't exist. We anchor at
//!   Let's Encrypt HQ (San Francisco) as the honest stand-in —
//!   Let's Encrypt issues the majority of publicly observable
//!   CT-logged certs, so anchoring there is a defensible
//!   metadata stand-in for "this cert came from the LE trust
//!   anchor".
//! - **KIND**: "cyber" (matches otx/urlscan/ahmia/leaksify/tor_exit
//!   / ipsum / shodan_internetdb visual cluster). crt.sh is
//!   complementary to urlscan (URL intel) and shodan_internetdb
//!   (IP intel) — crt.sh is TLS-cert-issuance intel that surfaces
//!   typosquats and shadow-IT subdomains.
//! - **VOLUME**: each sweep can return hundreds of certs per
//!   domain; we cap at MAX_PER_DOMAIN × domains_per_sweep. New
//!   certs land (different `id` ⇒ different content_hash) while
//!   repeats are dedup'd at ingest layer.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://crt.sh";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(35);
const INTER_QUERY_GAP: Duration = Duration::from_secs(3);
const MAX_PER_DOMAIN: usize = 20;

/// Let's Encrypt HQ (San Francisco). Honest stand-in for "this
/// cert came from the dominant CT-trust anchor" — majority of
/// crt.sh records are LE-issued. Use San Francisco (37.7749, -122.4194)
/// keeps the radar cluster visually compact.
const LETSENCRYPT_HQ: (f64, f64) = (37.7749, -122.4194);

/// Default watchlist: 5 high-traffic domains across critical
/// infrastructure. Per crt.sh query syntax:
/// - bare domain → exact + subdomain match
/// - `%domain`   → wildcard subdomain match (subdomain-only)
/// - `%.domain`  → wildcard subdomain + multi-level match
const DEFAULT_WATCH: &[&str] = &[
    "google.com",
    "microsoft.com",
    "amazon.com",
    "github.com",
    "gov.uk",
];

pub struct CrtSh;

impl Source for CrtSh {
    fn name(&self) -> &'static str {
        "crtsh"
    }
    fn interval(&self) -> Duration {
        // 6h. CT log inclusion is near-real-time; 6h sweep is the
        // smallest cadence that catches typosquat issuance within
        // a working day.
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let watchlist: Vec<String> = if !ctx.config.monitor_crtsh_watch.is_empty() {
                ctx.config
                    .monitor_crtsh_watch
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                DEFAULT_WATCH.iter().map(|s| s.to_string()).collect()
            };

            let mut out = Vec::new();
            for (idx, domain) in watchlist.iter().enumerate() {
                if idx > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query_domain(ctx, domain).await {
                    Ok(mut sigs) => out.append(&mut sigs),
                    Err(e) => {
                        tracing::warn!(
                            target: "monitor::crtsh",
                            domain = domain,
                            error = %e,
                            "query failed"
                        );
                    }
                }
            }

            if out.is_empty() {
                return Ok(out);
            }
            Ok(out)
        }
        .boxed()
    }
}

async fn query_domain(ctx: &Ctx, domain: &str) -> Result<Vec<Signal>> {
    let url = format!(
        "{API_BASE}/?q={}&output=json&limit={}",
        urlencode(domain),
        MAX_PER_DOMAIN
    );
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http.get(&url).send(),
    )
    .await
    .map_err(|_| HubError::sensor("crtsh: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("crtsh: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(HubError::sensor(format!(
            "crtsh: HTTP {status}: {snippet}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("crtsh: parse: {e}")))?;

    let entries = body
        .as_array()
        .cloned()
        .unwrap_or_default();

    if entries.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in entries.into_iter().take(MAX_PER_DOMAIN) {
        let common_name = entry
            .get("common_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if common_name.is_empty() {
            continue;
        }
        let issuer_name = entry
            .get("issuer_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cert_id = entry
            .get("id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let not_before = entry
            .get("not_before")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let not_after = entry
            .get("not_after")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Severity: LE-issued wildcard → "priority"; short lifetime
        // (<30 days, common for typosquats and ephemeral C2) →
        // "flash"; otherwise routine.
        let is_wildcard = common_name.starts_with("*.");
        let is_short = not_before.len() >= 10
            && not_after.len() >= 10
            && days_between(&not_before, &not_after) < 30;
        let severity = if is_short {
            "flash"
        } else if is_wildcard {
            "priority"
        } else {
            "routine"
        };

        out.push(
            Signal::new(
                "cyber",
                format!(
                    "Cert observed: {common_name} ({}{} issuer={})",
                    if is_wildcard { "wildcard " } else { "" },
                    severity,
                    truncate_issuer(&issuer_name),
                ),
                LETSENCRYPT_HQ.0,
                LETSENCRYPT_HQ.1,
                format!("crtsh:cert:{cert_id}"),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "common_name": common_name,
                "cert_id": cert_id,
                "issuer_name": issuer_name,
                "not_before": not_before,
                "not_after": not_after,
                "is_wildcard": is_wildcard,
                "is_short_lived": is_short,
                "query_domain": domain,
                "anchor_source": "letsencrypt_hq_sf",
            })),
        );
    }

    Ok(out)
}

fn urlencode(s: &str) -> String {
    // Minimal RFC 3986 percent-encoding for URL query values
    // (crt.sh uses standard URL encoding). Avoid adding a new
    // dependency just for one collector.
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

fn truncate_issuer(s: &str) -> String {
    // Strip the verbose C=...,O=...,CN=... issuer prefix and keep
    // only the CN. Display-friendly in the geo_events text.
    s.split(", CN=")
        .nth(1)
        .map(|cn| {
            let end = cn.find(',').unwrap_or(cn.len());
            cn[..end].to_string()
        })
        .unwrap_or_else(|| s.to_string())
}

fn days_between(not_before: &str, not_after: &str) -> i64 {
    // ISO 8601 like "2026-09-16T11:17:53" — compute day diff without
    // pulling in chrono. Naive parser is fine for our 30-day check.
    fn parse(s: &str) -> Option<i64> {
        let y: i64 = s.get(0..4)?.parse().ok()?;
        let m: i64 = s.get(5..7)?.parse().ok()?;
        let d: i64 = s.get(8..10)?.parse().ok()?;
        Some(y * 365 + m * 30 + d)
    }
    match (parse(not_before), parse(not_after)) {
        (Some(a), Some(b)) => b - a,
        _ => 999, // unknown → not short-lived
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_constant_matches_documented_url() {
        assert_eq!(API_BASE, "https://crt.sh");
    }

    #[test]
    fn urlencode_handles_wildcard_and_percent() {
        assert_eq!(urlencode("google.com"), "google.com");
        assert_eq!(urlencode("%25.evilcorp.com"), "%2525.evilcorp.com");
        assert_eq!(urlencode("a b"), "a%20b");
    }

    #[test]
    fn truncate_issuer_strips_cn() {
        assert_eq!(
            truncate_issuer("C=US, O=Let's Encrypt, CN=R3"),
            "R3"
        );
        assert_eq!(
            truncate_issuer("C=US, O=Google Inc, CN=Google Internet Authority G2"),
            "Google Internet Authority G2"
        );
    }

    #[test]
    fn days_between_basic() {
        assert!(days_between("2026-01-01T00:00:00", "2026-01-31T00:00:00") >= 25);
        assert!(days_between("2026-09-01T00:00:00", "2026-09-15T00:00:00") < 30);
    }
}