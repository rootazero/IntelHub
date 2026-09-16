//! urlscan.io — live URL scanning search feed.
//!
//! Polls urlscan's public search API for scans matching a default OSINT
//! watchlist (phishing/credential-harvest domains are NOT included by
//! default — the focus is independent-OSINT submissions from researchers,
//! NGOs, and security journalists). Emits a Signal per scan carrying
//! verdict, page metadata (title, server, ASN), and the live screenshot URL.
//!
//! - Free public endpoint: `/api/v1/search/` — keyless, but rate-limited to
//!   ~100 req/day unauthenticated. With an API key (free registration at
//!   urlscan.io/user/signup) the ceiling rises to 5,000/day.
//! - Built-in query: `task.url:osint OR page.domain:osint-framework` —
//!   tuned to surface independent-OSINT activity. Override via
//!   `HUB_URLSCAN_QUERY`.
//! - Kind: "cyber" (matches OTX/cisakev/nvd cluster on radar).
//!
//! Anchored at urlscan.io (operated by a Berlin-based team). Distinct from
//! DC (cisakev/nvd/ofac/opensanctions), Singapore (etherscan), Mountain
//! View (osv), and San Mateo (otx).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://urlscan.io/api/v1";
/// urlscan.io is operated by a Berlin-based team (Chaos Computer Club
/// founders). Distinct from existing anchors.
const BERLIN: (f64, f64) = (52.5200, 13.4050);

/// Default query — broad OSINT-ecosystem surveillance that catches a steady
/// stream of new independent-OSINT scan submissions without flooding with
/// routine phishing noise. Adjustable via env.
const DEFAULT_QUERY: &str =
    "task.url:osint-framework OR task.url:bellingcat OR page.domain:nitter.net OR page.domain:ddosecrets";

const MAX_SCANS_PER_SWEEP: usize = 50;

pub struct Urlscan;

