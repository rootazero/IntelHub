//! GDELT conflict/crisis news via the DOC 2.0 API (ArtList). The GEO 2.0
//! PointData endpoint (/api/v2/geo/geo) was RETIRED upstream (404 across all
//! variants since 2026-09, confirmed from two networks); DOC API articles
//! carry no coordinates, so titles are geotagged with the shared
//! keyword→centroid table (same matcher as the RSS source — points stay
//! "where the event is", never "where the publisher is"). Keyless but HARD
//! rate-limited: ≤1 request / 5s — enforced via the shared per-host limiter.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};
use super::rss::geotag;

const URL: &str = "https://api.gdeltproject.org/api/v2/doc/doc?query=conflict%20OR%20military%20OR%20protest%20OR%20crisis&mode=ArtList&maxrecords=75&timespan=24h&format=json&sort=DateDesc";

pub struct Gdelt;

impl Source for Gdelt {
    fn name(&self) -> &'static str {
        "gdelt"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(900)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            ctx.limiter
                .wait("api.gdeltproject.org", Duration::from_secs(5))
                .await;
            let resp = ctx.http.get(URL).send().await?;
            if resp.status().as_u16() == 429 {
                return Err(HubError::sensor(
                    "GDELT 429 rate limit (one request per 5s per IP) — will retry next sweep",
                ));
            }
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("GDELT HTTP {}", resp.status())));
            }
            let j: serde_json::Value = resp.json().await?;
            Ok(parse_articles(&j))
        }
        .boxed()
    }
}

/// DOC API ArtList → geo signals. Articles whose title matches no keyword
/// are skipped (no coordinates = not mappable); each signal dedupes on the
/// article URL hash so repeat sweeps don't re-ingest.
fn parse_articles(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(articles) = j.get("articles").and_then(|a| a.as_array()) else {
        return out;
    };
    for a in articles {
        let title = a.get("title").and_then(|t| t.as_str()).unwrap_or("");
        if title.is_empty() {
            continue;
        }
        let Some((lat, lon)) = geotag(title) else { continue };
        let url = a.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let ext = {
            use sha2::Digest;
            let h = format!("{:x}", sha2::Sha256::digest(url.as_bytes()));
            format!("gdelt:{}", &h[..16])
        };
        let mut sig = Signal::new("news", title, lat, lon, ext).severity("info");
        if let Some(sd) = a.get("seendate").and_then(|s| s.as_str()) {
            // GDELT v2 seendate: "20260910T143000Z" (older dumps: "20260910143000")
            let ts = chrono::NaiveDateTime::parse_from_str(sd, "%Y%m%dT%H%M%SZ")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(sd, "%Y%m%d%H%M%S"))
                .ok();
            if let Some(t) = ts {
                sig = sig.occurred(t.and_utc());
            }
        }
        out.push(sig.payload(serde_json::json!({
            "url": url,
            "domain": a.get("domain").and_then(|d| d.as_str()).unwrap_or(""),
            "sourcecountry": a.get("sourcecountry").and_then(|c| c.as_str()).unwrap_or(""),
            "seendate": a.get("seendate").and_then(|s| s.as_str()).unwrap_or(""),
            "api": "doc-v2-artlist",
        })));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_geotags() {
        let j = serde_json::json!({"articles": [
            {"title": "Missile strike reported in Ukraine", "url": "https://ex.com/1",
             "seendate": "20260910T143000Z", "domain": "ex.com", "sourcecountry": "United States"},
            {"title": "Quarterly earnings beat expectations", "url": "https://ex.com/2",
             "seendate": "20260910T150000Z", "domain": "ex.com", "sourcecountry": "United States"},
        ]});
        let sigs = parse_articles(&j);
        assert_eq!(sigs.len(), 1, "non-geotagged article must be skipped");
        assert_eq!(sigs[0].kind, "news");
        assert!((sigs[0].lat - 49.0).abs() < 0.01, "Ukraine centroid");
        assert!(sigs[0].external_id.starts_with("gdelt:"));
        assert_eq!(sigs[0].occurred_at.format("%Y-%m-%d").to_string(), "2026-09-10");
    }

    #[test]
    fn legacy_seendate_format() {
        let j = serde_json::json!({"articles": [
            {"title": "Protests in Iran", "url": "https://ex.com/3", "seendate": "20260910143000"},
        ]});
        let sigs = parse_articles(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].occurred_at.format("%H:%M").to_string(), "14:30");
    }

    #[test]
    fn empty_when_no_articles_key() {
        assert!(parse_articles(&serde_json::json!({})).is_empty());
    }
}
