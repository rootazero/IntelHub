//! WHO Disease Outbreak News feed (https://www.who.int/emergencies/
//! disease-outbreak-news). Phase 4.2 of the public-API integration
//! roadmap (`docs/superpowers/roadmaps/2026-09-27-public-api-
//! integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Polls the WHO Disease Outbreak News (DON) JSON API every 24h:
//! `GET https://www.who.int/api/emergencies/diseaseoutbreaknews?
//! $top=20&$orderby=PublicationDate desc` — returns the most
//! recent 20 DONs across the entire archive (Sitefinity OData
//! endpoint; no auth; the page at /emergencies/disease-outbreak-
//! news loads via this same JSON).
//!
//! Filters to DONs published in the past 30 days (so the same
//! articles don't re-emit every sweep). Each DON has:
//! - `Id` UUID (stable identifier)
//! - `Title` (e.g. "Ebola disease caused by Bundibugyo virus -
//!   Democratic Republic of the Congo")
//! - `PublicationDate` (ISO 8601)
//! - `Overview` (HTML body — trimmed to plain text for the payload)
//! - `ItemDefaultUrl` (relative path e.g. `/2026_09_25-en`)
//!
//! ## Compounds with...
//!
//! - Phase 4.1 `threatcluster` (incident clustering): a new WHO DON
//!   + recent threat cluster on the same disease/region in 24h =
//!   correlated signal worth surfacing.
//! - Phase 3 `acled` (already shipped) for unrest events that often
//!   coincide with disease outbreaks.
//!
//! ## Severity ladder
//!
//! - `Ebola` / `Marburg` / `H5N1` / `avian` / `H1N1` / `pandemic` /
//!   `SARS` / `MERS` / `plague` / `cholera` / `haemorrhagic` /
//!   `smallpox` / `mpox` → **priority** (high-fatality / pandemic
//!   potential)
//! - `measles` / `polio` / `dengue` / `malaria` / `typhoid` /
//!   `tuberculosis` / `COVID` → **routine** (significant but
//!   established disease pressure)
//! - else → **info** (any other DON)
//!
//! The severity ladder is keyed by title substring so we don't
//! need to know the exact disease taxonomy. Title text covers
//! "Ebola disease caused by Bundibugyo virus" (matches "ebola"),
//! "Cholera - Situation in X" (matches "cholera"), etc.
//!
//! ## external_id
//!
//! `who:{Id_first_8}` — the WHO Id is a UUID, stable across
//! re-polls, and unique per DON article. Truncated to 8 hex chars
//! (matches existing pattern in currents + helium_news).
//!
//! ## Cadence
//!
//! 24h — the OSINT monitor default per roadmap §3. WHO publishes
//! 5-15 DONs per month on average; daily sweep captures everything
//! without burning rate budget.
//!
//! ## Top-N
//!
//! 20 per sweep (matches the `$top=20` API call). Within that
//! 20, we additionally filter to past 30 days so a quiet period
//! doesn't make us emit 20 items from 2 years ago.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://www.who.int/api/emergencies/diseaseoutbreaknews";

const INTERVAL_SECS: u64 = 24 * 3600;

const TOP_N: usize = 20;

/// Cutoff: only emit DONs published in this many past days.
/// Long enough to capture laggards (WHO publishes 5-15/month,
/// so 30 days covers ~5x normal volume); short enough that we
/// don't re-emit months-old baseline items on first sweep.
const RECENT_WINDOW_DAYS: i64 = 30;

/// Priority keywords — high-fatality / pandemic-potential
/// diseases. Match is case-insensitive substring against `Title`.
const PRIORITY_KEYWORDS: &[&str] = &[
    "ebola",
    "marburg",
    "h5n1",
    "avian",
    "h1n1",
    "pandemic",
    "sars",
    "mers",
    "plague",
    "cholera",
    "haemorrhagic",
    "hemorrhagic",
    "smallpox",
    "mpox",
    "monkeypox",
    "nipah",
    "crimean-congo",
    "lassa",
];

/// Routine keywords — significant established disease pressure.
const ROUTINE_KEYWORDS: &[&str] = &[
    "measles",
    "polio",
    "dengue",
    "malaria",
    "typhoid",
    "tuberculosis",
    "covid",
    "influenza",
    "rsv",
    "pertussis",
    "leptospirosis",
    "anthrax",
    "rabies",
];

pub struct WhoDiseaseOutbreak;

