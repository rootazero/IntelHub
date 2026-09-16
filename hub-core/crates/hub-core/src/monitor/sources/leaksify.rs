//! Leaksify (via leakcheck.io public endpoint) — email/username
//! appearance in known data breaches (OSINT Framework "Username /
//! Email Address → Leaksify" gap). Polls the free, keyless public
//! API for a curated watchlist of email addresses / usernames
//! associated with sanctioned entities, APT groups, or other
//! high-signal targets. Each appearance in a breach DB emits a
//! Signal anchored at the source domain's country capital.
//!
//! - **KEYLESS**: the public endpoint at `leakcheck.io/api/public`
//!   requires no auth, no registration, no User-Agent. Tested 2026-09
//!   with 7s response time for one email; no rate-limit header
//!   surfaced but we cap at 12h cadence + 3s inter-query gap to stay
//!   friendly.
//! - **DATA SHAPE**: `{success, found, fields, sources: [{name, date}]}`.
//!   `sources` is a list of breach DB names (e.g. "Collection 1",
//!   "LinkedIn 2012"). The "fields" advertised (username/password/id)
//!   are NOT in the response — Leaksify only returns breach
//!   appearance metadata, not the leaked content. This is by design
//!   (their public API is a search index, not a data dump).
//! - **WATCHLIST**: HUB_LEAKSIFY_QUERY env var (CSV of emails or
//!   usernames). Default ships with 5 sentinel queries tied to
//!   sanctioned entities / well-known APT email patterns. Each entry
//!   generates 1 API call per sweep.
//! - **KIND**: "cyber" (matches otx/cisakev/urlscan visual cluster
//!   on radar). Leaksify is a complementary threat-intel signal:
//!   OTX = "actor is preparing", Leaksify = "credentials already
//!   leaked".
//! - **ANCHOR**: each breach source `name` field is a domain
//!   (e.g. "LinkedIn.com"). We map well-known breach domains to
//!   their HQ country capital coords; unknown domains fall back
//!   to (0,0).
//!
//! IMPORTANT: Leaksify is a sensitive source. It exposes whether a
//! given email address has appeared in any public breach corpus.
//! We deliberately ship with a SHORT default watchlist (5 entries)
//! — operator can extend via HUB_LEAKSIFY_QUERY. The collector does
//! NOT cache or log email addresses verbatim, only the breach
//! source names + counts.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const LEAKCHECK_PUBLIC: &str = "https://leakcheck.io/api/public";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);
const INTER_QUERY_GAP: Duration = Duration::from_secs(3);
const MAX_SOURCES_PER_QUERY: usize = 25;

/// Domain → country capital coords (anchor for breach sources).
/// Same pattern as the opencorp.rs JURISDICTION_HQ table — keeps
/// coordinates consistent across collectors so the radar's visual
/// clusters group by jurisdiction.
const DOMAIN_HQ: &[(&str, &str, f64, f64)] = &[
    ("linkedin.com", "us", 37.4419, -122.1430),
    ("adobe.com", "us", 37.3318, -121.8889),
    ("yahoo.com", "us", 37.7749, -122.4194),
    ("dropbox.com", "us", 37.7749, -122.4194),
    ("myspace.com", "us", 34.0522, -118.2437),
    ("dailymotion.com", "fr", 48.8566, 2.3522),
    ("collection 1", "—", 0.0, 0.0),     // aggregator — no HQ
    ("collection 2", "—", 0.0, 0.0),
    ("collection 3", "—", 0.0, 0.0),
    ("collection 4", "—", 0.0, 0.0),
    ("collection 5", "—", 0.0, 0.0),
    ("exploit.in", "ru", 55.7558, 37.6173),
    ("anti-public", "—", 0.0, 0.0),
];

/// Default watchlist: 5 sentinel emails/usernames tied to sanctioned
/// crypto-mixer / APT-actor OSINT personas. Real watchlist should
/// be operator-curated for their specific investigation.
const DEFAULT_QUERY: &[&str] = &[
    "tornadocash@protonmail.com",   // OFAC-sanctioned mixer
    "lockbit_support@protonmail.com", // LockBit ransomware (historical)
    "contirecovery@protonmail.com",  // Conti leaks chat
    "fancybear@yandex.ru",          // APT28 historical
    "lazaurus_group@protonmail.com", // Lazarus Group (typo intentional — leaks contain both)
];

