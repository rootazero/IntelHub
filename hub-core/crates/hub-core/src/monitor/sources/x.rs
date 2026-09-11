//! X.com (Twitter) watch — VISIBLE-DEGRADED by design (SP8-C).
//!
//! X has no free read path anymore (guest tokens killed 2023-2024, Nitter
//! mostly dead, snscrape broken). The only sustainable route is the paid
//! Basic tier ($200/mo, 10k reads). Per user decision 2026-09-11: stay on
//! free sources, but keep this collector in the registry so the gap is
//! VISIBLE on the health board and self-heals the day X_BEARER_TOKEN is
//! configured. Default watchlist deliberately covers the Bluesky gap
//! (accounts with no official bsky presence).

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};
use super::{rss::geotag, textclass::classify_title};

/// Default X watchlist = the Bluesky gap (verified absence 2026-09-11).
pub const DEFAULT_WATCH: &[&str] = &[
    "StateDept", "WhiteHouse", "NATO", "IMFNews", "WorldBank", "OpenAI",
];

pub struct XWatch;

impl Source for XWatch {
    fn name(&self) -> &'static str {
        "x"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(token) = ctx.config.x_bearer_token.clone() else {
                return Err(HubError::internal(
                    "X_BEARER_TOKEN not configured — X killed free read access (guest tokens dead, \
                     Nitter dying); only the paid Basic tier ($200/mo) remains. Watching Bluesky + \
                     Telegram meanwhile. Set the token and this collector self-heals",
                ));
            };
            let watch: Vec<String> = if ctx.config.x_watch.is_empty() {
                DEFAULT_WATCH.iter().map(|s| s.to_string()).collect()
            } else {
                ctx.config.x_watch.clone()
            };
            let mut out: Vec<Signal> = Vec::new();
            for handle in watch {
                let url = format!(
                    "https://api.x.com/2/tweets/search/recent\
                     ?query=from:{handle}&max_results=10&tweet.fields=created_at,public_metrics"
                );
                let resp = ctx.http.get(&url).bearer_auth(&token).send().await?;
                if resp.status().as_u16() == 403 {
                    return Err(HubError::internal(
                        "X API 403 — token tier too low (need Basic $200/mo) or suspended",
                    ));
                }
                if !resp.status().is_success() {
                    return Err(HubError::sensor(format!("X API HTTP {}", resp.status())));
                }
                let j: serde_json::Value = resp.json().await?;
                out.extend(parse_tweets(&j, &handle));
            }
            Ok(out)
        }
        .boxed()
    }
}

/// recent-search payload → signals. Pure + unit-tested (endpoint is paywalled).
pub fn parse_tweets(j: &serde_json::Value, handle: &str) -> Vec<Signal> {
    let mut out = Vec::new();
    for t in j.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default() {
        let text = t["text"].as_str().unwrap_or("").trim().to_string();
        if text.len() < 20 {
            continue;
        }
        let Some((lat, lon)) = geotag(&text) else { continue };
        let id = t["id"].as_str().unwrap_or("x");
        let mut sig = Signal::new(
            classify_title(&text).unwrap_or("political"),
            text.chars().take(120).collect::<String>(),
            lat,
            lon,
            format!("x:{handle}:{id}"),
        )
        .severity("info");
        if let Some(ts) = t["created_at"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        {
            sig = sig.occurred(ts.with_timezone(&Utc));
        }
        out.push(sig.payload(serde_json::json!({
            "author": handle,
            "url": format!("https://x.com/{handle}/status/{id}"),
            "metrics": t["public_metrics"].clone(),
        })));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_recent_search() {
        let j = serde_json::json!({ "data": [
            { "id": "1700", "text": "Statement on the situation in Ukraine: sanctions remain in force", "created_at": "2026-09-11T08:00:00.000Z" },
            { "id": "1701", "text": "Happy Friday everyone!", "created_at": "2026-09-11T07:00:00.000Z" },
        ]});
        let sigs = parse_tweets(&j, "StateDept");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].kind, "political"); // sanctions keyword
        assert_eq!(sigs[0].external_id, "x:StateDept:1700");
    }
}