impl Source for WhoDiseaseOutbreak {
    fn name(&self) -> &'static str {
        "who_disease_outbreak"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let resp = match ctx
                .http
                .get(BASE_URL)
                .query(&[
                    ("$top", &TOP_N.to_string()),
                    ("$orderby", "PublicationDate desc"),
                ])
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "who_disease_outbreak fetch failed");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "who_disease_outbreak non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "who_disease_outbreak parse failed");
                    return Ok(Vec::new());
                }
            };
            Ok(parse_response(&body))
        }
        .boxed()
    }
}

/// Parse the OData response. Expected shape:
/// `{ "@odata.context": "...", "value": [...] }`.
/// Each `value[]` entry has `Id`, `Title`, `PublicationDate`,
/// `Overview` (HTML), `ItemDefaultUrl`, etc.
fn parse_response(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(arr) = j.get("value").and_then(|v| v.as_array()) else {
        return out;
    };
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - (RECENT_WINDOW_DAYS * 86_400);
    for r in arr.iter().take(TOP_N) {
        let id = r.get("Id").and_then(|v| v.as_str()).unwrap_or("");
        let title = r.get("Title").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() || title.is_empty() {
            continue;
        }
        let publication_date = r
            .get("PublicationDate")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let overview_html = r.get("Overview").and_then(|v| v.as_str()).unwrap_or("");
        let item_url = r.get("ItemDefaultUrl").and_then(|v| v.as_str()).unwrap_or("");
        // Skip old items (already-archived baseline).
        if !publication_date.is_empty() {
            if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(publication_date) {
                if ts.timestamp() < cutoff {
                    continue;
                }
            }
        }
        // Defensive: strip HTML tags from overview for payload cleanliness.
        let overview_text = strip_html(overview_html);
        let (severity, kind) = classify_title(title);
        out.push(
            Signal::new(
                kind,
                title.to_string(),
                0.0,
                0.0,
                format!("who:{}", &id[..id.len().min(16)]),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "kind": "who_disease_outbreak",
                "don_id": id,
                "title": title,
                "publication_date": publication_date,
                "overview": overview_text,
                "item_url": format!(
                    "https://www.who.int/emergencies/disease-outbreak-news{item_url}"
                ),
                "extreme_type": kind,
            })),
        );
    }
    out
}

fn classify_title(title: &str) -> (&'static str, &'static str) {
    let lower = title.to_lowercase();
    if PRIORITY_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return ("priority", "disease_outbreak_priority");
    }
    if ROUTINE_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return ("routine", "disease_outbreak_routine");
    }
    ("info", "disease_outbreak_info")
}

