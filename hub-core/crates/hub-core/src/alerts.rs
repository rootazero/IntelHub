//! Alert Engine (directive §54) — unified alert center.
//!
//! Sources: osint|agent|sensor|infra|budget|security. Every alert carries
//! severity, links (task/investigation/entity/evidence) and a recommended
//! action. Dedup: an open alert with the same dedupe_key within 1h is bumped
//! (occurrence+1) instead of duplicated. Delivery: generic webhook (config
//! driven, severity-filtered, 3 retries, every attempt in alert_deliveries).

use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::Result;
use crate::state::AppState;
use crate::types::BusEvent;

pub struct NewAlert<'a> {
    pub severity: &'a str, // info|warning|critical
    pub source: &'a str,   // osint|agent|sensor|infra|budget|security
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub task_id: Option<Uuid>,
    pub investigation_id: Option<Uuid>,
    pub entity_name: Option<&'a str>,
    pub evidence_id: Option<Uuid>,
    pub recommended_action: Option<&'a str>,
    pub dedupe_key: Option<&'a str>,
}

fn severity_rank(s: &str) -> i32 {
    match s {
        "critical" => 3,
        "warning" => 2,
        _ => 1,
    }
}

/// Raise an alert (deduped), broadcast ALERT_RAISED, enqueue webhook delivery.
pub async fn raise(state: &AppState, a: NewAlert<'_>) -> Result<Uuid> {
    // Dedup: same key still open and touched within the last hour → bump.
    if let Some(key) = a.dedupe_key {
        let existing: Option<(Uuid,)> = sqlx::query_as(
            "UPDATE alerts SET occurrence = occurrence + 1, updated_at = now()
             WHERE dedupe_key = $1 AND status = 'open' AND updated_at > now() - interval '1 hour'
             RETURNING alert_id",
        )
        .bind(key)
        .fetch_optional(&state.pg)
        .await?;
        if let Some((id,)) = existing {
            return Ok(id);
        }
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO alerts (alert_id, severity, source, title, body, task_id, investigation_id,
                             entity_name, evidence_id, recommended_action, dedupe_key)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(id)
    .bind(a.severity)
    .bind(a.source)
    .bind(a.title)
    .bind(a.body)
    .bind(a.task_id)
    .bind(a.investigation_id)
    .bind(a.entity_name)
    .bind(a.evidence_id)
    .bind(a.recommended_action)
    .bind(a.dedupe_key)
    .execute(&state.pg)
    .await?;

    crate::events::publish(
        state,
        BusEvent::new(
            "ALERT_RAISED",
            "system:alert-engine",
            json!({
                "alert_id": id, "severity": a.severity, "source": a.source,
                "title": a.title, "investigation_id": a.investigation_id,
            }),
        ),
    )
    .await;

    // Webhook delivery (severity-filtered) — dispatcher worker picks it up.
    if let Some(url) = &state.config.alert_webhook_url {
        if severity_rank(a.severity) >= severity_rank(&state.config.alert_webhook_min_severity) {
            sqlx::query(
                "INSERT INTO alert_deliveries (delivery_id, alert_id, endpoint) VALUES ($1,$2,$3)",
            )
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(url)
            .execute(&state.pg)
            .await?;
        }
    }
    Ok(id)
}

// ---------- queries / actions (shared by MCP tools and REST) ----------

pub async fn list(
    state: &AppState,
    status: Option<&str>,
    source: Option<&str>,
    severity: Option<&str>,
    limit: i64,
) -> Result<Value> {
    let rows: Vec<(Uuid, String, String, String, Option<String>, String, i32, Option<Uuid>, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "SELECT alert_id, severity, source, title, recommended_action, status, occurrence,
                    investigation_id, created_at
             FROM alerts
             WHERE ($1::text IS NULL OR status = $1)
               AND ($2::text IS NULL OR source = $2)
               AND ($3::text IS NULL OR severity = $3)
             ORDER BY created_at DESC LIMIT $4",
        )
        .bind(status)
        .bind(source)
        .bind(severity)
        .bind(limit.clamp(1, 200))
        .fetch_all(&state.pg)
        .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(id, sev, src, title, action, st, occ, inv, ts)| {
            json!({
                "alert_id": id, "severity": sev, "source": src, "title": title,
                "recommended_action": action, "status": st, "occurrence": occ,
                "investigation_id": inv, "created_at": ts,
            })
        })
        .collect();
    Ok(json!({ "count": items.len(), "items": items }))
}

pub async fn set_status(state: &AppState, alert_id: Uuid, status: &str, actor: &str) -> Result<bool> {
    let r = sqlx::query(
        "UPDATE alerts SET status=$1, actor=$2, updated_at=now() WHERE alert_id=$3 AND status='open'",
    )
    .bind(status)
    .bind(actor)
    .bind(alert_id)
    .execute(&state.pg)
    .await?;
    if r.rows_affected() > 0 {
        crate::events::publish(
            state,
            BusEvent::new(
                "ALERT_UPDATED",
                actor,
                json!({ "alert_id": alert_id, "status": status }),
            ),
        )
        .await;
    }
    Ok(r.rows_affected() > 0)
}

