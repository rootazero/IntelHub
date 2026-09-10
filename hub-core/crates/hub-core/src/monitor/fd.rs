//! SP6B: financialdatasets.ai on-demand client (spec §5, financials_fetch).
//! Pay-per-request provider → PG-backed 30-day TTL cache with the atlas
//! semantics: a ticker is cached ONLY when every required endpoint succeeded,
//! and a cached row is valid ONLY if it still contains every required
//! endpoint key (adding endpoints to the set invalidates old rows).
//! Auth is the `X-API-KEY` header. All failures degrade to Ok(partial) or
//! Err — never panic, never cache partials.

use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::state::AppState;

/// (endpoint name, path, response wrapper key). The segments→segmented_financials
/// wrapper-name mismatch is an atlas-surveyed trap — keep the mapping explicit.
///
/// Tier reality (verified 2026-09-10 against the deployed key): CORE endpoints
/// answer on the free tier; ENRICH endpoints answer 402 Payment Required until
/// the key is upgraded. Enrichers are fetched best-effort every miss — an
/// upgraded key starts landing them automatically, no code change.
pub const CORE: &[(&str, &str, &str)] = &[
    ("company_facts", "/company/facts?ticker={t}", "company_facts"),
    ("income_statements", "/financials/income-statements?ticker={t}&period=annual&limit=5", "income_statements"),
];

pub const ENRICH: &[(&str, &str, &str)] = &[
    ("balance_sheets", "/financials/balance-sheets?ticker={t}&period=annual&limit=5", "balance_sheets"),
    ("cash_flow_statements", "/financials/cash-flow-statements?ticker={t}&period=annual&limit=5", "cash_flow_statements"),
    ("segments", "/financials/segments?ticker={t}&period=annual&limit=5", "segmented_financials"),
    ("institutional_holdings", "/institutional-holdings?ticker={t}&limit=20", "institutional_holdings"),
    ("kpi_guidance", "/kpi/guidance?ticker={t}&period=quarterly&limit=4", "kpi_guidance"),
    ("kpi_non_gaap", "/kpi/non-gaap?ticker={t}&period=quarterly&limit=4", "kpi_non_gaap"),
];

const TTL_DAYS: i64 = 30;
const BASE: &str = "https://api.financialdatasets.ai";

pub struct FdOutcome {
    pub cached: bool,
    pub fetched_at: DateTime<Utc>,
    pub brief: serde_json::Value,
}

pub async fn fetch_with_cache(state: &AppState, ticker: &str) -> Result<FdOutcome> {
    let ticker = ticker.to_uppercase();
    // ── cache read: TTL + required-endpoints completeness ──
    let row: Option<(DateTime<Utc>, serde_json::Value, serde_json::Value)> = sqlx::query_as(
        "SELECT fetched_at, endpoints, payload FROM fd_cache WHERE ticker = $1",
    )
    .bind(&ticker)
    .fetch_optional(&state.pg)
    .await?;
    if let Some((fetched_at, endpoints, payload)) = row {
        let fresh = fetched_at > Utc::now() - chrono::Duration::days(TTL_DAYS);
        // Completeness is judged on CORE only — enrichers are opportunistic.
        let complete = CORE
            .iter()
            .all(|(name, _, _)| endpoints.get(name).is_some());
        if fresh && complete {
            return Ok(FdOutcome { cached: true, fetched_at, brief: payload });
        }
    }

    // ── cache miss: fetch ALL required endpoints ──
    let key = state
        .config
        .financialdatasets_api_key
        .clone()
        .ok_or_else(|| crate::error::HubError::BadRequest("FINANCIALDATASETS_API_KEY not configured".into()))?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("intelhub-financials/1.0")
        .build()?;
    let mut endpoints = serde_json::Map::new();
    let mut failed: Vec<&str> = Vec::new();
    let mut core_failed = false;
    for (is_core, set) in [(true, CORE), (false, ENRICH)] {
        for (name, path_tmpl, wrapper) in set {
            let url = format!("{BASE}{}", path_tmpl.replace("{t}", &ticker));
            match http.get(&url).header("X-API-KEY", &key).send().await {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    match body.get(wrapper) {
                        Some(inner) => {
                            endpoints.insert(name.to_string(), inner.clone());
                        }
                        None => {
                            tracing::warn!(endpoint = name, "fd: wrapper key missing");
                            failed.push(name);
                            core_failed |= is_core;
                        }
                    }
                }
                Ok(r) if r.status() == reqwest::StatusCode::PAYMENT_REQUIRED => {
                    // Tier-gated: tolerated for enrichers, but a 402 on a CORE
                    // endpoint (e.g. empty credit balance) must block caching —
                    // otherwise an empty brief would be cached as "complete".
                    tracing::debug!(endpoint = name, "fd: tier-gated (402)");
                    failed.push(name);
                    core_failed |= is_core;
                }
                Ok(r) => {
                    tracing::warn!(endpoint = name, status = %r.status(), "fd http");
                    failed.push(name);
                    core_failed |= is_core;
                }
                Err(e) => {
                    tracing::warn!(endpoint = name, error = %e, "fd fetch");
                    failed.push(name);
                    core_failed |= is_core;
                }
            }
        }
    }

    let complete = !core_failed;
    let brief = distill(&ticker, &serde_json::Value::Object(endpoints.clone()), complete, &failed);
    let now = Utc::now();
    if complete {
        // All-or-nothing cache write (atlas: partials never poison the cache).
        sqlx::query(
            "INSERT INTO fd_cache (ticker, fetched_at, endpoints, payload) VALUES ($1,$2,$3,$4)
             ON CONFLICT (ticker) DO UPDATE SET fetched_at=$2, endpoints=$3, payload=$4",
        )
        .bind(&ticker)
        .bind(now)
        .bind(serde_json::Value::Object(endpoints))
        .bind(&brief)
        .execute(&state.pg)
        .await?;
    }
    Ok(FdOutcome { cached: false, fetched_at: now, brief })
}

