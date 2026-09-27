//! GitGuardian Public API (https://api.gitguardian.com/v1/incidents/
//! secrets) — leaked-secret detection feed. Phase 3.4 of the
//! public-API integration roadmap (`docs/superpowers/roadmaps/
//! 2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Polls GitGuardian's public incidents feed for new `TRIGGERED`
//! secret-leak incidents and emits a Signal per incident. The data
//! is sensitive (real leaked secrets) — Phase 3.4 v1 surfaces the
//! incident metadata (detector type, occurrence count, age) without
//! the leaked secret payload itself (a future 3.4.x with a paid tier
//! could enrich with payload hashes for entity-graph integration).
//!
//! ## Auth
//!
//! `HUB_GITGUARDIAN_API_KEY` (env-gated, paid tier). Without key →
//! source NOT registered; sp6 reports `shelved-by-design`. User
//! signup at https://dashboard.gitguardian.com.
//!
//! ## Severity ladder
//!
//! - `status == "TRIGGERED"` AND `occurrences_count >= 5` (broadly
//!   leaked, multiple repos affected) → **priority**.
//! - `status == "TRIGGERED"` (any occurrences) → routine (still
//!   actionable).
//! - `status == "IGNORED"` → info (the team has acknowledged but
//!   chosen not to fix).
//! - else → info (ambient).
//!
//! ## external_id
//!
//! `gitguardian:{id}` — server-assigned auto-increment, stable
//! across re-polls. geo_events idempotent dedup.
//!
//! ## Cadence
//!
//! 24h. Default for OSINT monitors per roadmap §3.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.gitguardian.com/v1/incidents/secrets";

/// Env-var name for the GitGuardian API key (paid tier).
const ENV_KEY: &str = "HUB_GITGUARDIAN_API_KEY";

/// 24h cadence — OSINT monitor default per roadmap §3.
const INTERVAL_SECS: u64 = 24 * 3600;

/// Cap on signals emitted per sweep. Per_page=100 gives 100
/// incidents per sweep; we cap at 50 to keep geo_events manageable.
const TOP_N: usize = 50;

pub struct GitGuardian;

impl Source for GitGuardian {
    fn name(&self) -> &'static str {
        "gitguardian"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(api_key) = api_key() else {
                tracing::warn!(
                    "gitguardian: no {ENV_KEY} - collector not registered; sp6 will report 'shelved-by-design' until signup at https://dashboard.gitguardian.com"
                );
                return Ok(Vec::new());
            };
            let resp = ctx
                .http
                .get(BASE_URL)
                .bearer_auth(&api_key)
                .query(&[("per_page", "100"), ("order_by", "-triggered_at")])
                .send()
                .await
                .map_err(|e| crate::error::HubError::sensor(format!("gitguardian http: {e}")))?;
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "gitguardian non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| crate::error::HubError::sensor(format!("gitguardian json: {e}")))?;
            let mut out = parse_incidents(&body);
            out.truncate(TOP_N);
            Ok(out)
        }
        .boxed()
    }
}

/// Read the API key from env at sweep time.
pub fn api_key() -> Option<String> {
    std::env::var(ENV_KEY).ok().filter(|v| !v.trim().is_empty())
}

