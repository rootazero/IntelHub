//! Crucix signal-layer integration (directive §51: aggregate key events /
//! alerts / geo markers — do NOT replicate Crucix; §26 Global Radar feeds).
//!
//! Worker: every 60s poll `{HUB_CRUCIX_URL}/api/health`; when `lastSweep`
//! advances, fetch `/api/data` and normalize every geo-bearing item across
//! sources into `geo_events` (idempotent upsert on (source, external_id)).
//! Explicit alert objects at FLASH/PRIORITY tier become hub alerts (§54)
//! via the standard dedupe path. Crucix being down degrades to log+backoff —
//! zero impact on any other hub function (§72).

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

const LAST_SWEEP_KEY: &str = "hub:crucix:last_sweep";
const MAX_PER_SWEEP: usize = 5000;

/// source/section name → geo_events.kind (dashboard payload top-level keys)
fn kind_for(source: &str) -> &'static str {
    match source {
        "thermal" | "FIRMS" => "fire",
        "acled" | "ACLED" => "conflict",
        "air" | "airMeta" | "OpenSky" | "ADS-B" => "flight",
        "nuke" | "nukeSignals" | "epa" | "Safecast" | "EPA" => "radiation",
        "chokepoints" | "Maritime" => "maritime",
        "news" | "newsFeed" | "gdelt" | "tg" | "GDELT" | "ReliefWeb" => "news",
        "who" | "health" | "WHO" => "health",
        "fred" | "energy" | "metals" | "bls" | "treasury" | "gscpi" | "markets"
        | "FRED" | "Treasury" | "BLS" | "EIA" | "GSCPI" | "YFinance" => "economic",
        _ => "other",
    }
}

/// Payload sections that never carry actionable geo items.
const SKIP_SECTIONS: &[&str] = &[
    "meta", "ideas", "ideasSource", "delta", "crucix", "errors", "timing",
];

fn severity_for(source: &str, item: &Value) -> &'static str {
    match source {
        "acled" | "ACLED" => {
            let f = item.get("fatalities").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if f >= 10.0 {
                "priority"
            } else if f > 0.0 {
                "routine"
            } else {
                "info"
            }
        }
        "thermal" | "FIRMS" => "routine",
        _ => "info",
    }
}

fn title_of(item: &Value) -> String {
    for k in ["title", "label", "name", "headline", "event_type", "description"] {
        if let Some(s) = item.get(k).and_then(|v| v.as_str()) {
            return s.chars().take(200).collect();
        }
    }
    "untitled".to_string()
}

fn num(v: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(n) = v.get(k).and_then(|x| x.as_f64()) {
            return Some(n);
        }
        // tolerate strings like "12.3"
        if let Some(n) = v.get(k).and_then(|x| x.as_str()).and_then(|s| s.parse().ok()) {
            return Some(n);
        }
    }
    None
}

