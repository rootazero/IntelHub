//! ReliefWeb (UN OCHA) humanitarian reports — disasters & crises with
//! country tags. Free POST API. Hard-won lessons (live-probed 2026-09):
//! v1 is DECOMMISSIONED (410) and v2 enforces APPROVED appnames only —
//! even Crucix's own is rejected now. Register a free appname at
//! https://apidoc.reliefweb.int/parameters#appname → RELIEFWEB_APPNAME in
//! secrets.env. Until then the source stays visibly degraded.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};
use super::rss;

const URL: &str = "https://api.reliefweb.int/v2/reports";

pub struct ReliefWeb;

impl Source for ReliefWeb {
    fn name(&self) -> &'static str {
        "reliefweb"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(1800)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(appname) = ctx.config.monitor_reliefweb_appname.clone() else {
                return Err(HubError::internal(
                    "RELIEFWEB_APPNAME not configured — ReliefWeb API v2 requires an APPROVED \
                     appname; register free at https://apidoc.reliefweb.int/parameters#appname \
                     then add RELIEFWEB_APPNAME to secrets.env",
                ));
            };
            let url = format!("{URL}?appname={appname}");
            let body = serde_json::json!({
                "limit": 25,
                "fields": { "include": [
                    "title", "date.created", "country.name",
                    "disaster_type.name", "url_alias",
                ]},
                "sort": ["date.created:desc"],
            });
            let resp = ctx.http.post(&url).json(&body).send().await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "ReliefWeb HTTP {} (403 = appname '{appname}' not approved — register at \
                     apidoc.reliefweb.int/parameters#appname)",
                    resp.status()
                )));
            }
            let j: serde_json::Value = resp.json().await?;
            let mut out = Vec::new();
            for item in j.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default() {
                let f = item.get("fields").cloned().unwrap_or_default();
                let title = f.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
                if title.is_empty() {
                    continue;
                }
                let countries: Vec<String> = f
                    .get("country")
                    .and_then(|c| c.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let dtypes: Vec<String> = f
                    .get("disaster_type")
                    .and_then(|c| c.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                // No coordinates in the reports payload — geotag title+countries,
                // skip honestly when nothing matches (better than HQ-dumping).
                let hay = format!("{} {}", title, countries.join(" "));
                let Some((lat, lon)) = rss::geotag(&hay) else { continue };
                let joined = dtypes.join(" ").to_lowercase();
                let kind: &'static str = if joined.contains("epidemic") {
                    "health"
                } else if joined.contains("complex emergency") {
                    "conflict"
                } else {
                    "disaster"
                };
                let severity: &'static str = if joined.contains("earthquake")
                    || joined.contains("tsunami")
                    || joined.contains("epidemic")
                    || joined.contains("complex emergency")
                {
                    "priority"
                } else {
                    "routine"
                };
                let occurred_at = f
                    .get("date")
                    .and_then(|d| d.get("created"))
                    .and_then(|c| c.as_str())
                    .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);
                let ext = item
                    .get("id")
                    .and_then(|i| i.as_str().map(String::from).or_else(|| i.as_i64().map(|n| n.to_string())))
                    .map(|id| format!("reliefweb:{id}"))
                    .unwrap_or_else(|| {
                        use sha2::Digest;
                        let h = format!("{:x}", sha2::Sha256::digest(title.as_bytes()));
                        format!("reliefweb:{}", &h[..16])
                    });
                out.push(
                    Signal::new(kind, truncate(&title, 140), lat, lon, ext)
                        .severity(severity)
                        .occurred(occurred_at)
                        .payload(serde_json::json!({
                            "countries": countries,
                            "disaster_types": dtypes,
                            "url": f.get("url_alias").and_then(|u| u.as_str()),
                        })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}
