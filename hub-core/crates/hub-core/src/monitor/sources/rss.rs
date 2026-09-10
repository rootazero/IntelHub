//! RSS news layer — 19 keyless feeds. Ported from Crucix `dashboard/inject.mjs`
//! with its two defects fixed (spec §3):
//!   1. NO ±1° random jitter (Crucix re-randomized coordinates every sweep —
//!      the same story wandered the map; we emit stable centroids).
//!   2. Precision is honest: every signal carries geo_precision:"country".
//! Geotag: first keyword hit in the title over a 164-entry centroid table
//! (ported verbatim), with per-feed fallbacks for 4 regional sources.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const FEEDS: &[(&str, &str)] = &[
    ("http://feeds.bbci.co.uk/news/world/rss.xml", "BBC"),
    ("https://rss.nytimes.com/services/xml/rss/nyt/World.xml", "NYT"),
    ("https://www.aljazeera.com/xml/rss/all.xml", "Al Jazeera"),
    ("https://feeds.npr.org/1001/rss.xml", "NPR"),
    ("https://feeds.bbci.co.uk/news/technology/rss.xml", "BBC Tech"),
    ("https://feeds.bbci.co.uk/news/science_and_environment/rss.xml", "BBC Science"),
    ("https://rss.nytimes.com/services/xml/rss/nyt/Americas.xml", "NYT Americas"),
    ("https://rss.dw.com/rdf/rss-en-all", "DW"),
    ("https://www.france24.com/en/rss", "France 24"),
    ("https://www.euronews.com/rss?format=mrss", "Euronews"),
    ("https://rss.dw.com/rdf/rss-en-africa", "DW Africa"),
    ("https://www.rfi.fr/en/rss", "RFI"),
    ("https://www.africanews.com/feed/rss", "Africa News"),
    ("https://rss.nytimes.com/services/xml/rss/nyt/Africa.xml", "NYT Africa"),
    ("https://rss.nytimes.com/services/xml/rss/nyt/AsiaPacific.xml", "NYT Asia"),
    ("https://www.sbs.com.au/news/topic/australia/feed", "SBS Australia"),
    ("https://indianexpress.com/section/india/feed/", "Indian Express"),
    ("https://www.thehindu.com/news/national/feeder/default.rss", "The Hindu"),
    ("https://en.mercopress.com/rss/latin-america", "MercoPress"),
];

/// Per-feed fallback when no keyword matches (Crucix RSS_SOURCE_FALLBACKS).
const FEED_FALLBACKS: &[(&str, f64, f64)] = &[
    ("SBS Australia", -35.2809, 149.13),
    ("Indian Express", 28.6139, 77.209),
    ("The Hindu", 13.0827, 80.2707),
    ("MercoPress", -34.9011, -56.1645),
];

const MAX_SIGNALS: usize = 50;