// ---------- webhook dispatcher worker ----------

pub async fn run_dispatcher(state: AppState, ct: tokio_util::sync::CancellationToken) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = ct.cancelled() => break,
            _ = tick.tick() => {
                if let Err(e) = dispatch_once(&state).await {
                    tracing::warn!(error = %e, "alert dispatcher tick failed");
                }
            }
        }
    }
}

async fn dispatch_once(state: &AppState) -> Result<()> {
    let pending: Vec<(Uuid, Uuid, String, i32, String, String, String, Option<String>)> =
        sqlx::query_as(
            "SELECT d.delivery_id, d.alert_id, d.endpoint, d.attempts,
                    a.severity, a.source, a.title, a.recommended_action
             FROM alert_deliveries d JOIN alerts a ON a.alert_id = d.alert_id
             WHERE d.status = 'PENDING'
             ORDER BY d.created_at LIMIT 10",
        )
        .fetch_all(&state.pg)
        .await?;
    for (delivery_id, alert_id, endpoint, attempts, sev, src, title, action) in pending {
        let payload = json!({
            "alert_id": alert_id, "severity": sev, "source": src,
            "title": title, "recommended_action": action,
            "ts": chrono::Utc::now(),
        });
        let res = state
            .http
            .post(&endpoint)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        let (ok, err) = match &res {
            Ok(r) if r.status().is_success() => (true, None),
            Ok(r) => (false, Some(format!("HTTP {}", r.status()))),
            Err(e) => (false, Some(e.to_string())),
        };
        if ok {
            sqlx::query(
                "UPDATE alert_deliveries SET status='DELIVERED', attempts=attempts+1, delivered_at=now()
                 WHERE delivery_id=$1",
            )
        } else if attempts >= 2 {
            sqlx::query(
                "UPDATE alert_deliveries SET status='FAILED', attempts=attempts+1, last_error=$2
                 WHERE delivery_id=$1",
            )
            .bind(err)
        } else {
            // Backoff: leave PENDING; next tick retries (attempts gate the delay).
            sqlx::query(
                "UPDATE alert_deliveries SET attempts=attempts+1, last_error=$2,
                 created_at = now() + make_interval(secs => power(2, attempts) * 15)
                 WHERE delivery_id=$1",
            )
            .bind(err)
        }
        .bind(delivery_id)
        .execute(&state.pg)
        .await?;
    }
    Ok(())
}

// ---------- sensor health flap watcher (§54 sensor alerts) ----------

pub async fn run_sensor_watcher(state: AppState, ct: tokio_util::sync::CancellationToken) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = ct.cancelled() => break,
            _ = tick.tick() => {
                if let Err(e) = check_sensor_flaps(&state).await {
                    tracing::warn!(error = %e, "sensor watcher tick failed");
                }
            }
        }
    }
}

async fn check_sensor_flaps(state: &AppState) -> Result<()> {
    let health = crate::api::system_health(state).await;
    let Some(components) = health.get("components").and_then(|c| c.as_object()) else {
        return Ok(());
    };
    for (name, probe) in components {
        let cur = probe.get("status").and_then(|s| s.as_str()).unwrap_or("down");
        let key = format!("hub:sensor_state:{name}");
        let prev: Option<String> = state
            .redis_timed::<Option<String>>(redis::cmd("GET").arg(&key).clone(), 2000)
            .await
            .flatten();
        let _: Option<()> = state
            .redis_timed(redis::cmd("SET").arg(&key).arg(cur).clone(), 2000)
            .await;
        match (prev.as_deref(), cur) {
            (Some("up"), "down") => {
                let _ = raise(
                    state,
                    NewAlert {
                        severity: "critical",
                        source: if ["searxng", "crawl4ai"].contains(&name.as_str()) {
                            "sensor"
                        } else {
                            "infra"
                        },
                        title: &format!("Component {name} went DOWN"),
                        body: None,
                        task_id: None,
                        investigation_id: None,
                        entity_name: None,
                        evidence_id: None,
                        recommended_action: Some(
                            "Check container status: hub-compose.sh ps / logs",
                        ),
                        dedupe_key: Some(&format!("flap:{name}:down")),
                    },
                )
                .await;
            }
            (Some("down"), "up") => {
                let _ = raise(
                    state,
                    NewAlert {
                        severity: "info",
                        source: "infra",
                        title: &format!("Component {name} recovered"),
                        body: None,
                        task_id: None,
                        investigation_id: None,
                        entity_name: None,
                        evidence_id: None,
                        recommended_action: None,
                        dedupe_key: None,
                    },
                )
                .await;
            }
            _ => {}
        }
    }
    Ok(())
}