impl Source for Urlscan {
    fn name(&self) -> &'static str {
        "urlscan"
    }
    fn interval(&self) -> Duration {
        // 4h cadence — enough to surface new scan submissions within a day.
        Duration::from_secs(4 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let query = if ctx.config.monitor_urlscan_query.is_empty() {
                DEFAULT_QUERY.to_string()
            } else {
                ctx.config.monitor_urlscan_query.clone()
            };
            let api_key = ctx
                .config
                .monitor_urlscan_api_key
                .clone()
                .unwrap_or_default();

            // `size` is the per-page scan count; `q` is the URL-encoded query.
            let url = format!(
                "{API_BASE}/search/?q={q}&size={n}",
                q = urlencoded(&query),
                n = MAX_SCANS_PER_SWEEP,
            );

            let mut req = ctx.http.get(&url).header("User-Agent", "intelhub-monitor/1.0");
            if !api_key.is_empty() {
                req = req.header("API-Key", api_key);
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(target: "monitor::urlscan", "fetch: {e}");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                // 429 is the common case (free tier day-cap reached).
                // Self-degrade without failing the sweep.
                tracing::warn!(target: "monitor::urlscan",
                    "HTTP {} (likely rate-limit); collector self-degraded",
                    resp.status());
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(target: "monitor::urlscan", "parse: {e}");
                    return Ok(Vec::new());
                }
            };
            let results = body
                .get("results")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();

            let mut out = Vec::new();
            for scan in results.into_iter().take(MAX_SCANS_PER_SWEEP) {
                let parsed = match parse_scan(&scan) {
                    Some(p) => p,
                    None => continue,
                };

                let title = format!(
                    "urlscan: {} ({}) — {}",
                    parsed.page_domain,
                    parsed.verdict_short,
                    parsed.title.chars().take(80).collect::<String>(),
                );

                out.push(
                    Signal::new(
                        "cyber",
                        title,
                        BERLIN.0,
                        BERLIN.1,
                        format!("urlscan:{}", parsed.uuid),
                    )
                    .severity(parsed.severity)
                    .occurred(parsed.scan_time)
                    .payload(serde_json::json!({
                        "uuid": parsed.uuid,
                        "task_url": parsed.task_url,
                        "page_domain": parsed.page_domain,
                        "page_url": parsed.page_url,
                        "title": parsed.title,
                        "server": parsed.server,
                        "asn": parsed.asn,
                        "country": parsed.country,
                        "verdict": parsed.verdict,
                        "screenshot_url": parsed.screenshot_url,
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

#[derive(Debug, Default)]
struct ParsedScan {
    uuid: String,
    task_url: String,
    page_url: String,
    page_domain: String,
    title: String,
    server: String,
    asn: String,
    country: String,
    verdict: String,
    verdict_short: String,
    severity: &'static str,
    scan_time: chrono::DateTime<chrono::Utc>,
    screenshot_url: String,
}

fn parse_scan(scan: &serde_json::Value) -> Option<ParsedScan> {
    let uuid = scan.get("_id").and_then(|x| x.as_str())?.to_string();
    if uuid.is_empty() {
        return None;
    }
    let task_url = scan
        .get("task")
        .and_then(|t| t.get("url"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let page = scan.get("page");
    let page_url = page
        .and_then(|p| p.get("url"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let page_domain = page
        .and_then(|p| p.get("domain"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let title = page
        .and_then(|p| p.get("title"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let server = scan
        .get("server")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let asn = scan
        .get("asn")
        .and_then(|x| x.as_str())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .unwrap_or_default();
    let country = scan
        .get("country")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let verdict = scan
        .get("verdicts")
        .and_then(|v| v.get("overall"))
        .and_then(|o| o.get("verdict"))
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let verdict_short = verdict_short_label(&verdict);
    let severity = severity_from_verdict(&verdict);

    let scan_time = scan
        .get("task")
        .and_then(|t| t.get("time"))
        .and_then(|x| x.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let screenshot_url = scan
        .get("screenshot")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    Some(ParsedScan {
        uuid,
        task_url,
        page_url,
        page_domain,
        title,
        server,
        asn,
        country,
        verdict,
        verdict_short,
        severity,
        scan_time,
        screenshot_url,
    })
}

fn verdict_short_label(verdict: &str) -> String {
    match verdict {
        "malicious" => "malicious".to_string(),
        "suspicious" => "suspicious".to_string(),
        "benign" => "benign".to_string(),
        _ => "?".to_string(),
    }
}

fn severity_from_verdict(verdict: &str) -> &'static str {
    match verdict {
        "malicious" => "flash",
        "suspicious" => "priority",
        _ => "routine",
    }
}

/// Minimal URL-encoder for the urlscan query (avoids pulling in a crate).
/// Encodes the bytes that are NOT safe in urlscan query strings.
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b':'
            | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scan() {
        let s = serde_json::json!({
            "_id": "0123abcd",
            "task": {"url": "https://osint.example/post", "time": "2026-09-13T12:00:00Z"},
            "page": {"url": "https://osint.example/post", "domain": "osint.example", "title": "OSINT writeup"},
            "server": "nginx/1.25",
            "asn": "AS13335, US",
            "country": "US",
            "verdicts": {"overall": {"verdict": "benign"}},
            "screenshot": "https://urlscan.io/screenshots/0123abcd.png"
        });
        let p = parse_scan(&s).unwrap();
        assert_eq!(p.uuid, "0123abcd");
        assert_eq!(p.page_domain, "osint.example");
        assert_eq!(p.verdict_short, "benign");
        assert_eq!(p.severity, "routine");
        assert_eq!(p.asn, "AS13335");
    }

    #[test]
    fn urlencoded_escapes_spaces() {
        assert_eq!(urlencoded("a b"), "a%20b");
        assert_eq!(urlencoded("osint-framework.com/"), "osint-framework.com%2F");
    }

    #[test]
    fn severity_scales() {
        assert_eq!(severity_from_verdict("malicious"), "flash");
        assert_eq!(severity_from_verdict("suspicious"), "priority");
        assert_eq!(severity_from_verdict("benign"), "routine");
        assert_eq!(severity_from_verdict("unknown"), "routine");
    }

    #[test]
    fn rejects_empty_uuid() {
        let s = serde_json::json!({"_id": "", "task": {"url": "x"}});
        assert!(parse_scan(&s).is_none());
    }
}