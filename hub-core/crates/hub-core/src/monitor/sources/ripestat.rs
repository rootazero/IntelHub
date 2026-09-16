//! RIPEstat `abuse-contact-finder` IP abuse-contact lookup.
//!
//! Free keyless RIPE stat service. For each IP in the watchlist,
//! returns the abuse-contact email(s) registered with the
//! regional Internet registry (RIPE / ARIN / APNIC / LACNIC /
//! AFRINIC). This is a classic OSINT step for incident response:
//! when a packet lands from a hostile IP, you want to know who
//! to email — RIPEstat gives you that in one HTTP call.
//!
//! Each hit becomes a `cyber`-kind Signal anchored at RIPE NCC
//! HQ (Amsterdam). Severity escalates when the abuse-contact
//! field is non-empty (priority) vs empty (info).

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INTER_QUERY_GAP: Duration = Duration::from_millis(250);

/// Anchor coordinates for Amsterdam (RIPE NCC HQ).
const RIPE_LAT: f64 = 52.3676;
const RIPE_LON: f64 = 4.9041;

const DEFAULT_WATCH: &[&str] = &[
    "1.1.1.1",            // Cloudflare DNS
    "8.8.8.8",            // Google DNS
    "9.9.9.9",            // Quad9 DNS
    "208.67.222.222",     // OpenDNS
    "140.82.121.4",       // GitHub
];

#[derive(Clone)]
pub struct Ripestat {
    pub watch: Vec<String>,
}

impl Default for Ripestat {
    fn default() -> Self {
        Self {
            watch: DEFAULT_WATCH.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct RipestatEnvelope {
    #[serde(rename = "data_call_status")]
    status: String,
    data: Option<RipestatData>,
}

#[derive(Deserialize)]
struct RipestatData {
    abuse_contacts: Vec<String>,
    #[serde(rename = "latest_abuse_contact_finder_result")]
    latest_result: Option<String>,
    #[serde(rename = "authoritative_abuse_contacts")]
    authoritative: Option<Vec<String>>,
}

impl Source for Ripestat {
    fn name(&self) -> &'static str {
        "ripestat"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move {
            let mut sigs = Vec::new();
            for (i, ip) in self.watch.iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query(ctx, ip).await {
                    Ok(Some(sig)) => sigs.push(sig),
                    Ok(None) => {
                        tracing::debug!(ip = %ip, "ripestat: no result");
                    }
                    Err(e) => {
                        tracing::warn!(
                            ip = %ip,
                            error = %e,
                            "ripestat: query failed",
                        );
                    }
                }
            }
            Ok(sigs)
        })
    }
}

async fn query(ctx: &Ctx, ip: &str) -> Result<Option<Signal>, HubError> {
    let url = format!(
        "https://stat.ripe.net/data/abuse-contact-finder/data.json?resource={ip}"
    );
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(&url)
            .header("Accept", "application/json")
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("ripestat: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("ripestat: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "ripestat: HTTP {s}: {snip}"
        )));
    }

    let body: RipestatEnvelope = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("ripestat: parse: {e}")))?;

    if body.status != "supported" {
        return Ok(None);
    }

    let Some(data) = body.data else {
        return Ok(None);
    };
    if data.abuse_contacts.is_empty()
        && data
            .authoritative
            .as_ref()
            .map(|a| a.is_empty())
            .unwrap_or(true)
    {
        return Ok(None);
    }

    let sev: &'static str = if !data.abuse_contacts.is_empty() {
        "priority"
    } else {
        "info"
    };

    let payload = json!({
        "ip": ip,
        "abuse_contacts": data.abuse_contacts,
        "authoritative_abuse_contacts": data.authoritative,
        "latest_abuse_contact_finder_result": data.latest_result,
        "looked_up_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("RIPEstat abuse contact: {ip}"),
            RIPE_LAT,
            RIPE_LON,
            format!("ripestat:{ip}"),
        )
        .severity(sev)
        .payload(payload),
    ))
}