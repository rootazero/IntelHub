//! Internet Archive Wayback Machine URL archive lookup.
//!
//! Free keyless API: given a URL, returns the closest archived
//! snapshot (status + URL + timestamp). Useful for:
//! - phishing forensics — see when a phishing domain first
//!   appeared and how it has evolved
//! - takedown verification — confirm a hostile URL was archived
//!   before being taken down (helps with attribution chains)
//! - content drift detection — see what changed on a target
//!   site over time
//!
//! Each watch entry is a URL. We emit a `cyber`-kind Signal
//! anchored at the Internet Archive HQ (San Francisco) with
//! the archive snapshot URL + timestamp in the payload.
//! Severity escalates if the snapshot is recent.

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INTER_QUERY_GAP: Duration = Duration::from_millis(200);

const ARCHIVE_LAT: f64 = 37.8024;
const ARCHIVE_LON: f64 = -122.4058;

const DEFAULT_WATCH: &[&str] = &[
    "google.com",
    "microsoft.com",
    "github.com",
    "gov.uk",
    "iana.org",
];

#[derive(Clone)]
pub struct Wayback {
    pub watch: Vec<String>,
}

impl Default for Wayback {
    fn default() -> Self {
        Self {
            watch: DEFAULT_WATCH.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct WaybackEnvelope {
    #[serde(rename = "archived_snapshots")]
    archived_snapshots: WaybackSnapshot,
}

#[derive(Deserialize)]
struct WaybackSnapshot {
    closest: Option<WaybackHit>,
}

#[derive(Deserialize)]
struct WaybackHit {
    available: bool,
    url: String,
    timestamp: String,
}

impl Source for Wayback {
    fn name(&self) -> &'static str {
        "wayback"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move {
            let mut sigs = Vec::new();
            for (i, target) in self.watch.iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query(ctx, target).await {
                    Ok(Some(sig)) => sigs.push(sig),
                    Ok(None) => {
                        tracing::debug!(target = %target, "wayback: no snapshot");
                    }
                    Err(e) => {
                        tracing::warn!(
                            target = %target,
                            error = %e,
                            "wayback: query failed",
                        );
                    }
                }
            }
            Ok(sigs)
        })
    }
}

async fn query(ctx: &Ctx, target: &str) -> Result<Option<Signal>, HubError> {
    let url = format!(
        "https://archive.org/wayback/available?url={}",
        urlencoded(target),
    );
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http.get(&url).send(),
    )
    .await
    .map_err(|_| HubError::sensor("wayback: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("wayback: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "wayback: HTTP {s}: {snip}"
        )));
    }

    let body: WaybackEnvelope = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("wayback: parse: {e}")))?;

    let Some(closest) = body.archived_snapshots.closest else {
        return Ok(None);
    };
    if !closest.available {
        return Ok(None);
    }

    let ts = parse_wayback_ts(&closest.timestamp).unwrap_or_else(Utc::now);
    let age_days = (Utc::now() - ts).num_days();
    let sev: &'static str = if age_days <= 1 {
        "flash"
    } else if age_days <= 90 {
        "priority"
    } else {
        "info"
    };

    let payload = json!({
        "target_url": target,
        "snapshot_url": closest.url,
        "timestamp_wayback": closest.timestamp,
        "timestamp_iso": ts.to_rfc3339(),
        "age_days": age_days,
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("Wayback snapshot: {target}"),
            ARCHIVE_LAT,
            ARCHIVE_LON,
            format!("wayback:{}", content_hash(target)),
        )
        .severity(sev)
        .occurred(ts)
        .payload(payload),
    ))
}

/// Parse Wayback Machine timestamp `YYYYMMDDhhmmss` (UTC).
fn parse_wayback_ts(s: &str) -> Option<DateTime<Utc>> {
    if s.len() < 14 {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(&s[..14], "%Y%m%d%H%M%S").ok()?;
    Some(Utc.from_utc_datetime(&naive))
}

/// Stable content hash for external_id (FNV-1a 64-bit, hex).
fn content_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}