/// Minimal HTML stripper for the WHO Overview field. Keeps the
/// first paragraph text; strips tags and decodes common entities.
fn strip_html(s: &str) -> String {
    // Replace tags with spaces, then collapse whitespace.
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
        }
        // inside-tag content trimmed
        _ => {}
    }
    // Decode common entities.
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    // Collapse whitespace.
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_ws = false;
    for c in out.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                collapsed.push(' ');
                prev_ws = true;
            }
        } else {
            collapsed.push(c);
            prev_ws = false;
        }
    }
    collapsed.trim().chars().take(400).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> serde_json::Value {
        // 3 entries: ebola (priority), dengue (routine), tetanus (info)
        let now = chrono::Utc::now().to_rfc3339();
        let old = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
        serde_json::json!({
            "@odata.context": "...",
            "value": [
                {
                    "Id": "5df1872c-fdb1-4f07-b54b-007046fb5b59",
                    "Title": "Ebola disease caused by Bundibugyo virus - Democratic Republic of the Congo",
                    "PublicationDate": now,
                    "Overview": "<p>20 March 2026</p><p>WHO confirms outbreak in DRC.</p>",
                    "ItemDefaultUrl": "/2026_09_25-en"
                },
                {
                    "Id": "aabbccdd-1111-2222-3333-444455556666",
                    "Title": "Dengue fever - Philippines",
                    "PublicationDate": now,
                    "Overview": "<p>Situation update.</p>",
                    "ItemDefaultUrl": "/2026_09_24-en"
                },
                {
                    "Id": "deadbeef-cafe-babe-feed-facefaceface",
                    "Title": "Tetanus - somewhere rare",
                    "PublicationDate": now,
                    "Overview": "",
                    "ItemDefaultUrl": "/2026_09_20-en"
                },
                {
                    "Id": "old-old-old-old",
                    "Title": "Ebola - 2020 archive",
                    "PublicationDate": old,
                    "Overview": "",
                    "ItemDefaultUrl": "/2020_01_01-en"
                }
            ]
        })
    }

    /// Priority keywords (ebola, cholera, etc.) → priority
    #[test]
    fn ebola_is_priority() {
        let sigs = parse_response(&sample());
        let ebola = sigs.iter().find(|s| s.title.contains("Ebola")).unwrap();
        assert_eq!(ebola.severity, "priority");
        assert_eq!(ebola.kind, "disease_outbreak_priority");
    }

    /// Routine keywords (dengue, malaria) → routine
    #[test]
    fn dengue_is_routine() {
        let sigs = parse_response(&sample());
        let dengue = sigs.iter().find(|s| s.title.contains("Dengue")).unwrap();
        assert_eq!(dengue.severity, "routine");
        assert_eq!(dengue.kind, "disease_outbreak_routine");
    }

    /// No keyword match → info
    #[test]
    fn unknown_disease_is_info() {
        let sigs = parse_response(&sample());
        let tet = sigs.iter().find(|s| s.title.contains("Tetanus")).unwrap();
        assert_eq!(tet.severity, "info");
        assert_eq!(tet.kind, "disease_outbreak_info");
    }

    /// Old items (>30 days) are filtered out
    #[test]
    fn filters_old_dons() {
        let sigs = parse_response(&sample());
        assert!(sigs.iter().all(|s| !s.title.contains("2020 archive")));
        assert_eq!(sigs.len(), 3);
    }

    /// External ID = who:{Id_first_16}
    #[test]
    fn external_id_shape() {
        let sigs = parse_response(&sample());
        assert_eq!(sigs[0].external_id, "who:5df1872c-fdb1-4f0");
    }

    /// Skip entries without ID or title (defensive)
    #[test]
    fn skips_empty_id_or_title() {
        let j = serde_json::json!({
            "value": [
                {"Id": "", "Title": "no id"},
                {"Id": "abc", "Title": ""},
                {"Id": "real", "Title": "real entry"}
            ]
        });
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].external_id, "who:real");
    }

    /// Empty value[] → empty
    #[test]
    fn empty_value_returns_empty() {
        let j = serde_json::json!({"value": []});
        assert!(parse_response(&j).is_empty());
    }

    /// Missing value key → empty
    #[test]
    fn missing_value_returns_empty() {
        let j = serde_json::json!({"@odata.context": "x"});
        assert!(parse_response(&j).is_empty());
    }

    /// Top-N cap respected
    #[test]
    fn caps_at_top_n() {
        let big: Vec<_> = (0..50)
            .map(|i| {
                serde_json::json!({
                    "Id": format!("id-{i:02}"),
                    "Title": format!("DON {i}"),
                    "PublicationDate": chrono::Utc::now().to_rfc3339(),
                    "Overview": "",
                    "ItemDefaultUrl": format!("/{i}")
                })
            })
            .collect();
        let j = serde_json::json!({"value": big});
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), TOP_N);
    }

    /// Mpox matches priority (monkeypox alias)
    #[test]
    fn mpox_is_priority() {
        let j = serde_json::json!({
            "value": [{"Id": "x", "Title": "Mpox outbreak - DRC", "PublicationDate": chrono::Utc::now().to_rfc3339()}]
        });
        let sigs = parse_response(&j);
        assert_eq!(sigs[0].severity, "priority");
    }

    /// Cholera, plague, SARS/MERS all priority
    #[test]
    fn high_fatality_keywords_priority() {
        for kw in &["cholera", "plague", "MERS", "SARS", "haemorrhagic fever"] {
            let j = serde_json::json!({
                "value": [{"Id": "x", "Title": format!("{kw} - test"), "PublicationDate": chrono::Utc::now().to_rfc3339()}]
            });
            let sigs = parse_response(&j);
            assert_eq!(
                sigs[0].severity,
                "priority",
                "expected {kw:?} to be priority"
            );
        }
    }

    /// Overview HTML stripped in payload
    #[test]
    fn overview_html_stripped() {
        let sigs = parse_response(&sample());
        let overview = sigs[0].payload["overview"].as_str().unwrap();
        assert!(!overview.contains('<'));
        assert!(overview.contains("WHO confirms"));
    }

    /// item_url built from ItemDefaultUrl
    #[test]
    fn item_url_built() {
        let sigs = parse_response(&sample());
        assert_eq!(
            sigs[0].payload["item_url"],
            "https://www.who.int/emergencies/disease-outbreak-news/2026_09_25-en"
        );
    }
}