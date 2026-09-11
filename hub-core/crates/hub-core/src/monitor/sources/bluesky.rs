//! Bluesky official-account watch (SP8-C social plane, free tier).
//!
//! The AT Protocol public API is free, keyless, and server-friendly:
//!   GET public.api.bsky.app/xrpc/app.bsky.feed.getAuthorFeed?actor=H&limit=20
//! Watchlist = built-in defaults (all handles live-verified 2026-09-11)
//! overridable via BSKY_WATCH="handle|kind,handle". Posts carry no
//! coordinates → geotag the text; skip what isn't mappable. Kind comes from
//! the shared headline classifier, falling back to the per-account default.

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};
use super::{rss::geotag, textclass::{classify_title, static_kind}};

/// (handle, default kind) — the 2026-09-11 verified set (posts>0 or official).
pub const DEFAULT_WATCH: &[(&str, &str)] = &[
    // geopolitics / conflict analysis
    ("bellingcat.com", "political"),
    ("warontherocks.bsky.social", "political"),
    ("liveuamap.com", "conflict"),
    ("csis.org", "political"),
    ("crisisgroup.org", "political"),
    ("carnegieendowment.org", "political"),
    ("atlanticcouncil.bsky.social", "political"),
    ("rusi.bsky.social", "political"),
    ("cnas.bsky.social", "political"),
    // official statements
    ("un.org", "political"),
    ("unhcr.org", "political"),
    ("wfp.org", "political"),
    ("who.int", "health"),
    ("icrc.org", "political"),
    ("consilium.europa.eu", "political"),
    ("europeancommission.bsky.social", "political"),
    ("iaeaorg.bsky.social", "political"),
    // finance / central banks
    ("federalreserve.gov", "financial"),
    ("ecb.europa.eu", "financial"),
    ("oecd-ocde.bsky.social", "financial"),
    ("treasurydept.bsky.social", "financial"),
    ("financialtimes.com", "financial"),
    ("economist.com", "financial"),
    ("wsj.com", "financial"),
    ("cnbc.com", "financial"),
    // wires
    ("reuters.com", "news"),
    ("apnews.com", "news"),
    // tech
    ("theverge.com", "news"),
    ("techcrunch.com", "news"),
    ("wired.com", "news"),
    ("arstechnica.com", "news"),
    ("technologyreview.com", "news"),
    ("github.com", "news"),
    ("theregister.com", "news"),
    ("engadget.com", "news"),
];

pub struct Bluesky;

/// Config list ("handle|kind") or built-in defaults.
fn watchlist(cfg: &[String]) -> Vec<(String, String)> {
    if cfg.is_empty() {
        return DEFAULT_WATCH.iter().map(|(h, k)| (h.to_string(), k.to_string())).collect();
    }
    cfg.iter()
        .map(|e| {
            let mut parts = e.splitn(2, '|');
            let h = parts.next().unwrap_or("").trim().to_string();
            let k = parts.next().unwrap_or("news").trim().to_string();
            (h, k)
        })
        .filter(|(h, _)| !h.is_empty())
        .collect()
}

impl Source for Bluesky {
    fn name(&self) -> &'static str {
        "bluesky"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(1800)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let watch = watchlist(&ctx.config.bsky_watch);
            let mut out: Vec<Signal> = Vec::new();
            for (handle, defkind) in &watch {
                let url = format!(
                    "https://public.api.bsky.app/xrpc/app.bsky.feed.getAuthorFeed\
                     ?actor={handle}&limit=20&filter=posts_no_replies"
                );
                match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let body = r.text().await.unwrap_or_default();
                        match serde_json::from_str::<serde_json::Value>(&body) {
                            Ok(j) => out.extend(parse_feed(&j, handle, defkind)),
                            Err(e) => {
                                tracing::warn!(handle = %handle, error = %e, "bluesky decode")
                            }
                        }
                    }
                    Ok(r) => {
                        tracing::warn!(handle = %handle, status = %r.status(), "bluesky http")
                    }
                    Err(e) => {
                        tracing::warn!(handle = %handle, error = %e, "bluesky fetch")
                    }
                }
                if out.len() >= 60 {
                    break;
                }
            }
            Ok(out)
        }
        .boxed()
    }
}

/// getAuthorFeed payload → signals. Pure + unit-tested.
pub fn parse_feed(j: &serde_json::Value, handle: &str, defkind: &str) -> Vec<Signal> {
    let mut out = Vec::new();
    for item in j.get("feed").and_then(|f| f.as_array()).cloned().unwrap_or_default() {
        let post = &item["post"];
        let text = post["record"]["text"].as_str().unwrap_or("").trim().to_string();
        if text.len() < 20 {
            continue; // reposts/media-only stubs carry no signal
        }
        let Some((lat, lon)) = geotag(&text) else { continue }; // mappable-only
        let rkey = post["uri"]
            .as_str()
            .and_then(|u| u.rsplit('/').next())
            .unwrap_or("x");
        let mut sig = Signal::new(
            classify_title(&text).unwrap_or(static_kind(defkind)),
            text.chars().take(120).collect::<String>(),
            lat,
            lon,
            format!("bsky:{handle}:{rkey}"),
        )
        .severity("info");
        if let Some(t) = post["record"]["createdAt"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        {
            sig = sig.occurred(t.with_timezone(&Utc));
        }
        out.push(sig.payload(serde_json::json!({
            "author": handle,
            "url": format!("https://bsky.app/profile/{handle}/post/{rkey}"),
            "likes": post["likeCount"].as_i64().unwrap_or(0),
            "reposts": post["repostCount"].as_i64().unwrap_or(0),
        })));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_feed_and_skips_unmappable() {
        let j = serde_json::json!({ "feed": [
            { "post": {
                "uri": "at://did:plc:x/app.bsky.feed.post/abc123",
                "record": { "text": "Breaking: missile strike reported near Kyiv overnight, officials say", "createdAt": "2026-09-11T08:00:00.000Z" },
                "likeCount": 42, "repostCount": 5,
            } },
            { "post": {
                "uri": "at://did:plc:x/app.bsky.feed.post/def456",
                "record": { "text": "Our new annual report on transparency is now available for download", "createdAt": "2026-09-11T07:00:00.000Z" },
            } },
            { "post": { "uri": "at://did:plc:x/app.bsky.feed.post/ghi", "record": { "text": "short" } } },
        ]});
        let sigs = parse_feed(&j, "un.org", "political");
        assert_eq!(sigs.len(), 1, "only the Kyiv post is mappable");
        assert_eq!(sigs[0].kind, "conflict"); // classifier: missile/strike
        assert_eq!(sigs[0].external_id, "bsky:un.org:abc123");
        assert!((sigs[0].lat - 50.4).abs() < 0.5, "Kyiv centroid");
    }
}
