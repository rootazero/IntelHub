//! ACLED conflict events (7-day window, global). Auth = OAuth password
//! grant (the official current scheme per acleddata.com/api-documentation —
//! token 24h, cached process-wide). Hard-won lessons:
//!   1. HTTP 200 with body status != 200 is still an error (double-check).
//!   2. Cloudflare error 1010 bans bot-signature clients — the shared
//!      monitor client carries a browser UA; a 403 here means network-level
//!      blocking, NOT bad credentials.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const TOKEN_URL: &str = "https://acleddata.com/oauth/token";
const API_URL: &str = "https://acleddata.com/api/acled/read";
const TOKEN_TTL_S: u64 = 23 * 3600;

pub struct Acled;

impl Source for Acled {
    fn name(&self) -> &'static str {
        "acled"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let token = acled_token(ctx).await?;
            let end = chrono::Utc::now().format("%Y-%m-%d");
            let start = (chrono::Utc::now() - chrono::Duration::days(7)).format("%Y-%m-%d");
            let url = format!(
                "{API_URL}?_format=json&limit=2000&event_date={start}|{end}&event_date_where=BETWEEN"
            );
            let resp = ctx
                .http
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await?;
            if resp.status().as_u16() == 403 {
                return Err(HubError::internal(
                    "ACLED 403 — Cloudflare egress block (error 1010 bans bot-signature clients/IPs; \
                     check proxy routing for acleddata.com) or expired token",
                ));
            }
            if !resp.status().is_success() {
                return Err(HubError::internal(format!("ACLED HTTP {}", resp.status())));
            }
            let j: serde_json::Value = resp.json().await?;
            check_body(&j)?;
            let mut out = Vec::new();
            let Some(data) = j.get("data").and_then(|d| d.as_array()) else {
                return Ok(out);
            };
            for ev in data {
                let lat = f(ev, &["latitude"]);
                let lon = f(ev, &["longitude"]);
                let (Some(lat), Some(lon)) = (lat, lon) else { continue };
                let id = s(ev, &["event_id_cnty", "event_id", "data_id"]);
                if id.is_empty() {
                    continue;
                }
                let fatalities = f(ev, &["fatalities"]).unwrap_or(0.0);
                let sev = if fatalities >= 10.0 {
                    "priority"
                } else if fatalities > 0.0 {
                    "routine"
                } else {
                    "info"
                };
                let event_type = s(ev, &["event_type"]);
                let sub_type = s(ev, &["sub_event_type"]);
                let country = s(ev, &["country"]);
                let location = s(ev, &["location"]);
                let t = s(ev, &["event_date"]);
                let occurred = chrono::NaiveDate::parse_from_str(&t, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|n| chrono::DateTime::from_naive_utc_and_offset(n, chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);
                let title = if fatalities > 0.0 {
                    format!("{event_type}/{sub_type} — {location}, {country} ({fatalities:.0} fatalities)")
                } else {
                    format!("{event_type}/{sub_type} — {location}, {country}")
                };
                out.push(
                    Signal::new("conflict", title, lat, lon, id)
                        .severity(sev)
                        .occurred(occurred)
                        .payload(ev.clone()),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Lesson #1: ACLED answers HTTP 200 with an error body — double-check.
fn check_body(j: &serde_json::Value) -> Result<()> {
    let status = j.get("status").and_then(|s| s.as_i64()).unwrap_or(200);
    if status == 200 {
        return Ok(());
    }
    let msg = j
        .get("message")
        .or_else(|| j.get("error"))
        .and_then(|m| m.as_str())
        .unwrap_or("unknown");
    if status == 403 {
        return Err(HubError::internal(format!(
            "ACLED body-status 403: {msg} — invalid/expired API key or unaccepted ToS"
        )));
    }
    Err(HubError::internal(format!("ACLED body-status {status}: {msg}")))
}
/// OAuth password grant (official scheme): token valid 24h, cached
/// process-wide for 23h. `scope=authenticated` is REQUIRED (undocumented in
/// older integrations — its absence contributes to 403s).
async fn acled_token(ctx: &Ctx) -> Result<String> {
    static CACHE: tokio::sync::OnceCell<tokio::sync::Mutex<Option<(String, std::time::Instant)>>> =
        tokio::sync::OnceCell::const_new();
    let cell = CACHE.get_or_init(|| async { tokio::sync::Mutex::new(None) }).await;
    {
        let guard = cell.lock().await;
        if let Some((tok, at)) = guard.as_ref() {
            if at.elapsed() < Duration::from_secs(TOKEN_TTL_S) {
                return Ok(tok.clone());
            }
        }
    }
    let email = ctx.config.monitor_acled_email.clone().unwrap_or_default();
    let password = ctx.config.monitor_acled_password.clone().unwrap_or_default();
    if email.is_empty() || password.is_empty() {
        return Err(HubError::internal(
            "ACLED_EMAIL/ACLED_PASSWORD not configured (secrets.env) — source degraded by design",
        ));
    }
    let resp = ctx
        .http
        .post(TOKEN_URL)
        .form(&[
            ("username", email.as_str()),
            ("password", password.as_str()),
            ("grant_type", "password"),
            ("client_id", "acled"),
            ("scope", "authenticated"),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(HubError::internal(format!(
            "ACLED oauth HTTP {} (Cloudflare 1010 = egress/UA block, not credentials)",
            resp.status()
        )));
    }
    let j: serde_json::Value = resp.json().await?;
    check_body(&j)?;
    let token = j
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| HubError::internal("ACLED oauth: no access_token in response"))?
        .to_string();
    *cell.lock().await = Some((token.clone(), std::time::Instant::now()));
    Ok(token)
}

fn s(v: &serde_json::Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(x) = v.get(k).and_then(|x| x.as_str()) {
            return x.to_string();
        }
        if let Some(n) = v.get(k).and_then(|x| x.as_i64()) {
            return n.to_string();
        }
    }
    String::new()
}

fn f(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(n) = v.get(k).and_then(|x| x.as_f64()) {
            return Some(n);
        }
        if let Some(n) = v.get(k).and_then(|x| x.as_str()).and_then(|s| s.parse().ok()) {
            return Some(n);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn body_200_but_error_is_an_error() {
        let j = json!({"status": 403, "message": "Terms of Service must be accepted"});
        let err = check_body(&j).unwrap_err().to_string();
        assert!(err.contains("ToS") || err.contains("403"));
    }

    #[test]
    fn body_ok_passes() {
        assert!(check_body(&json!({"status": 200, "count": 0, "data": []})).is_ok());
        // ACLED success bodies sometimes omit status entirely
        assert!(check_body(&json!({"count": 3, "data": []})).is_ok());
    }

    #[test]
    fn numeric_strings_parse() {
        let ev = json!({"latitude": "34.5", "longitude": "44.2", "fatalities": "12"});
        assert_eq!(f(&ev, &["latitude"]), Some(34.5));
        assert_eq!(f(&ev, &["fatalities"]), Some(12.0));
    }
}
