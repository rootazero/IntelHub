//! Tor exit-addresses rich metadata feed.
//!
//! Free keyless endpoint at `https://check.torproject.org/
//! exit-addresses` (~498 KB, ~12,686 lines). Complements the
//! existing tor_exit collector (which polls the bare-IP
//! `torbulkexitlist`) by exposing per-relay metadata:
//! fingerprint + Published timestamp + LastStatus + ExitAddress
//! + the timestamp the address was first seen. The format
//! is line-oriented:
//!
//!   ExitNode 64D74AAA74F30DC2CFB36343CE5D4451B9A4DBA8
//!   Published 2026-09-15 22:50:30
//!   LastStatus 2026-09-16 14:00:00
//!   ExitAddress 171.25.193.25 2026-09-16 14:40:52
//!   ...
//!
//! Use case: tor_exit gives IPs (volatile); tor_exit_details
//! gives the RELAY IDENTITY (fingerprint) that owns those
//! IPs (stable across multiple address changes). When OSINT
//! intel flags a hostile Tor IP, the fingerprint lets you
//! pivot to relay metadata (when it joined the network,
//! when it was last seen, etc.) for forensics.
//!
//! MAX_PER_SWEEP: 30 fingerprints per sweep (Tor network has
//! ~3000 active exits; 30 is a representative sample for
//! the radar without flooding it).
//!
//! Anchor: Tor Project HQ — Seattle WA (47.6062, -122.3321).

use std::time::Duration;

use chrono::Utc;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_PER_SWEEP: usize = 30;

/// Tor Project HQ anchor — Seattle WA.
const TOR_LAT: f64 = 47.6062;
const TOR_LON: f64 = -122.3321;

impl Source for TorExitDetails {
    fn name(&self) -> &'static str {
        "tor_exit_details"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await })
    }
}

pub struct TorExitDetails;

#[derive(Default)]
struct RelayBlock {
    fingerprint: Option<String>,
    published: Option<String>,
    last_status: Option<String>,
    addresses: Vec<(String, String)>, // (ip, seen_at)
}

impl RelayBlock {
    fn flush(self) -> Option<TorRelay> {
        let fp = self.fingerprint?;
        if self.addresses.is_empty() {
            return None;
        }
        Some(TorRelay {
            fingerprint: fp,
            published: self.published,
            last_status: self.last_status,
            addresses: self.addresses,
        })
    }
}

struct TorRelay {
    fingerprint: String,
    published: Option<String>,
    last_status: Option<String>,
    addresses: Vec<(String, String)>,
}

async fn fetch(ctx: &Ctx) -> Result<Vec<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get("https://check.torproject.org/exit-addresses")
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("tor_exit_details: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("tor_exit_details: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "tor_exit_details: HTTP {s}: {snip}"
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| HubError::sensor(format!("tor_exit_details: body: {e}")))?;

    // Parse line-oriented blocks. A new block starts when we
    // see an `ExitNode` line. We flush the previous block when
    // a new `ExitNode` is seen or at end of stream.
    let mut relays: Vec<TorRelay> = Vec::new();
    let mut cur = RelayBlock::default();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("ExitNode ") {
            // New block — flush the previous.
            if let Some(r) = cur.flush() {
                relays.push(r);
            }
            cur = RelayBlock::default();
            cur.fingerprint = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Published ") {
            cur.published = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("LastStatus ") {
            cur.last_status = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("ExitAddress ") {
            // Format: "<ip> <timestamp>"
            let mut parts = rest.split_whitespace();
            if let (Some(ip), Some(seen_at)) = (parts.next(), parts.next()) {
                cur.addresses.push((ip.to_string(), seen_at.to_string()));
            }
        }
        // Other lines (e.g. "Bandwidth", etc.) ignored.
    }
    if let Some(r) = cur.flush() {
        relays.push(r);
    }

    if relays.is_empty() {
        return Ok(Vec::new());
    }

    // Sort by fingerprint for deterministic ordering across sweeps.
    relays.sort_by(|a, b| a.fingerprint.cmp(&b.fingerprint));

    let now = Utc::now();
    let mut sigs = Vec::new();
    for relay in relays.iter().take(MAX_PER_SWEEP) {
        let payload = json!({
            "fingerprint": relay.fingerprint,
            "published": relay.published,
            "last_status": relay.last_status,
            "addresses": relay.addresses.iter().map(|(ip, ts)| {
                json!({"ip": ip, "seen_at": ts})
            }).collect::<Vec<_>>(),
            "address_count": relay.addresses.len(),
            "fetched_at": now.to_rfc3339(),
        });

        let summary = format!(
            "Tor exit relay {} ({} addrs, published {})",
            &relay.fingerprint[..8.min(relay.fingerprint.len())],
            relay.addresses.len(),
            relay.published.as_deref().unwrap_or("?"),
        );

        // external_id embeds fingerprint for stable dedup —
        // the fingerprint survives IP rotations.
        sigs.push(
            Signal::new(
                "cyber",
                summary,
                TOR_LAT,
                TOR_LON,
                format!("tor_exit_details:{}", relay.fingerprint),
            )
            .payload(payload),
        );
    }

    Ok(sigs)
}