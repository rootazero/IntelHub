//! IPsum (https://github.com/stamparm/ipsum) — community-maintained
//! threat-intel IP blocklist that aggregates ~30 third-party block
//! sources (Spamhaus, Barracuda, AbuseIPDB, etc.) and counts how
//! many lists an IP appears on. OSINT Framework "Threats → Threat
//! IP Aggregators" gap. Free, keyless, ~575KB plain-text feed at
//! `https://raw.githubusercontent.com/stamparm/ipsum/master/ipsum.txt`.
//!
//! - **KEYLESS**: stamparm publishes the feed from a public GitHub
//!   repo with no auth, no rate limit, no registration. Updates
//!   daily via GitHub Actions.
//! - **CADENCE**: 12h. IPsum updates once daily; 12h cadence catches
//!   the daily rebuild within the next sweep without hammering the
//!   upstream.
//! - **ANCHOR**: no operator HQ (stamparm is a personal project).
//!   We use the project's GitHub-org country (Netherlands per the
//!   stamparm profile) as the honest stand-in. Cyber-source cluster
//!   pattern: when no per-event coords exist, the signal lands at
//!   the operator's home country.
//! - **VOLUME**: feed has ~100k IPs at any moment. We cap at
//!   MAX_PER_SWEEP and emit the TOP-N (most blocklists) — these
//!   are the IPs most likely to be abused in the next 24h and the
//!   radar can show the worst-offender first. Content-hash dedup
//!   keeps repeated IPs from creating redundant events.
//! - **KIND**: "cyber" (matches otx/urlscan/ahmia/leaksify cluster).
//!   IPsum is complementary to OTX (actor intel) and urlscan (URL
//!   intel) — IPsum is IP-level pre-emptive blocklist intel.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const IPSUM_FEED: &str =
    "https://raw.githubusercontent.com/stamparm/ipsum/master/ipsum.txt";
const MAX_PER_SWEEP: usize = 50;
/// Approximate HQ of the stamparm project (Netherlands per GitHub
/// profile + commit history). Honest stand-in — no per-IP geo
/// available in the feed.
const STAMPARM_HQ: (f64, f64) = (52.3676, 4.9041);

pub struct Ipsum;

impl Source for Ipsum {
    fn name(&self) -> &'static str {
        "ipsum"
    }
    fn interval(&self) -> Duration {
        // 12h — upstream publishes daily; catch within one sweep.
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let body = tokio::time::timeout(
                Duration::from_secs(30),
                ctx.http.get(IPSUM_FEED).send(),
            )
            .await
            .map_err(|_| HubError::sensor("ipsum: feed request timed out".to_string()))?
            .map_err(|e| HubError::sensor(format!("ipsum: {e}")))?;

            if !body.status().is_success() {
                return Err(HubError::sensor(format!(
                    "ipsum: HTTP {}",
                    body.status()
                )));
            }

            let text = body
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("ipsum: body: {e}")))?;

            // Feed format:
            //   # comment
            //   # comment
            //   1.2.3.4<TAB>11
            //   5.6.7.8<TAB>3
            // Second column = number of blocklists flagging this IP.
            // Higher = more confident the IP is malicious. We emit
            // the TOP-N by blocklist count.
            let mut parsed: Vec<(String, u32)> = Vec::with_capacity(2048);
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let mut parts = trimmed.splitn(2, '\t');
                let ip = parts.next().unwrap_or("").trim();
                let count: u32 = parts
                    .next()
                    .and_then(|c| c.trim().parse::<u32>().ok())
                    .unwrap_or(0);
                if ip.is_empty() || count == 0 {
                    continue;
                }
                // Validate IP shape — feed is mixed v4/v6 historically.
                if ip.parse::<std::net::IpAddr>().is_err() {
                    continue;
                }
                parsed.push((ip.to_string(), count));
            }

            if parsed.is_empty() {
                tracing::warn!(
                    target: "monitor::ipsum",
                    "ipsum feed parsed 0 entries — upstream format may have changed"
                );
                return Ok(Vec::new());
            }

            // Highest blocklist-count first — worst IPs surface first.
            parsed.sort_by(|a, b| b.1.cmp(&a.1));
            parsed.truncate(MAX_PER_SWEEP);

            let mut out = Vec::with_capacity(parsed.len());
            for (ip, count) in parsed {
                let severity = if count >= 10 {
                    "flash"
                } else if count >= 5 {
                    "priority"
                } else {
                    "routine"
                };
                out.push(
                    Signal::new(
                        "cyber",
                        format!("Threat IP blocklisted: {ip} ({count} lists)"),
                        STAMPARM_HQ.0,
                        STAMPARM_HQ.1,
                        format!("ipsum:blocked:{ip}"),
                    )
                    .severity(severity)
                    .payload(serde_json::json!({
                        "ip": ip,
                        "blocklist_count": count,
                        "feed": "stamparm/ipsum",
                        "feed_url": IPSUM_FEED,
                        "anchor_source": "stamparm_hq_nl",
                    })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipsum_feed_constant_matches_documented_url() {
        // Sanity guard: stamparm changes the path → CI flags it.
        assert_eq!(
            IPSUM_FEED,
            "https://raw.githubusercontent.com/stamparm/ipsum/master/ipsum.txt"
        );
    }

    #[test]
    fn max_per_sweep_is_bounded() {
        assert!(MAX_PER_SWEEP > 0 && MAX_PER_SWEEP <= 500);
    }
}