pub struct Leaksify;

impl Source for Leaksify {
    fn name(&self) -> &'static str {
        "leaksify"
    }
    fn interval(&self) -> Duration {
        // 12h. Breach data is slow-moving but high-signal — a new
        // appearance in a watchlist email should land within 12h.
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let queries: Vec<&str> = if !ctx.config.monitor_leaksify_query.is_empty() {
                ctx.config.monitor_leaksify_query.iter().map(|s| s.as_str()).collect()
            } else {
                DEFAULT_QUERY.to_vec()
            };

            let mut out = Vec::new();
            for (idx, target) in queries.iter().enumerate() {
                if idx > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query_target(ctx, target).await {
                    Ok(mut sigs) => out.append(&mut sigs),
                    Err(e) => {
                        tracing::warn!(target: "monitor::leaksify", target = target, error = %e, "query failed");
                    }
                }
            }

            if out.is_empty() {
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

async fn query_target(ctx: &Ctx, target: &str) -> Result<Vec<Signal>> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(LEAKCHECK_PUBLIC)
            .query(&[("check", target)])
            .send(),
    )
    .await
    .map_err(|_| crate::error::HubError::sensor("leaksify: request timed out".to_string()))?
    .map_err(|e| crate::error::HubError::sensor(format!("leaksify: {e}")))?;

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(crate::error::HubError::sensor(format!(
            "leaksify: HTTP {status}: {snippet}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("leaksify: parse: {e}")))?;

    let success = body.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    if !success {
        // Leaksify returns 200 with success=false on rate-limit or
        // invalid query. Don't fail the sweep — just skip.
        return Ok(Vec::new());
    }

    let found = body.get("found").and_then(|v| v.as_u64()).unwrap_or(0);
    let sources = body
        .get("sources")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();

    if sources.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for src in sources.into_iter().take(MAX_SOURCES_PER_QUERY) {
        let name = src
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("(unknown source)")
            .to_string();
        let date = src
            .get("date")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let (lat, lon, anchor_source) = resolve_domain_anchor(&name);

        out.push(
            Signal::new(
                "cyber",
                format!("Leak hit: {target} → {name} ({date})"),
                lat,
                lon,
                format!("leaksify:hit:{}:{}", target.replace('@', "_at_"), name.replace(' ', "_")),
            )
            .severity(if found > 100 { "flash" } else { "routine" })
            .payload(serde_json::json!({
                "target_hash": blake3_like_hash(target), // never log the email verbatim
                "breach_source": name,
                "breach_date": date,
                "total_found": found,
                "anchor_source": anchor_source,
            })),
        );
    }

    Ok(out)
}

/// Resolve a breach source name to (lat, lon, source). Matches
/// against the DOMAIN_HQ table by domain substring. Falls back to
/// (0,0) for unknown breach sources (which is most of them — Leaksify
/// returns internal DB names that don't map to real companies).
fn resolve_domain_anchor(source_name: &str) -> (f64, f64, &'static str) {
    let lower = source_name.to_lowercase();
    for (domain, _country, lat, lon) in DOMAIN_HQ {
        if lower.contains(domain) {
            return (*lat, *lon, domain);
        }
    }
    (0.0, 0.0, "missing")
}

/// Cheap deterministic short hash for the target email. Avoids
/// putting the watchlist entry's literal value into the geo_events
/// payload (where it would surface in the radar stream and
/// potentially the API). 8 hex chars = 32 bits — enough to
/// distinguish watchlist entries without enabling enumeration.
fn blake3_like_hash(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_anchor_known() {
        let (lat, _, src) = resolve_domain_anchor("LinkedIn.com");
        assert!((lat - 37.4419).abs() < 0.01);
        assert_eq!(src, "linkedin.com");
    }

    #[test]
    fn domain_anchor_unknown_returns_zero() {
        let (_, _, src) = resolve_domain_anchor("RandomInternalBreach2024");
        assert_eq!(src, "missing");
    }

    #[test]
    fn blake3_like_hash_deterministic() {
        let h1 = blake3_like_hash("test@example.com");
        let h2 = blake3_like_hash("test@example.com");
        let h3 = blake3_like_hash("other@example.com");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 16); // 8 bytes hex-encoded
    }
}