/// Recursively collect objects that carry both lat and lon (bounded).
fn collect_geo(v: &Value, out: &mut Vec<Value>, depth: u8) {
    if out.len() >= MAX_PER_SWEEP || depth > 6 {
        return;
    }
    match v {
        Value::Object(m) => {
            let lat = num(v, &["lat", "latitude"]);
            let lon = num(v, &["lon", "lng", "longitude"]);
            if let (Some(_), Some(_)) = (lat, lon) {
                out.push(v.clone());
                return; // don't descend into a geo item's own fields
            }
            for (_, child) in m {
                collect_geo(child, out, depth + 1);
            }
        }
        Value::Array(a) => {
            for child in a {
                collect_geo(child, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn external_id(source: &str, item: &Value) -> String {
    for k in ["id", "event_id", "eventId", "uid", "key"] {
        if let Some(s) = item.get(k).and_then(|v| v.as_str()) {
            return format!("{k}:{s}");
        }
        if let Some(n) = item.get(k).and_then(|v| v.as_i64()) {
            return format!("{k}:{n}");
        }
    }
    // fallback: content hash of stable fields
    let basis = format!(
        "{}|{}|{}|{}",
        source,
        num(item, &["lat", "latitude"]).unwrap_or(0.0),
        num(item, &["lon", "lng", "longitude"]).unwrap_or(0.0),
        title_of(item)
    );
    format!("h:{}", crate::auth::hash_key(&basis))
}

/// One ingestion pass over a fetched /api/data payload. Returns row count.
/// The dashboard payload puts sections at the TOP level (thermal, acled,
/// chokepoints, news, …) — not under `sources` (that's the raw briefing file).
pub async fn ingest_sweep(state: &AppState, data: &Value) -> crate::error::Result<usize> {
    let mut total = 0usize;
    let root = data
        .get("sources")
        .and_then(|s| s.as_object())
        .or_else(|| data.as_object());
    let Some(sources) = root else { return Ok(0) };
    for (name, sdata) in sources {
        if SKIP_SECTIONS.contains(&name.as_str()) {
            continue;
        }
        let mut items = Vec::new();
        collect_geo(sdata, &mut items, 0);
        if items.is_empty() {
            continue;
        }
        let kind = kind_for(name);
        let source = format!("crucix:{}", name.to_lowercase());
        for item in items.iter().take(MAX_PER_SWEEP.saturating_sub(total)) {
            let lat = num(item, &["lat", "latitude"]);
            let lon = num(item, &["lon", "lng", "longitude"]);
            let (Some(lat), Some(lon)) = (lat, lon) else { continue };
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                continue;
            }
            let res = sqlx::query(
                "INSERT INTO geo_events (source, external_id, kind, title, lat, lon, severity, payload)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                 ON CONFLICT (source, external_id) DO NOTHING",
            )
            .bind(&source)
            .bind(external_id(name, item))
            .bind(kind)
            .bind(title_of(item))
            .bind(lat)
            .bind(lon)
            .bind(severity_for(name, item))
            .bind(item)
            .execute(&state.pg)
            .await;
            match res {
                Ok(r) => total += r.rows_affected() as usize,
                Err(e) => tracing::warn!(error = %e, source = %source, "geo_events upsert failed"),
            }
        }
    }

    // Convert explicit alert objects (FLASH / PRIORITY) into hub alerts (§54).
    for key in ["alerts", "breaking", "breakingAlerts"] {
        if let Some(arr) = data.get(key).and_then(|a| a.as_array()) {
            for a in arr {
                let tier = a
                    .get("tier")
                    .or_else(|| a.get("severity"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let sev = match tier.as_str() {
                    "flash" => "critical",
                    "priority" => "warning",
                    _ => continue,
                };
                let title = title_of(a);
                let dkey = format!("crucix:{}:{}", tier, crate::auth::hash_key(&title));
                let _ = crate::alerts::raise(
                    state,
                    crate::alerts::NewAlert {
                        severity: sev,
                        source: "osint",
                        title: &format!("[Crucix/{tier}] {title}"),
                        body: a.get("body").and_then(|v| v.as_str()),
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
        }
    }

    Ok(total)
}

pub async fn run_crucix_sync(state: AppState, ct: CancellationToken) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    tracing::info!(url = %state.config.crucix_url, "crucix-sync worker started");
    loop {
        tokio::select! {
            _ = ct.cancelled() => { tracing::info!("crucix-sync shutting down"); return; }
            _ = tick.tick() => {}
        }

        // 1) health probe — is there a new sweep?
        let health = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            state.http.get(format!("{}/api/health", state.config.crucix_url)).send(),
        )
        .await;
        let last_sweep = match health {
            Ok(Ok(r)) => match r.json::<Value>().await {
                Ok(j) => j.get("lastSweep").and_then(|v| v.as_str()).map(|s| s.to_string()),
                Err(_) => None,
            },
            _ => None, // crucix down / timeout → backoff to next tick (§72)
        };
        let Some(last_sweep) = last_sweep else { continue };

        let seen: Option<String> = state
            .redis_timed::<Option<String>>(redis::cmd("GET").arg(LAST_SWEEP_KEY).clone(), 2000)
            .await
            .flatten();
        if seen.as_deref() == Some(last_sweep.as_str()) {
            continue; // nothing new
        }

        // 2) fetch + ingest
        let data = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            state.http.get(format!("{}/api/data", state.config.crucix_url)).send(),
        )
        .await;
        let Ok(Ok(resp)) = data else { continue };
        let Ok(payload) = resp.json::<Value>().await else { continue };
        match ingest_sweep(&state, &payload).await {
            Ok(n) => {
                let _: Option<()> = state
                    .redis_timed(redis::cmd("SET").arg(LAST_SWEEP_KEY).arg(&last_sweep).clone(), 2000)
                    .await;
                tracing::info!(sweep = %last_sweep, geo_events = n, "crucix sweep ingested");
                crate::events::publish(
                    &state,
                    crate::types::BusEvent::new(
                        "crucix_sweep_ingested",
                        "hub:crucix-sync",
                        serde_json::json!({ "sweep": last_sweep, "geo_events_new": n }),
                    ),
                )
                .await;
            }
            Err(e) => tracing::warn!(error = %e, "crucix sweep ingest failed"),
        }
    }
}
