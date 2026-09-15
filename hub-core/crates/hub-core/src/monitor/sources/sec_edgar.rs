//! SEC EDGAR — US Securities and Exchange Commission filing tracker.
//! Free full-text search via EFTS (https://efts.sec.gov/), no key —
//! but User-Agent with contact email is mandatory per SEC fair-access
//! policy. Default form filter = 8-K (current reports = material events
//! like M&A, leadership changes, restatements, going-concern flags) —
//! the most newsworthy subset of filings. Override via HUB_SEC_FORM
//! (e.g. "10-K" for annual reports, "4" for insider Form 4 transactions).
//!
//! Each sweep queries the lookback window (24h) and emits the most
//! recent MAX_PER_SWEEP filings as Signals. SEC HQ is the anchor — same
//! neighbourhood as OFAC/OpenSanctions, giving operators the visual
//! "three US-federal-tempo signals in DC" pattern. Per-filing entity
//! resolution (linking filing → company → officers) belongs to the
//! graph plane, not the Radar map.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const EFTS_URL: &str = "https://efts.sec.gov/LATEST/search-index";
const SEC_HQ: (f64, f64) = (38.8894, -77.0266); // Washington DC
const LOOKBACK_HOURS: i64 = 24;
const MAX_PER_SWEEP: usize = 20;
const DEFAULT_FORM: &str = "8-K";

pub struct SecEdgar;

impl Source for SecEdgar {
    fn name(&self) -> &'static str {
        "sec-edgar"
    }
    fn interval(&self) -> Duration {
        // 6h cadence — EFTS results surface new filings within minutes
        // of SEC acceptance; 6h captures the daily rhythm without spam.
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let form = if ctx.config.monitor_sec_form.is_empty() {
                DEFAULT_FORM.to_string()
            } else {
                ctx.config.monitor_sec_form.clone()
            };

            let now = chrono::Utc::now();
            let end_str = now.format("%Y-%m-%d").to_string();
            let start_str = (now - chrono::Duration::hours(LOOKBACK_HOURS))
                .format("%Y-%m-%d")
                .to_string();

            // Empty q= returns all filings in the date range matching the
            // form filter — SEC EDGAR's "list latest" semantics.
            let url = format!(
                "{EFTS_URL}?q=&forms={form}&dateRange=custom&startdt={start_str}&enddt={end_str}"
            );

            let mut req = ctx.http.get(&url);
            // SEC fair-access: User-Agent must include a contact email.
            // Without it, requests are blocked or rate-limited. Override
            // via HUB_SEC_USER_AGENT_EMAIL (secrets.env). Default to a
            // generic OSINT-research tag if not configured — operators
            // should set their own contact.
            let ua = ctx
                .config
                .monitor_sec_user_agent_email
                .clone()
                .unwrap_or_else(|| "IntelHub OSINT research zou@example.com".to_string());
            req = req.header("User-Agent", format!("IntelHub OSINT {ua}"));
            req = req.header("Accept", "application/json");

            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "SEC EDGAR EFTS HTTP {}",
                    resp.status()
                )));
            }
            let j: serde_json::Value = resp.json().await?;

            let hits = j
                .get("hits")
                .and_then(|h| h.get("hits"))
                .and_then(|h| h.as_array())
                .cloned()
                .unwrap_or_default();

            if hits.is_empty() {
                return Ok(Vec::new());
            }

            let mut out = Vec::new();
            for hit in hits.into_iter().take(MAX_PER_SWEEP) {
                let src = match hit.get("_source") {
                    Some(s) => s,
                    None => continue,
                };

                // CIKs can be space-separated ("320193 789012") for
                // joint filings; take the first as the primary.
                let cik = src
                    .get("ciks")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                if cik.is_empty() {
                    continue;
                }

                let display_names: Vec<String> = src
                    .get("display_names")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|n| n.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let display_name = display_names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("CIK {cik}"));

                let form_type = src
                    .get("form")
                    .and_then(|f| f.as_str())
                    .unwrap_or(&form)
                    .to_string();
                let file_date = src
                    .get("file_date")
                    .and_then(|f| f.as_str())
                    .unwrap_or("")
                    .to_string();
                let accession = src
                    .get("adsh")
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_string();
                let period_ending = src
                    .get("period_ending")
                    .and_then(|p| p.as_str())
                    .map(String::from);

                // SEC dates lack a time component — anchor at noon UTC
                // so the timestamp is stable across sweeps.
                let occurred_at = chrono::NaiveDate::parse_from_str(&file_date, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(12, 0, 0))
                    .map(|d| chrono::DateTime::from_naive_utc_and_offset(d, chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);

                let title = format!("{form_type} — {display_name} (filed {file_date})");

                out.push(
                    Signal::new(
                        "filing",
                        title,
                        SEC_HQ.0,
                        SEC_HQ.1,
                        format!("sec:{cik}:{accession}"),
                    )
                    .severity("routine")
                    .occurred(occurred_at)
                    .payload(serde_json::json!({
                        "cik": cik,
                        "form": form_type,
                        "filed_date": file_date,
                        "accession": accession,
                        "period_ending": period_ending,
                        "display_names": display_names,
                        "url": format!(
                            "https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK={cik}&type={form_type}"
                        ),
                    })),
                );
            }

            if out.is_empty() {
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}