/// 164 keyword→centroid entries ported from Crucix `geoKeywords` (verbatim;
/// jitter removed). Order matters: first hit wins (Crucix semantics).
const GEO_KEYWORDS: &[(&str, f64, f64)] = &[
    ("Ukraine", 49.0, 32.0), ("Russia", 56.0, 38.0), ("Moscow", 55.7, 37.6), ("Kyiv", 50.4, 30.5),
    ("China", 35.0, 105.0), ("Beijing", 39.9, 116.4), ("Iran", 32.0, 53.0), ("Tehran", 35.7, 51.4),
    ("Israel", 31.5, 35.0), ("Gaza", 31.4, 34.4), ("Palestine", 31.9, 35.2), ("Syria", 35.0, 38.0),
    ("Iraq", 33.0, 44.0), ("Saudi", 24.0, 45.0), ("Yemen", 15.0, 48.0), ("Lebanon", 34.0, 36.0),
    ("India", 20.0, 78.0), ("Japan", 36.0, 138.0), ("Korea", 37.0, 127.0), ("Pyongyang", 39.0, 125.7),
    ("Taiwan", 23.5, 121.0), ("Philippines", 13.0, 122.0), ("Myanmar", 20.0, 96.0), ("Canada", 56.0, -96.0),
    ("Mexico", 23.0, -102.0), ("Brazil", -14.0, -51.0), ("Argentina", -38.0, -63.0), ("Colombia", 4.0, -74.0),
    ("Venezuela", 7.0, -66.0), ("Cuba", 22.0, -80.0), ("Chile", -35.0, -71.0), ("Germany", 51.0, 10.0),
    ("France", 46.0, 2.0), ("UK", 54.0, -2.0), ("Britain", 54.0, -2.0), ("London", 51.5, -0.1),
    ("Spain", 40.0, -4.0), ("Italy", 42.0, 12.0), ("Poland", 52.0, 20.0), ("NATO", 50.0, 4.0),
    ("EU", 50.0, 4.0), ("Turkey", 39.0, 35.0), ("Greece", 39.0, 22.0), ("Romania", 46.0, 25.0),
    ("Finland", 64.0, 26.0), ("Sweden", 62.0, 15.0), ("Africa", 0.0, 20.0), ("Nigeria", 10.0, 8.0),
    ("South Africa", -30.0, 25.0), ("Kenya", -1.0, 38.0), ("Egypt", 27.0, 30.0), ("Libya", 27.0, 17.0),
    ("Sudan", 13.0, 30.0), ("Ethiopia", 9.0, 38.0), ("Somalia", 5.0, 46.0), ("Congo", -4.0, 22.0),
    ("Uganda", 1.0, 32.0), ("Morocco", 32.0, -6.0), ("Pakistan", 30.0, 70.0), ("Afghanistan", 33.0, 65.0),
    ("Bangladesh", 24.0, 90.0), ("Australia", -25.0, 134.0), ("Indonesia", -2.0, 118.0), ("Thailand", 15.0, 100.0),
    ("US", 39.0, -98.0), ("America", 39.0, -98.0), ("Washington", 38.9, -77.0), ("Pentagon", 38.9, -77.0),
    ("Trump", 38.9, -77.0), ("White House", 38.9, -77.0), ("Wall Street", 40.7, -74.0), ("New York", 40.7, -74.0),
    ("California", 37.0, -120.0), ("Nepal", 28.0, 84.0), ("Cambodia", 12.5, 105.0), ("Malawi", -13.5, 34.0),
    ("Burundi", -3.4, 29.9), ("Oman", 21.0, 57.0), ("Netherlands", 52.1, 5.3), ("Gabon", -0.8, 11.6),
    ("Peru", -10.0, -76.0), ("Ecuador", -2.0, -78.0), ("Bolivia", -17.0, -65.0), ("Singapore", 1.35, 103.8),
    ("Malaysia", 4.2, 101.9), ("Vietnam", 16.0, 108.0), ("Algeria", 28.0, 3.0), ("Tunisia", 34.0, 9.0),
    ("Zimbabwe", -20.0, 30.0), ("Mozambique", -18.0, 35.0), ("Texas", 31.0, -100.0), ("Florida", 28.0, -82.0),
    ("Chicago", 41.9, -87.6), ("Los Angeles", 34.0, -118.0), ("San Francisco", 37.8, -122.4), ("Seattle", 47.6, -122.3),
    ("Miami", 25.8, -80.2), ("Toronto", 43.7, -79.4), ("Ottawa", 45.4, -75.7), ("Vancouver", 49.3, -123.1),
    ("São Paulo", -23.5, -46.6), ("Rio", -22.9, -43.2), ("Buenos Aires", -34.6, -58.4), ("Bogotá", 4.7, -74.1),
    ("Lima", -12.0, -77.0), ("Santiago", -33.4, -70.7), ("Caracas", 10.5, -66.9), ("Havana", 23.1, -82.4),
    ("Panama", 9.0, -79.5), ("Guatemala", 14.6, -90.5), ("Honduras", 14.1, -87.2), ("El Salvador", 13.7, -89.2),
    ("Costa Rica", 10.0, -84.0), ("Jamaica", 18.1, -77.3), ("Haiti", 19.0, -72.0), ("Dominican", 18.5, -70.0),
    ("Puerto Rico", 18.2, -66.5), ("Sri Lanka", 7.0, 80.0), ("Hong Kong", 22.3, 114.2), ("Taipei", 25.0, 121.5),
    ("Seoul", 37.6, 127.0), ("Osaka", 34.7, 135.5), ("Mumbai", 19.1, 72.9), ("Delhi", 28.6, 77.2),
    ("Shanghai", 31.2, 121.5), ("Shenzhen", 22.5, 114.1), ("Auckland", -36.8, 174.8), ("Papua New Guinea", -6.3, 147.0),
    ("Berlin", 52.5, 13.4), ("Paris", 48.9, 2.3), ("Madrid", 40.4, -3.7), ("Rome", 41.9, 12.5),
    ("Warsaw", 52.2, 21.0), ("Prague", 50.1, 14.4), ("Vienna", 48.2, 16.4), ("Budapest", 47.5, 19.1),
    ("Bucharest", 44.4, 26.1), ("Oslo", 59.9, 10.7), ("Copenhagen", 55.7, 12.6), ("Brussels", 50.8, 4.4),
    ("Zurich", 47.4, 8.5), ("Dublin", 53.3, -6.3), ("Lisbon", 38.7, -9.1), ("Athens", 37.9, 23.7),
    ("Minsk", 53.9, 27.6), ("Nairobi", -1.3, 36.8), ("Lagos", 6.5, 3.4), ("Accra", 5.6, -0.2),
    ("Addis Ababa", 9.0, 38.7), ("Cape Town", -33.9, 18.4), ("Johannesburg", -26.2, 28.0), ("Kinshasa", -4.3, 15.3),
    ("Khartoum", 15.6, 32.5), ("Mogadishu", 2.1, 45.3), ("Dakar", 14.7, -17.5), ("Abuja", 9.1, 7.5),
    ("Fed", 38.9, -77.0), ("Congress", 38.9, -77.0), ("Senate", 38.9, -77.0), ("Silicon Valley", 37.4, -122.0),
    ("NASA", 28.6, -80.6), ("IMF", 38.9, -77.0), ("World Bank", 38.9, -77.0), ("UN", 40.7, -74.0),
];

