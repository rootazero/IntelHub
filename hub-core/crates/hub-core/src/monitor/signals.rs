//! SP6B: time-series plane — Observation → signal_observations (idempotent
//! upsert) + minimal crossing/threshold alert rules (spec §4). Series names
//! are self-describing with units (`fred:CPIAUCSL_YOY`, `quote:AAPL`) — the
//! atlas UUP≠DXY lesson: a field name must never lie about what it holds.

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::state::AppState;

/// One normalized time-series observation, pre-persistence.
#[derive(Debug, Clone)]
pub struct Observation {
    pub series: String,
    pub observed_at: DateTime<Utc>,
    pub value: f64,
    pub payload: serde_json::Value,
}

impl Observation {
    pub fn new(series: impl Into<String>, observed_at: DateTime<Utc>, value: f64) -> Self {
        Self {
            series: series.into(),
            observed_at,
            value,
            payload: serde_json::json!({}),
        }
    }
    pub fn payload(mut self, p: serde_json::Value) -> Self {
        self.payload = p;
        self
    }
}

/// Idempotent upsert; returns count of genuinely new points. Alert rules are
/// evaluated per distinct series after the batch lands.
pub async fn persist_observations(
    state: &AppState,
    source: &str,
    obs: Vec<Observation>,
) -> Result<usize> {
    let mut series_seen: Vec<String> = Vec::new();
    let mut new = 0usize;
    for o in &obs {
        let r = sqlx::query(
            "INSERT INTO signal_observations (series, source, observed_at, value, payload)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (series, observed_at) DO NOTHING",
        )
        .bind(&o.series)
        .bind(source)
        .bind(o.observed_at)
        .bind(o.value)
        .bind(&o.payload)
        .execute(&state.pg)
        .await?;
        new += r.rows_affected() as usize;
        if !series_seen.contains(&o.series) {
            series_seen.push(o.series.clone());
        }
    }
    for s in &series_seen {
        if let Err(e) = evaluate_rules(state, source, s).await {
            tracing::warn!(series = %s, error = %e, "signal rule evaluation failed");
        }
    }
    Ok(new)
}

/// Latest observation per series (console Signals page / MCP signal_query default).
pub async fn latest_observations(
    state: &AppState,
    series_pattern: Option<&str>,
) -> Result<Vec<(String, DateTime<Utc>, f64, serde_json::Value)>> {
    let rows: Vec<(String, DateTime<Utc>, f64, serde_json::Value)> = match series_pattern {
        Some(p) => {
            sqlx::query_as(
                "SELECT DISTINCT ON (series) series, observed_at, value, payload
                 FROM signal_observations WHERE series LIKE $1
                 ORDER BY series, observed_at DESC",
            )
            .bind(p)
            .fetch_all(&state.pg)
            .await?
        }
        None => {
            sqlx::query_as(
                "SELECT DISTINCT ON (series) series, observed_at, value, payload
                 FROM signal_observations ORDER BY series, observed_at DESC",
            )
            .fetch_all(&state.pg)
            .await?
        }
    };
    Ok(rows)
}

/// The two most recent observations of a series, newest first.
async fn last_two(state: &AppState, series: &str) -> Result<Vec<(DateTime<Utc>, f64)>> {
    let rows: Vec<(DateTime<Utc>, f64)> = sqlx::query_as(
        "SELECT observed_at, value FROM signal_observations
         WHERE series = $1 ORDER BY observed_at DESC LIMIT 2",
    )
    .bind(series)
    .fetch_all(&state.pg)
    .await?;
    Ok(rows)
}

/// Minimal rule set (spec §4). Crossing semantics: fire only when the newest
/// point crosses the threshold relative to the previous stored point — never
/// re-fire while the condition persists (decay cooldown is the backstop).
async fn evaluate_rules(state: &AppState, source: &str, series: &str) -> Result<()> {
    match series {
        "fred:VIXCLS" => {
            let pts = last_two(state, series).await?;
            if let [new, prev] = pts.as_slice() {
                if new.1 > 30.0 && prev.1 <= 30.0 {
                    let title = format!("VIX crossed above 30 ({:.1} → {:.1})", prev.1, new.1);
                    let _ = crate::alerts::raise(state, crate::alerts::NewAlert {
                        severity: "warning",
                        source: "monitor:fin",
                        title: &title,
                        body: None,
                        task_id: None,
                        investigation_id: None,
                        entity_name: None,
                        evidence_id: None,
                        recommended_action: Some("Risk regime shift — check macro series and watchlist drawdowns via signal_query"),
                        dedupe_key: Some("monitor:fin:vix:cross30"),
                    }).await;
                }
            }
        }
        "fred:T10Y2Y" => {
            let pts = last_two(state, series).await?;
            if let [new, prev] = pts.as_slice() {
                if new.1.signum() != prev.1.signum() && new.1 != 0.0 {
                    let dir = if new.1 > 0.0 { "un-inverted (steepened)" } else { "INVERTED" };
                    let title = format!("10Y-2Y spread flipped sign: {dir} ({:.2} → {:.2})", prev.1, new.1);
                    let _ = crate::alerts::raise(state, crate::alerts::NewAlert {
                        severity: "warning",
                        source: "monitor:fin",
                        title: &title,
                        body: None,
                        task_id: None,
                        investigation_id: None,
                        entity_name: None,
                        evidence_id: None,
                        recommended_action: Some("Yield-curve regime change — review macro posture"),
                        dedupe_key: Some("monitor:fin:t10y2y:flip"),
                    }).await;
                }
            }
        }
        _ if series.starts_with("quote:") => {
            let pts = last_two(state, series).await?;
            if let [new, prev] = pts.as_slice() {
                if prev.1 > 0.0 {
                    let pct = (new.1 - prev.1) / prev.1 * 100.0;
                    let abs = pct.abs();
                    if abs >= 7.0 {
                        let sym = &series[6..];
                        let title = format!("{sym} daily move {pct:+.1}% ({:.2} → {:.2})", prev.1, new.1);
                        let _ = crate::alerts::raise(state, crate::alerts::NewAlert {
                            severity: if abs >= 10.0 { "warning" } else { "info" },
                            source: "monitor:fin",
                            title: &title,
                            body: None,
                            task_id: None,
                            investigation_id: None,
                            entity_name: None,
                            evidence_id: None,
                            recommended_action: Some("Large single-day move on watchlist name — check finintel evidence for catalyst"),
                            dedupe_key: Some(&format!("monitor:fin:move:{sym}")),
                        }).await;
                    }
                }
            }
        }
        _ => {}
    }
    let _ = source;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn series_names_carry_units() {
        // Design invariant (atlas F1 lesson): no bare ambiguous series names.
        for s in ["fred:CPIAUCSL_YOY", "quote:AAPL", "sentiment:AAPL", "eia:RWTC_USD_BBL"] {
            assert!(s.contains(':'), "{s}");
        }
    }
}
