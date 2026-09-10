//! USAspending — federal contract awards (defense/military keywords, 30d,
//! top by amount). Free POST API, no key. Ported from Crucix
//! `sources/usaspending.mjs`. Geo: contract descriptions often name the
//! place of performance → shared keyword geotag; unmatched awards anchor
//! at the Pentagon (agency-side fallback, same honesty pattern as KEV).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};
use super::rss;

const URL: &str = "https://api.usaspending.gov/api/v2/search/spending_by_award/";
const PENTAGON: (f64, f64) = (38.8719, -77.0563);

pub struct UsaSpending;

impl Source for UsaSpending {
    fn name(&self) -> &'static str {
        "usaspending"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let end = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let start = (chrono::Utc::now() - chrono::Duration::days(30)).format("%Y-%m-%d").to_string();
            let body = serde_json::json!({
                "filters": {
                    "keywords": ["defense", "military"],
                    "time_period": [{ "start_date": start, "end_date": end }],
                    "award_type_codes": ["A", "B", "C", "D"],
                },
                "fields": [
                    "Award ID", "Recipient Name", "Award Amount", "Description",
                    "Awarding Agency", "Start Date", "Award Type",
                ],
                "limit": 10,
                "page": 1,
                "sort": "Award Amount",
                "order": "desc",
            });
            let resp = ctx.http.post(URL).json(&body).send().await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "USAspending HTTP {}",
                    resp.status()
                )));
            }
            let j: serde_json::Value = resp.json().await?;
            let mut out = Vec::new();
            for r in j.get("results").and_then(|a| a.as_array()).cloned().unwrap_or_default() {
                let award_id = r.get("Award ID").and_then(|v| v.as_str()).unwrap_or("");
                if award_id.is_empty() {
                    continue;
                }
                let recipient = r.get("Recipient Name").and_then(|v| v.as_str()).unwrap_or("");
                let agency = r.get("Awarding Agency").and_then(|v| v.as_str()).unwrap_or("");
                let desc = r.get("Description").and_then(|v| v.as_str()).unwrap_or("");
                let amount = r.get("Award Amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let title = format!(
                    "${} {} → {}: {}",
                    fmt_amount(amount),
                    short_agency(agency),
                    recipient,
                    desc.chars().take(90).collect::<String>()
                );
                let (lat, lon) = rss::geotag(desc).unwrap_or(PENTAGON);
                let severity: &'static str = if amount >= 100_000_000.0 {
                    "priority"
                } else {
                    "routine"
                };
                let occurred_at = r
                    .get("Start Date")
                    .and_then(|v| v.as_str())
                    .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|d| chrono::DateTime::from_naive_utc_and_offset(d, chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);
                out.push(
                    Signal::new("economic", title, lat, lon, format!("usasp:{award_id}"))
                        .severity(severity)
                        .occurred(occurred_at)
                        .payload(serde_json::json!({
                            "award_id": award_id,
                            "recipient": recipient,
                            "agency": agency,
                            "amount_usd": amount,
                            "award_type": r.get("Award Type").and_then(|v| v.as_str()),
                        })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}

fn fmt_amount(v: f64) -> String {
    if v >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if v >= 1e6 {
        format!("{:.0}M", v / 1e6)
    } else if v >= 1e3 {
        format!("{:.0}K", v / 1e3)
    } else {
        format!("{v:.0}")
    }
}

/// Trim the verbose "DEPARTMENT OF DEFENSE.DEPT OF THE NAVY" style names.
fn short_agency(s: &str) -> &str {
    s.split('.').next().unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_format() {
        assert_eq!(fmt_amount(1_250_000_000.0), "1.3B");
        assert_eq!(fmt_amount(250_000_000.0), "250M");
        assert_eq!(fmt_amount(48_000.0), "48K");
    }
}
