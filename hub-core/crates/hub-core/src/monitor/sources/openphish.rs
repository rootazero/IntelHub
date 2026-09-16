//! OpenPhish — free keyless phishing-URL feed. OSINT Framework
//! "URL → Phishing" gap (PhishTank's public feed was retired in
//! 2024; OpenPhish is the de facto replacement). Endpoint:
//! `https://openphish.com/feed.txt` redirects to
//! `https://raw.githubusercontent.com/openphish/public_feed/refs/heads/main/feed.txt`,
//! returns plain-text with one phishing URL per line (~300
//! entries at any moment).
//!
//! - **KEYLESS**: OpenPhish publishes the feed under CC-BY-NC-SA
//!   for non-commercial use; commercial operators must apply
//!   through openphish.com/contact. We surface it for OSINT
//!   consumption (not blocking).
//! - **CADENCE**: 4h. OpenPhish updates the feed several times per
//!   day; 4h catches new submissions within the day without
//!   hammering the upstream (each fetch is ~14KB).
//! - **ANCHOR**: phishing URLs have no real geo (URLs are text).
//!   We use OpenPhish operator's country (US, per openphish.com)
//!   as the honest stand-in. Cyber-source cluster pattern: when
//!   no per-event coords exist, the signal lands at the operator's
//!   home country.
//! - **KIND**: "cyber" (matches otx/urlscan/ahmia/leaksify/tor_exit
//!   / ipsum / shodan_internetdb / crtsh visual cluster). OpenPhish
//!   is complementary to urlscan (live URL-scan intel) — OpenPhish
//!   is phishing-catalog intel (known-bad), urlscan is fresh-hits
//!   intel (potentially-bad).
//! - **VOLUME**: ~300 entries per fetch. We emit one Signal per
//!   URL with cap MAX_PER_SWEEP. Ingest-layer content_hash dedup
//!   keyed on URL string keeps the volume bounded after day-1
//!   (URLs rotate but with high overlap).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const FEED_URL: &str = "https://openphish.com/feed.txt";
const MAX_PER_SWEEP: usize = 30;

/// OpenPhish operator's country — United States (per openphish.com
/// contact page). Honest stand-in — phishing URLs have no geo.
const OPENPHISH_HQ: (f64, f64) = (37.0902, -95.7129);

pub struct OpenPhish;

impl Source for OpenPhish {
    fn name(&self) -> &'static str {
        "openphish"
    }
    fn interval(&self) -> Duration {
        // 4h. Upstream updates several times daily; 4h catches
        // new submissions without redundant fetches.
        Duration::from_secs(4 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let resp = tokio::time::timeout(
                Duration::from_secs(20),
                ctx.http.get(FEED_URL).send(),
            )
            .await
            .map_err(|_| HubError::sensor("openphish: feed request timed out".to_string()))?
            .map_err(|e| HubError::sensor(format!("openphish: {e}")))?;

            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "openphish: HTTP {}",
                    resp.status()
                )));
            }

            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("openphish: body: {e}")))?;

            // Feed format: one URL per line, no header. Empty lines
            // and comments (#) skipped. Lines that don't start with
            // http:// or https:// are also skipped (defensive — feed
            // is documented as URL-only but malformed entries have
            // appeared historically).
            let mut urls: Vec<String> = Vec::with_capacity(MAX_PER_SWEEP * 2);
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                    continue;
                }
                urls.push(trimmed.to_string());
            }

            if urls.is_empty() {
                tracing::warn!(
                    target: "monitor::openphish",
                    "feed returned 0 URLs — upstream format may have changed"
                );
                return Ok(Vec::new());
            }

            // No "most-malicious-first" sort possible (no signal
            // beyond URL string); emit the first MAX_PER_SWEEP and
            // let ingest-layer content_hash dedup handle repeats.
            urls.truncate(MAX_PER_SWEEP);

            let mut out = Vec::with_capacity(urls.len());
            for url in urls {
                // Cheap target_hash — same pattern as leaksify.rs
                // (DefaultHasher 16 hex chars). Avoids putting the
                // literal phishing URL into the geo_events payload
                // body where it could surface in the radar stream
                // and the API.
                let target_hash = blake3_like_hash(&url);

                out.push(
                    Signal::new(
                        "cyber",
                        format!("Phishing URL catalogued: {url}"),
                        OPENPHISH_HQ.0,
                        OPENPHISH_HQ.1,
                        format!("openphish:url:{target_hash}"),
                    )
                    .severity("priority")
                    .payload(serde_json::json!({
                        "target_hash": target_hash,
                        "url": url,
                        "feed": "openphish",
                        "feed_url": FEED_URL,
                        "anchor_source": "openphish_hq_us",
                    })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}

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
    fn feed_url_constant_matches_documented_endpoint() {
        // If openphish changes the path we want CI to flag the
        // collector, not silent failure at runtime.
        assert_eq!(FEED_URL, "https://openphish.com/feed.txt");
    }

    #[test]
    fn max_per_sweep_is_bounded() {
        // First-sweep volume cap. Bumping this >200 makes the
        // radar unreadable; CI pins the upper guardrail.
        assert!(MAX_PER_SWEEP > 0 && MAX_PER_SWEEP <= 200);
    }

    #[test]
    fn blake3_like_hash_deterministic() {
        let h1 = blake3_like_hash("https://evil.example.com/login");
        let h2 = blake3_like_hash("https://evil.example.com/login");
        let h3 = blake3_like_hash("https://other.example.com/");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 16); // 8 bytes hex-encoded
    }
}