/// Parse the incidents array. Pure function for tests.
fn parse_incidents(j: &serde_json::Value) -> Vec<Signal> {
    // GitGuardian returns a JSON array directly (not wrapped in an
    // envelope). Be defensive in case the API changes shape.
    let arr = if j.is_array() {
        j.as_array().unwrap()
    } else if let Some(a) = j.get("data").and_then(|v| v.as_array()) {
        a
    } else if let Some(a) = j.get("incidents").and_then(|v| v.as_array()) {
        a
    } else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for inc in arr {
        let id = inc.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        if id == 0 {
            continue;
        }
        let detector_name = inc
            .get("detector")
            .and_then(|d| d.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let detector_display = inc
            .get("detector")
            .and_then(|d| d.get("display_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let detector_family = inc
            .get("detector")
            .and_then(|d| d.get("family"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let detector_category = inc
            .get("detector")
            .and_then(|d| d.get("category"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let date = inc.get("date").and_then(|v| v.as_str()).unwrap_or("");
        let occurrences = inc
            .get("occurrences_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let status = inc.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let triggered_at = inc
            .get("triggered_at")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (kind, severity) = classify(status, occurrences);
        let title = build_title(id, detector_display, detector_name, status, occurrences);
        out.push(
            Signal::new(kind, title, 0.0, 0.0, format!("gitguardian:{id}"))
                .severity(severity)
                .payload(serde_json::json!({
                    "id": id,
                    "detector_name": detector_name,
                    "detector_display": detector_display,
                    "detector_family": detector_family,
                    "detector_category": detector_category,
                    "date": date,
                    "occurrences_count": occurrences,
                    "status": status,
                    "triggered_at": triggered_at,
                    "extreme_type": kind,
                })),
        );
    }
    out
}

fn classify(status: &str, occurrences: i64) -> (&'static str, &'static str) {
    if status == "TRIGGERED" && occurrences >= 5 {
        return ("secret_leak_broad", "priority");
    }
    if status == "TRIGGERED" {
        return ("secret_leak_triggered", "routine");
    }
    if status == "IGNORED" {
        return ("secret_leak_ignored", "info");
    }
    ("secret_leak_info", "info")
}

fn build_title(
    id: i64,
    detector_display: &str,
    detector_name: &str,
    status: &str,
    occurrences: i64,
) -> String {
    let det = if !detector_display.is_empty() {
        detector_display
    } else if !detector_name.is_empty() {
        detector_name
    } else {
        "unknown detector"
    };
    format!("#{id} {det} {status} occurrences={occurrences}")
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_triggered_broad() -> serde_json::Value {
        serde_json::json!([{
            "id": 37665997,
            "detector": {
                "name": "json_web_token",
                "display_name": "JSON Web Token",
                "nature": "generic",
                "family": "other",
                "category": "other"
            },
            "date": "2025-01-02T07:26:25Z",
            "secret_id": 39860631,
            "occurrences_count": 7,
            "status": "TRIGGERED",
            "triggered_at": "2026-09-27T11:10:03.711156Z"
        }])
    }

    fn sample_triggered_small() -> serde_json::Value {
        serde_json::json!([{
            "id": 12345,
            "detector": {"name": "aws", "display_name": "AWS API Key"},
            "occurrences_count": 2,
            "status": "TRIGGERED",
            "triggered_at": "2026-09-26T08:00:00Z"
        }])
    }

    fn sample_ignored() -> serde_json::Value {
        serde_json::json!([{
            "id": 99999,
            "detector": {"name": "slack", "display_name": "Slack Token"},
            "occurrences_count": 1,
            "status": "IGNORED",
            "triggered_at": "2026-09-25T08:00:00Z"
        }])
    }

    /// TRIGGERED + occurrences >= 5 → priority
    #[test]
    fn triggered_broad_is_priority() {
        let out = parse_incidents(&sample_triggered_broad());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "priority");
        assert_eq!(out[0].kind, "secret_leak_broad");
        assert!(out[0].title.contains("JSON Web Token"));
        assert!(out[0].title.contains("occurrences=7"));
    }

    /// TRIGGERED + occurrences < 5 → routine
    #[test]
    fn triggered_small_is_routine() {
        let out = parse_incidents(&sample_triggered_small());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "routine");
        assert_eq!(out[0].kind, "secret_leak_triggered");
    }

    /// IGNORED → info
    #[test]
    fn ignored_is_info() {
        let out = parse_incidents(&sample_ignored());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "info");
        assert_eq!(out[0].kind, "secret_leak_ignored");
    }

    /// external_id = gitguardian:{id}
    #[test]
    fn external_id_shape() {
        let out = parse_incidents(&sample_triggered_broad());
        assert_eq!(out[0].external_id, "gitguardian:37665997");
    }

    /// Records missing id are skipped
    #[test]
    fn skips_no_id() {
        let j = serde_json::json!([{
            "detector": {"name": "x"},
            "occurrences_count": 1,
            "status": "TRIGGERED"
        }]);
        let out = parse_incidents(&j);
        assert!(out.is_empty());
    }

    /// Wrapped envelope shape (alternative API response shape)
    #[test]
    fn handles_wrapped_envelope() {
        let j = serde_json::json!({
            "data": [{
                "id": 42,
                "detector": {"name": "github", "display_name": "GitHub Token"},
                "occurrences_count": 1,
                "status": "TRIGGERED"
            }]
        });
        let out = parse_incidents(&j);
        assert_eq!(out.len(), 1);
    }

    /// Body without incidents array → empty
    #[test]
    fn handles_missing_incidents() {
        let j = serde_json::json!({"error": "upstream boom"});
        let out = parse_incidents(&j);
        assert!(out.is_empty());
    }

    /// Payload retains key fields
    #[test]
    fn payload_carries_incident_fields() {
        let out = parse_incidents(&sample_triggered_broad());
        let p = &out[0].payload;
        assert_eq!(p["id"], 37665997);
        assert_eq!(p["detector_name"], "json_web_token");
        assert_eq!(p["detector_display"], "JSON Web Token");
        assert_eq!(p["occurrences_count"], 7);
        assert_eq!(p["status"], "TRIGGERED");
    }
}