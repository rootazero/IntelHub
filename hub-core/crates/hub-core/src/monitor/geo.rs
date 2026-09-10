//! Signal → geo_events persistence + fan-out (bus event, high-severity
//! alerts, per-source health cells in Redis).

use serde_json::json;

use crate::state::AppState;

use super::Signal;

/// Idempotently upsert a batch of signals for one source. Returns rows added.
pub async fn persist_signals(
    state: &AppState,
    source: &str,
    signals: Vec<Signal>,
) -> crate::error::Result<usize> {
    let mut new = 0usize;
    for s in &signals {
        if !(-90.0..=90.0).contains(&s.lat) || !(-180.0..=180.0).contains(&s.lon) {
            continue;
        }
        let src = format!("monitor:{source}");
        let res = sqlx::query(
            "INSERT INTO geo_events (source, external_id, kind, title, lat, lon, severity, occurred_at, payload)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT (source, external_id) DO NOTHING",
        )
        .bind(&src)
        .bind(&s.external_id)
        .bind(s.kind)
        .bind(&s.title)
        .bind(s.lat)
        .bind(s.lon)
        .bind(s.severity)
        .bind(s.occurred_at)
        .bind(&s.payload)
        .execute(&state.pg)
        .await;
        match res {
            Ok(r) if r.rows_affected() > 0 => {
                new += 1;
                if matches!(s.severity, "priority" | "flash") {
                    raise_for(state, source, s).await;
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, source, "geo_events upsert failed"),
        }
    }
    crate::events::publish(
        state,
        crate::types::BusEvent::new(
            "monitor_sweep_ingested",
            &format!("hub:monitor:{source}"),
            json!({ "source": source, "new": new, "fetched": signals.len() }),
        ),
    )
    .await;
    Ok(new)
}

/// PRIORITY/FLASH signals become hub alerts via the standard dedupe path
/// (semantic key: digit-normalized so escalating counts bump, not re-open).
async fn raise_for(state: &AppState, source: &str, s: &Signal) {
    let sev = if s.severity == "flash" {
        "critical"
    } else {
        "warning"
    };
    let dkey = format!("monitor:{source}:{}", crate::alerts::semantic_key(&s.title));
    let title = format!("[monitor/{source}] {}", s.title);
    let body = serde_json::to_string(&s.payload).ok();
    let _ = crate::alerts::raise(
        state,
        crate::alerts::NewAlert {
            severity: sev,
            source: "osint",
            title: &title,
            body: body.as_deref(),
            task_id: None,
            investigation_id: None,
            entity_name: None,
            evidence_id: None,
            recommended_action: Some("在 Radar 页查看地理上下文"),
            dedupe_key: Some(&dkey),
        },
    )
    .await;
}

/// Per-source health cell: HSET hub:monitor:health <name> <json>. Read by the
/// console overview (System/Overview pages) and the SP7 /monitor/delta
/// endpoint. Before overwriting, the previous cell's counters are shifted into
/// prev_* so the delta endpoint can compute per-sweep direction (single writer
/// per source loop — no read-modify-write race).
pub async fn report_health(
    state: &AppState,
    source: &str,
    ok: bool,
    detail: &str,
    new: usize,
    fetched: usize,
) {
    let prev: Option<serde_json::Value> = state
        .redis_timed(
            redis::cmd("HGET")
                .arg("hub:monitor:health")
                .arg(source)
                .clone(),
            2000,
        )
        .await
        .flatten()
        .and_then(|s: String| serde_json::from_str(&s).ok());
    let mut cell = json!({
        "state": if ok { "ok" } else { "error" },
        "ts": chrono::Utc::now().to_rfc3339(),
        "detail": detail.chars().take(300).collect::<String>(),
        "last_new": new,
        "last_fetched": fetched,
    });
    if ok {
        // Sweep history ring (SP7 polish): successful sweeps only, so a failed
        // sweep never poisons the delta baseline, and the baseline survives
        // hub restarts (the list lives in redis, not in process memory).
        let entry = json!({"ts": cell["ts"].clone(), "new": new, "fetched": fetched}).to_string();
        let key = format!("hub:monitor:sweephist:{source}");
        let _: Option<()> = state
            .redis_timed(redis::cmd("LPUSH").arg(&key).arg(entry).clone(), 2000)
            .await;
        let _: Option<()> = state
            .redis_timed(redis::cmd("LTRIM").arg(&key).arg(0).arg(9).clone(), 2000)
            .await;
    }
    if let Some(p) = prev {
        if let Some(v) = p.get("last_new").and_then(|v| v.as_u64()) {
            cell["prev_new"] = json!(v);
        }
        if let Some(v) = p.get("last_fetched").and_then(|v| v.as_u64()) {
            cell["prev_fetched"] = json!(v);
        }
    }
    let _: Option<()> = state
        .redis_timed(
            redis::cmd("HSET")
                .arg("hub:monitor:health")
                .arg(source)
                .arg(cell.to_string())
                .clone(),
            2000,
        )
        .await;
    if new > 0 {
        let _: Option<()> = state
            .redis_timed(
                redis::cmd("INCRBY")
                    .arg(format!("hub:monitor:events:{source}"))
                    .arg(new)
                    .clone(),
                2000,
            )
            .await;
    }
}