/// Distill raw endpoint payloads into an agent-readable brief (atlas
/// deep_financials_brief pattern, zero extra API cost).
pub fn distill(
    ticker: &str,
    endpoints: &serde_json::Value,
    complete: bool,
    failed: &[&str],
) -> serde_json::Value {
    let facts = endpoints.get("company_facts").cloned().unwrap_or_default();
    let stmt_trend = |name: &str, fields: &[&str]| -> Vec<serde_json::Value> {
        endpoints
            .get(name)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .take(5)
                    .map(|row| {
                        let mut m = serde_json::Map::new();
                        m.insert("report_period".into(), row.get("report_period").cloned().unwrap_or_default());
                        for f in fields {
                            if let Some(v) = row.get(f) {
                                m.insert(f.to_string(), v.clone());
                            }
                        }
                        serde_json::Value::Object(m)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let holders: Vec<serde_json::Value> = endpoints
        .get("institutional_holdings")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .take(5)
                .map(|h| {
                    serde_json::json!({
                        "filer": h.get("filer_name").or_else(|| h.get("name_of_issuer")).cloned().unwrap_or_default(),
                        "value_usd": h.get("value_usd").cloned().unwrap_or_default(),
                        "report_period": h.get("report_period").cloned().unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let guidance: Vec<serde_json::Value> = endpoints
        .get("kpi_guidance")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().take(8).cloned().collect())
        .unwrap_or_default();
    let non_gaap: Vec<serde_json::Value> = endpoints
        .get("kpi_non_gaap")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().take(8).cloned().collect())
        .unwrap_or_default();

    serde_json::json!({
        "ticker": ticker,
        "complete": complete,
        "failed_endpoints": failed,
        "company": {
            "name": facts.get("name").cloned().unwrap_or_default(),
            "sector": facts.get("sector").cloned().unwrap_or_default(),
            "industry": facts.get("industry").cloned().unwrap_or_default(),
            "market_cap": facts.get("market_cap").cloned().unwrap_or_default(),
        },
        "income_trend": stmt_trend("income_statements", &["revenue", "net_income", "eps_diluted"]),
        "balance_trend": stmt_trend("balance_sheets", &["total_assets", "total_debt", "shareholders_equity"]),
        "cashflow_trend": stmt_trend("cash_flow_statements", &["operating_cash_flow", "free_cash_flow"]),
        "top_holders": holders,
        "guidance": guidance,
        "non_gaap_kpis": non_gaap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_and_enrich_split() {
        assert_eq!(CORE.len(), 2);
        assert_eq!(ENRICH.len(), 6);
        let seg = ENRICH.iter().find(|(n, _, _)| *n == "segments").unwrap();
        assert_eq!(seg.2, "segmented_financials"); // wrapper-name trap
    }

    #[test]
    fn distill_handles_missing_gracefully() {
        let eps = serde_json::json!({
            "company_facts": {"name": "Apple Inc.", "sector": "Technology"},
            "income_statements": [{"report_period": "2025-09-27", "revenue": 4.1e11, "net_income": 1.0e11}],
        });
        let brief = distill("AAPL", &eps, false, &["segments"]);
        assert_eq!(brief["company"]["name"], "Apple Inc.");
        assert_eq!(brief["income_trend"].as_array().unwrap().len(), 1);
        assert_eq!(brief["complete"], false);
        assert!(brief["top_holders"].as_array().unwrap().is_empty());
    }
}