pub struct Rss;

impl Source for Rss {
    fn name(&self) -> &'static str {
        "rss"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(1800)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let results = futures::future::join_all(FEEDS.iter().map(|(url, name)| async move {
                let body = ctx.http.get(*url).send().await?.text().await?;
                Ok::<_, crate::error::HubError>((parse_items(&body), *name))
            }))
            .await;
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for r in results.into_iter().flatten() {
                let (items, feed) = r;
                for item in items {
                    if out.len() >= MAX_SIGNALS {
                        return Ok(out);
                    }
                    let key: String = item.title.chars().take(40).collect::<String>().to_lowercase();
                    if key.is_empty() || !seen.insert(key.clone()) {
                        continue;
                    }
                    let Some((lat, lon)) = geotag(&item.title).or_else(|| feed_fallback(feed))
                    else {
                        continue; // no geo anchor → not a radar signal
                    };
                    let t = item
                        .date
                        .as_deref()
                        .and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok())
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_else(chrono::Utc::now);
                    out.push(
                        Signal::new(
                            "news",
                            item.title.chars().take(100).collect::<String>(),
                            lat,
                            lon,
                            format!("rss:{}", crate::auth::hash_key(&format!("{feed}:{key}"))),
                        )
                        .occurred(t)
                        .payload(serde_json::json!({
                            "feed": feed, "url": item.url, "geo_precision": "country",
                        })),
                    );
                }
            }
            Ok(out)
        }
        .boxed()
    }
}

