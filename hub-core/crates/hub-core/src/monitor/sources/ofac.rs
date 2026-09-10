//! OFAC SDN sanctions list — publication-tempo signal. The full SDN.XML is
//! ~29MB behind an S3 redirect and carries NO per-entry update dates (delta
//! endpoint returns empty), so per-entity "what's new" is impossible without
//! storing the whole list — instead we read just the header chunk and emit
//! ONE event per publication day: "list updated, N entries". Sanctions
//! tempo is itself the signal; entity-level work belongs to the graph
//! plane (batch-B later), not the map.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const SDN_URL: &str =
    "https://sanctionslistservice.ofac.treas.gov/api/PublicationPreview/exports/SDN.XML";
const TREASURY_DC: (f64, f64) = (38.8951, -77.0364);

pub struct Ofac;

impl Source for Ofac {
    fn name(&self) -> &'static str {
        "ofac"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let mut resp = ctx.http.get(SDN_URL).send().await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("OFAC SDN HTTP {}", resp.status())));
            }
            // Header (Publish_Date + Record_Count) lives in the first chunk —
            // never buffer the whole 29MB.
            let mut head = String::new();
            while head.len() < 64 * 1024 {
                match resp.chunk().await {
                    Ok(Some(c)) => {
                        head.push_str(&String::from_utf8_lossy(&c));
                        if head.contains("</publshInformation>")
                            || head.contains("</PublishInformation>")
                            || (head.contains("Publish_Date") && head.contains("Record_Count"))
                        {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => return Err(e.into()),
                }
            }
            drop(resp);
            let publish = capture(&head, "Publish_Date")
                .ok_or_else(|| HubError::sensor("OFAC SDN header: no Publish_Date"))?;
            let count = capture(&head, "Record_Count").unwrap_or_else(|| "?".into());
            // One event per publication day (dedupe on the date itself).
            let occurred = chrono::NaiveDate::parse_from_str(&publish, "%m/%d/%Y")
                .ok()
                .and_then(|d| d.and_hms_opt(12, 0, 0))
                .map(|d| chrono::DateTime::from_naive_utc_and_offset(d, chrono::Utc))
                .unwrap_or_else(chrono::Utc::now);
            Ok(vec![Signal::new(
                "sanction",
                format!("OFAC SDN list updated — {count} entries (published {publish})"),
                TREASURY_DC.0,
                TREASURY_DC.1,
                format!("ofac:list:{publish}"),
            )
            .severity("routine")
            .occurred(occurred)
            .payload(serde_json::json!({
                "list": "SDN",
                "record_count": count,
                "publish_date": publish,
                "url": "https://sanctionslistservice.ofac.treas.gov/",
            }))])
        }
        .boxed()
    }
}

fn capture(hay: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = hay.find(&open)? + open.len();
    let end = hay[start..].find(&close)? + start;
    Some(hay[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header() {
        let h = "<publshInformation><Publish_Date>09/09/2026</Publish_Date><Record_Count>12543</Record_Count></publshInformation>";
        assert_eq!(capture(h, "Publish_Date").as_deref(), Some("09/09/2026"));
        assert_eq!(capture(h, "Record_Count").as_deref(), Some("12543"));
    }
}
