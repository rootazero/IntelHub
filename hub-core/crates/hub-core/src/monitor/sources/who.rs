//! WHO Disease Outbreak News (DONs) — global epidemic early warning.
//! Free JSON API. Hard-won lessons (live-probed 2026-09): the old RSS feed
//! 404s; the JSON endpoint IGNORES $orderby, caps $top at 100, and serves
//! pages in non-chronological order — but $filter=PublicationDate ge
//! <date> works, which is the only sane way to get recent items.
//! Titles geotagged via the shared keyword table (DON titles always name
//! the country), falling back to WHO Geneva HQ when truly unmatchable.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};
use super::rss;

const DON_API: &str = "https://www.who.int/api/news/diseaseoutbreaknews";
const GENEVA: (f64, f64) = (46.2044, 6.1432); // WHO HQ fallback

pub struct Who;

impl Source for Who {
    fn name(&self) -> &'static str {
        "who"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let since = (chrono::Utc::now() - chrono::Duration::days(60)).format("%Y-%m-%d");
            let url = format!("{DON_API}?$filter=PublicationDate ge {since}&$top=100");
            let resp = ctx.http.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("WHO DON HTTP {}", resp.status())));
            }
            let j: serde_json::Value = resp.json().await?;
            let mut items: Vec<serde_json::Value> =
                j.get("value").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            // Sort by PublicationDate desc (server ignores $orderby), keep 30d/20.
            items.sort_by_key(|i| {
                std::cmp::Reverse(
                    i.get("PublicationDate").and_then(|d| d.as_str()).map(String::from),
                )
            });
            let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
            let mut out = Vec::new();
            for item in items.into_iter().take(40) {
                let title = item.get("Title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                if title.is_empty() {
                    continue;
                }
                let occurred_at = item
                    .get("PublicationDate")
                    .and_then(|d| d.as_str())
                    .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);
                if occurred_at < cutoff {
                    continue;
                }
                if out.len() >= 20 {
                    break;
                }
                let (lat, lon) = rss::geotag(&title).unwrap_or(GENEVA);
                let ext = item
                    .get("DonId")
                    .and_then(|d| d.as_str().map(String::from).or_else(|| d.as_i64().map(|n| n.to_string())))
                    .map(|id| format!("who:{id}"))
                    .unwrap_or_else(|| {
                        use sha2::Digest;
                        let h = format!("{:x}", sha2::Sha256::digest(title.as_bytes()));
                        format!("who:{}", &h[..16])
                    });
                let summary = item
                    .get("Summary")
                    .or_else(|| item.get("Overview"))
                    .and_then(|s| s.as_str())
                    .map(|s| s.chars().take(300).collect::<String>());
                let url = item.get("ItemDefaultUrl").and_then(|u| u.as_str()).map(|p| {
                    format!("https://www.who.int/emergencies/disease-outbreak-news{p}")
                });
                out.push(
                    Signal::new("health", title, lat, lon, ext)
                        .severity("priority")
                        .occurred(occurred_at)
                        .payload(serde_json::json!({ "summary": summary, "url": url })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}