fn geotag(title: &str) -> Option<(f64, f64)> {
    GEO_KEYWORDS
        .iter()
        .find(|(kw, _, _)| title.contains(kw))
        .map(|(_, lat, lon)| (*lat, *lon))
}

fn feed_fallback(feed: &str) -> Option<(f64, f64)> {
    FEED_FALLBACKS
        .iter()
        .find(|(n, _, _)| *n == feed)
        .map(|(_, lat, lon)| (*lat, *lon))
}

pub struct Item {
    pub title: String,
    pub url: Option<String>,
    pub date: Option<String>,
}

/// Minimal tolerant RSS/Atom item extractor: locates <item>/<entry> blocks in
/// the full document, pulls title/link/pubDate with CDATA handling and entity
/// decoding. Deliberately NOT regex-on-everything like Crucix, and no new dep.
pub fn parse_items(xml: &str) -> Vec<Item> {
    let mut out = Vec::new();
    for tag in ["item", "entry"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        let mut pos = 0usize;
        while let Some(i) = xml[pos..].find(&open) {
            let start = pos + i;
            // <itemFoo> must not match <item>: next char after the tag name
            // must be '>' or whitespace.
            let after = start + open.len();
            match xml.as_bytes().get(after) {
                Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') => {}
                _ => {
                    pos = after;
                    continue;
                }
            }
            let Some(gt) = xml[after..].find('>') else { break };
            let body_start = after + gt + 1;
            let Some(j) = xml[body_start..].find(&close) else { break };
            let body = &xml[body_start..body_start + j];
            pos = body_start + j + close.len();

            let title = extract_tag(body, "title").unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let url = extract_tag(body, "link").or_else(|| {
                // Atom: <link href="..."/>
                body.split("href=\"")
                    .nth(1)
                    .and_then(|s| s.split('\"').next())
                    .map(|s| s.to_string())
            });
            let date = extract_tag(body, "pubDate")
                .or_else(|| extract_tag(body, "updated"))
                .or_else(|| extract_tag(body, "published"));
            out.push(Item { title, url, date });
        }
    }
    out
}

fn extract_tag(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = body.find(&open)?;
    let after_open = body[start..].find('>')? + start + 1;
    let close = format!("</{tag}>");
    let end = body[after_open..].find(&close)? + after_open;
    let mut text = body[after_open..end].trim().to_string();
    // CDATA unwrap
    if text.starts_with("<![CDATA[") && text.ends_with("]]>") {
        text = text[9..text.len() - 3].to_string();
    }
    Some(entity_decode(&text))
}

fn entity_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_items_with_cdata() {
        let xml = r#"<?xml version="1.0"?><rss><channel>
            <item><title><![CDATA[Strike hits Kyiv &amp; region]]></title>
            <link>https://example.com/1</link><pubDate>Wed, 10 Sep 2026 06:00:00 GMT</pubDate></item>
            <item><title>Markets rally</title><link>https://example.com/2</link></item>
        </channel></rss>"#;
        let items = parse_items(xml);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Strike hits Kyiv & region");
        assert_eq!(items[0].url.as_deref(), Some("https://example.com/1"));
        assert!(items[0].date.is_some());
    }

    #[test]
    fn parses_atom_entries() {
        let xml = r#"<feed><entry><title>Test entry</title><link href="https://a.b/c"/><updated>2026-09-10T06:00:00Z</updated></entry></feed>"#;
        let items = parse_items(xml);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url.as_deref(), Some("https://a.b/c"));
    }

    #[test]
    fn geotag_is_deterministic_no_jitter() {
        let a = geotag("Missile strike reported in Ukraine").unwrap();
        let b = geotag("Missile strike reported in Ukraine").unwrap();
        assert_eq!(a, b); // the whole point: stable coordinates
        assert_eq!(a, (49.0, 32.0));
        assert!(geotag("quarterly earnings report").is_none());
    }
}
