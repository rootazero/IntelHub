//! REST API (SP3 prerequisite) + SSE event stream + health aggregation.
//! Shares the store/service layer with MCP tools — no duplicated logic.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Json, Response,
    },
    routing::get,
    Router,
};
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::state::AppState;
use crate::types::AgentIdentity;
use axum::Extension;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/health", get(health))
        .route("/api/v1/investigations", get(list_investigations).post(create_investigation))
        .route("/api/v1/investigations/{id}", get(get_investigation).patch(update_investigation))
        .route("/api/v1/investigations/{id}/findings", get(list_findings))
        .route("/api/v1/documents/{id}", get(get_document))
        .route("/api/v1/search", get(search))
        .route("/api/v1/events", get(events_sse))
        .route("/api/v1/events/recent", get(events_recent))
        .route("/api/v1/alerts", get(list_alerts))
        .route("/api/v1/alerts/{id}/ack", axum::routing::post(ack_alert))
        .route("/api/v1/alerts/{id}/mute", axum::routing::post(mute_alert))
        .route("/api/v1/traces/{trace_id}", get(get_trace))
        .route("/api/v1/components", get(list_components))
        .route("/api/v1/components/{name}/actions", axum::routing::post(component_action))
        // SP3 console API (read-only aggregates, §24)
        .route("/api/v1/overview", get(console_overview))
        .route("/api/v1/search/unified", get(console_unified_search))
        .route("/api/v1/documents", get(console_documents))
        .route("/api/v1/entities", get(console_entities))
        .route("/api/v1/entities/{id}", get(console_entity))
        .route("/api/v1/agents/activity", get(console_agents_activity))
        .route("/api/v1/audit", get(console_audit))
        .route("/api/v1/tasks", get(console_tasks))
        .route("/api/v1/investigations/{id}/workspace", get(console_workspace))
        // SP5
        .route("/api/v1/evidence", axum::routing::post(post_evidence))
        // SP4
        .route("/api/v1/radar/events", get(console_radar_events))
        .route("/api/v1/metrics/summary", get(console_metrics_summary))
        // SP6B finance signals
        .route("/api/v1/signals/latest", get(console_signals_latest))
        .route("/api/v1/signals/history", get(console_signals_history))
        .route("/api/v1/monitor/delta", get(console_monitor_delta))
        .route("/api/v1/signals/watchlist", get(console_watchlist_list))
        .route("/api/v1/signals/watchlist", axum::routing::post(console_watchlist_upsert))
        .route("/api/v1/signals/watchlist/{symbol}", axum::routing::delete(console_watchlist_remove))
        // SP9: knowledge graph read API (console entity/investigation pages)
        .route("/api/v1/graph/entities/search", get(graph_entities_search))
        .route("/api/v1/graph/neighbors", get(graph_neighbors))
        .route("/api/v1/graph/entity/{id}/timeline", get(graph_entity_timeline))
        .route("/api/v1/graph/path", get(graph_path))
        .route("/api/v1/graph/investigation/{id}", get(graph_investigation))
        .route("/api/v1/graph/evidence", get(graph_evidence))
        // SP3 static console (public shell; every data call still needs a key)
        .fallback(serve_static)
}

async fn healthz() -> &'static str {
    "ok"
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn hub_err(e: crate::error::HubError) -> Response {
    use crate::error::HubError::*;
    match e {
        NotFound(m) => err(StatusCode::NOT_FOUND, m),
        BadRequest(m) => err(StatusCode::BAD_REQUEST, m),
        PolicyDenied(m) => err(StatusCode::FORBIDDEN, format!("policy_denied: {m}")),
        BudgetDenied { state, reason } => {
            err(StatusCode::TOO_MANY_REQUESTS, format!("budget_denied ({state}): {reason}"))
        }
        other => err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

// ---------- health aggregation (shared with MCP get_system_health) ----------

async fn probe<Fut>(f: impl FnOnce() -> Fut) -> Value
where
    Fut: std::future::Future<Output = bool>,
{
    let t = Instant::now();
    // Hard 3s cap per probe: a hung dependency (e.g. half-open bolt connection
    // after a restart) must degrade to "down", never hang the endpoint (§68).
    let ok = tokio::time::timeout(std::time::Duration::from_secs(3), f())
        .await
        .unwrap_or(false);
    json!({ "status": if ok { "up" } else { "down" }, "latency_ms": t.elapsed().as_millis() as i64 })
}

pub async fn system_health(state: &AppState) -> Value {
    let pg = probe(|| async {
        sqlx::query("SELECT 1").execute(&state.pg).await.is_ok()
    })
    .await;

    let mut conn = state.redis().await;
    let redis = probe(|| async {
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map(|p| p == "PONG")
            .unwrap_or(false)
    })
    .await;

    let neo4j = probe(|| async {
        match state.neo4j.execute(neo4rs::query("RETURN 1")).await {
            Ok(mut rows) => rows.next().await.ok().flatten().is_some(),
            Err(_) => false,
        }
    })
    .await;

    let qdrant = probe(|| async { crate::vector::healthy(state).await }).await;

    let searxng = probe(|| async {
        let url = format!("{}/healthz", state.config.searxng_url);
        matches!(state.http.get(&url).send().await, Ok(r) if r.status().is_success())
    })
    .await;

    let crawl4ai = probe(|| async {
        let url = format!("{}/health", state.config.crawl4ai_url);
        matches!(state.http.get(&url).send().await, Ok(r) if r.status().is_success())
    })
    .await;

    // SP6: native monitor — healthy when ≥1 collector reports ok in Redis.
    let monitor = probe(|| async {
        let map: Option<std::collections::HashMap<String, String>> = state
            .redis_timed(redis::cmd("HGETALL").arg("hub:monitor:health").clone(), 2000)
            .await;
        map.map(|m| m.values().any(|cell| cell.contains("\"state\":\"ok\"")))
            .unwrap_or(false)
    })
    .await;
    let prometheus = probe(|| async {
        let url = format!("{}/-/healthy", state.config.prometheus_url);
        matches!(state.http.get(&url).send().await, Ok(r) if r.status().is_success())
    })
    .await;
    let grafana = probe(|| async {
        matches!(state.http.get("http://172.30.3.22:3000/api/health").send().await, Ok(r) if r.status().is_success())
    })
    .await;

    let containers = docker_containers().await;

    json!({
        "service": "intelhub-core",
        "version": env!("CARGO_PKG_VERSION"),
        "ts": chrono::Utc::now(),
        "components": {
            "postgres": pg,
            "redis": redis,
            "neo4j": neo4j,
            "qdrant": qdrant,
            "searxng": searxng,
            "crawl4ai": crawl4ai,
            "monitor": monitor,
            "prometheus": prometheus,
            "grafana": grafana,
        },
        "containers": containers,
    })
}

/// Best-effort docker container states (native hub runs as zou ∈ docker group).
async fn docker_containers() -> Value {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("docker")
            .args(["ps", "-a", "--format", "{{json .}}"])
            .output(),
    )
    .await
    .ok()
    .and_then(|r| r.ok());
    match out {
        Some(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let items: Vec<Value> = text
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .map(|mut c| {
                    if let Some(obj) = c.as_object_mut() {
                        obj.retain(|k, _| ["Names", "Image", "Status", "State"].contains(&k.as_str()));
                    }
                    c
                })
                .collect();
            json!(items)
        }
        _ => json!({ "error": "docker ps unavailable" }),
    }
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(system_health(&state).await)
}

// ---------- investigations ----------

#[derive(Debug, Deserialize)]
struct ListParams {
    status: Option<String>,
    limit: Option<i64>,
}

async fn list_investigations(
    State(state): State<Arc<AppState>>,
    Query(p): Query<ListParams>,
) -> Result<Json<Value>, Response> {
    crate::store::list_investigations(&state.pg, p.status.as_deref(), p.limit.unwrap_or(20).clamp(1, 100))
        .await
        .map(Json)
        .map_err(hub_err)
}

#[derive(Debug, Deserialize)]
struct CreateInvestigationBody {
    title: String,
    question: Option<String>,
    target: Option<String>,
    hypothesis: Option<String>,
    /// Optional Radar geo event to seed as first evidence + system finding
    /// (console "convert to investigation" flow).
    source_event_id: Option<Uuid>,
}

async fn create_investigation(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Json(body): Json<CreateInvestigationBody>,
) -> Result<Json<Value>, Response> {
    // SP2B: REST goes through the same governance gate as MCP.
    crate::policy::preflight(&state, &agent, "create_investigation")
        .await
        .map_err(|e| hub_err(e.into()))?;
    let id = crate::store::create_investigation(
        &state.pg,
        &body.title,
        body.question.as_deref(),
        body.target.as_deref(),
        body.hypothesis.as_deref(),
        &format!("agent:{}", agent.name),
    )
    .await
    .map_err(|e| hub_err(e.into()))?;
    // Seed from the originating Radar event (console convert flow) so the
    // investigation never opens empty; best-effort — a seed failure must not
    // fail investigation creation.
    let mut seeded = false;
    if let Some(ev) = body.source_event_id {
        match crate::store::seed_from_geo_event(&state.pg, id, ev).await {
            Ok(s) => seeded = s,
            Err(e) => tracing::warn!("radar seed failed for investigation {id}: {e}"),
        }
    }
    // REST and MCP must behave identically — publish the same event.
    crate::events::publish(
        &state,
        crate::types::BusEvent::new(
            "TASK_CREATED",
            &format!("agent:{}", agent.name),
            json!({ "investigation_id": id, "title": body.title }),
        )
        .with_investigation(id),
    )
    .await;
    Ok(Json(json!({ "investigation_id": id, "seeded": seeded })))
}

async fn get_investigation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    crate::store::get_investigation(&state.pg, id).await.map(Json).map_err(hub_err)
}

#[derive(Debug, Deserialize)]
struct UpdateInvestigationBody {
    title: Option<String>,
    question: Option<String>,
    target: Option<String>,
    hypothesis: Option<String>,
    status: Option<String>,
}

async fn update_investigation(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateInvestigationBody>,
) -> Result<Json<Value>, Response> {
    crate::policy::preflight(&state, &agent, "update_investigation")
        .await
        .map_err(|e| hub_err(e.into()))?;
    crate::store::update_investigation(
        &state.pg,
        id,
        body.title.as_deref(),
        body.question.as_deref(),
        body.target.as_deref(),
        body.hypothesis.as_deref(),
        body.status.as_deref(),
    )
    .await
    .map(|updated| Json(json!({ "investigation_id": id, "updated": updated })))
    .map_err(hub_err)
}

async fn list_findings(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    crate::store::list_findings(&state.pg, id).await.map(Json).map_err(hub_err)
}

// ---------- documents / search ----------

async fn get_document(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    // SP3: richer detail (document + reverse references) via console module.
    crate::console::document_detail(&state, id).await.map(Json).map_err(hub_err)
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
    limit: Option<i64>,
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(p): Query<SearchParams>,
) -> Result<Json<Value>, Response> {
    crate::store::keyword_search(&state.pg, &p.q, p.limit.unwrap_or(10).clamp(1, 50))
        .await
        .map(Json)
        .map_err(hub_err)
}

// ---------- events ----------

async fn events_sse(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(ev) => {
                let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                Some((Ok(SseEvent::default().event(ev.event_type.clone()).data(data)), rx))
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                Some((Ok(SseEvent::default().event("LAGGED").data(format!("{n} events dropped"))), rx))
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(25)).text("keepalive"))
}

async fn events_recent(
    State(state): State<Arc<AppState>>,
    Query(p): Query<ListParams>,
) -> Result<Json<Value>, Response> {
    crate::store::recent_events(&state.pg, p.limit.unwrap_or(50).clamp(1, 200))
        .await
        .map(Json)
        .map_err(hub_err)
}

// ---------- SP2B: alerts (§54) ----------

#[derive(Debug, Deserialize)]
struct AlertListParams {
    status: Option<String>,
    source: Option<String>,
    severity: Option<String>,
    limit: Option<i64>,
}

async fn list_alerts(
    State(state): State<Arc<AppState>>,
    Query(p): Query<AlertListParams>,
) -> Result<Json<Value>, Response> {
    crate::alerts::list(
        &state,
        p.status.as_deref(),
        p.source.as_deref(),
        p.severity.as_deref(),
        p.limit.unwrap_or(50),
    )
    .await
    .map(Json)
    .map_err(hub_err)
}

async fn ack_alert(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    crate::alerts::set_status(&state, id, "ack", &format!("agent:{}", agent.name))
        .await
        .map(|ok| Json(json!({ "alert_id": id, "updated": ok })))
        .map_err(hub_err)
}

async fn mute_alert(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    crate::alerts::set_status(&state, id, "muted", &format!("agent:{}", agent.name))
        .await
        .map(|ok| Json(json!({ "alert_id": id, "updated": ok })))
        .map_err(hub_err)
}

// ---------- SP2B: component lifecycle (§68–69) ----------

async fn list_components(
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    Json(crate::components::list_components(&state).await)
}

#[derive(Debug, Deserialize)]
struct ComponentActionBody {
    action: String,
}

/// Level 3 — policy::preflight enforces the admin token (default DENY, §61).
async fn component_action(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Path(name): Path<String>,
    Json(body): Json<ComponentActionBody>,
) -> Result<Json<Value>, Response> {
    crate::policy::preflight(&state, &agent, "run_component_action")
        .await
        .map_err(|e| hub_err(e.into()))?;
    crate::components::run_action(
        &state,
        &name,
        &body.action,
        &format!("agent:{} (admin, REST)", agent.name),
    )
    .await
    .map(Json)
    .map_err(hub_err)
}

// ---------- SP3: console API handlers (thin wrappers over console.rs) ----------

async fn console_overview(State(state): State<Arc<AppState>>) -> Result<Json<Value>, Response> {
    crate::console::overview(&state).await.map(Json).map_err(hub_err)
}

#[derive(Debug, Deserialize)]
struct UnifiedSearchParams {
    q: String,
    limit: Option<i64>,
}

async fn console_unified_search(
    State(state): State<Arc<AppState>>,
    Query(p): Query<UnifiedSearchParams>,
) -> Result<Json<Value>, Response> {
    if p.q.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "q must not be empty".into()));
    }
    crate::console::unified_search(&state, p.q.trim(), p.limit.unwrap_or(8))
        .await
        .map(Json)
        .map_err(hub_err)
}

#[derive(Debug, Deserialize)]
struct DocListParams {
    q: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn console_documents(
    State(state): State<Arc<AppState>>,
    Query(p): Query<DocListParams>,
) -> Result<Json<Value>, Response> {
    crate::console::list_documents(
        &state,
        p.q.as_deref(),
        p.status.as_deref(),
        p.limit.unwrap_or(25),
        p.offset.unwrap_or(0),
    )
    .await
    .map(Json)
    .map_err(hub_err)
}

#[derive(Debug, Deserialize)]
struct EntityListParams {
    q: Option<String>,
    kind: Option<String>,
    limit: Option<i64>,
}

async fn console_entities(
    State(state): State<Arc<AppState>>,
    Query(p): Query<EntityListParams>,
) -> Result<Json<Value>, Response> {
    crate::console::list_entities(&state, p.q.as_deref(), p.kind.as_deref(), p.limit.unwrap_or(100))
        .await
        .map(Json)
        .map_err(hub_err)
}

async fn console_entity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    crate::console::get_entity(&state, id).await.map(Json).map_err(hub_err)
}

async fn console_agents_activity(State(state): State<Arc<AppState>>) -> Result<Json<Value>, Response> {
    crate::console::agents_activity(&state).await.map(Json).map_err(hub_err)
}

#[derive(Debug, Deserialize)]
struct AuditParams {
    actor: Option<String>,
    action: Option<String>,
    result: Option<String>,
    limit: Option<i64>,
}

async fn console_audit(
    State(state): State<Arc<AppState>>,
    Query(p): Query<AuditParams>,
) -> Result<Json<Value>, Response> {
    crate::console::query_audit(
        &state,
        p.actor.as_deref(),
        p.action.as_deref(),
        p.result.as_deref(),
        p.limit.unwrap_or(50),
    )
    .await
    .map(Json)
    .map_err(hub_err)
}

async fn console_tasks(
    State(state): State<Arc<AppState>>,
    Query(p): Query<ListParams>,
) -> Result<Json<Value>, Response> {
    crate::console::list_tasks(&state, p.status.as_deref(), p.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(hub_err)
}

async fn console_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    crate::console::investigation_workspace(&state, id)
        .await
        .map(Json)
        .map_err(hub_err)
}

// ---------- SP4: radar + metrics ----------

#[derive(Debug, Deserialize)]
struct RadarParams {
    from: Option<String>,
    to: Option<String>,
    severity: Option<String>,
    source: Option<String>,
    kind: Option<String>,
    limit: Option<i64>,
}

async fn console_radar_events(
    State(state): State<Arc<AppState>>,
    Query(p): Query<RadarParams>,
) -> Result<Json<Value>, Response> {
    crate::console::radar_events(
        &state,
        p.from.as_deref(),
        p.to.as_deref(),
        p.severity.as_deref(),
        p.source.as_deref(),
        p.kind.as_deref(),
        p.limit.unwrap_or(500).min(2000),
    )
    .await
    .map(Json)
    .map_err(hub_err)
}

async fn console_metrics_summary(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, Response> {
    crate::console::metrics_summary(&state).await.map(Json).map_err(hub_err)
}

// ---------- SP5: external evidence push (Huginn bridge, §50 Evidence Event) ----------

#[derive(Debug, Deserialize)]
struct EvidencePush {
    source: String,
    url: String,
    title: Option<String>,
    content: String,
    published_at: Option<String>,
    metadata: Option<Value>,
    provenance: Option<Value>,
}

async fn post_evidence(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Json(body): Json<EvidencePush>,
) -> Result<Json<Value>, Response> {
    if body.content.len() > 2 * 1024 * 1024 {
        return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "content exceeds 2MB".into()));
    }
    if body.source.trim().is_empty() || body.url.trim().is_empty() || body.content.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "source, url, content are required".into()));
    }
    if !(body.url.starts_with("http://") || body.url.starts_with("https://")) {
        return Err(err(StatusCode::BAD_REQUEST, "url must be http(s)".into()));
    }
    let published_at = body
        .published_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    let outcome = crate::ingest::ingest_content(
        &state,
        &body.source,
        &body.url,
        body.title.as_deref(),
        &body.content,
        published_at,
        body.metadata.unwrap_or_else(|| json!({})),
        body.provenance.unwrap_or_else(|| json!({ "pushed_by": agent.name })),
        None,
        "auto",
        None,
    )
    .await
    .map_err(|e| hub_err(e.into()))?;
    Ok(Json(json!({
        "document_id": outcome.document_id,
        "content_hash": outcome.content_hash,
        "duplicate": outcome.duplicate,
        "near_duplicate_of": outcome.near_duplicate_of,
    })))
}

// ---------- SP3: static console serving (public shell, SPA fallback) ----------

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("json" | "map") => "application/json",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn serve_static(
    State(state): State<Arc<AppState>>,
    uri: axum::http::Uri,
) -> Response {
    let root = std::path::PathBuf::from(&state.config.console_dir);
    let rel = uri.path().trim_start_matches('/');
    // Traversal guard: reject anything that tries to escape the root.
    if rel.split('/').any(|seg| seg == "..") {
        return err(StatusCode::BAD_REQUEST, "invalid path".into());
    }
    let candidate = if rel.is_empty() {
        root.join("index.html")
    } else {
        root.join(rel)
    };
    let (bytes, served) = match tokio::fs::read(&candidate).await {
        Ok(b) if candidate.is_file() => (b, candidate.clone()),
        // SPA fallback: unknown non-asset paths render the shell client-side.
        _ if !rel.starts_with("assets/") => {
            match tokio::fs::read(root.join("index.html")).await {
                Ok(b) => (b, root.join("index.html")),
                Err(_) => {
                    return err(
                        StatusCode::NOT_FOUND,
                        "console not built — run scripts/build-console.sh on the VM".into(),
                    )
                }
            }
        }
        _ => return err(StatusCode::NOT_FOUND, "asset not found".into()),
    };
    let ct = content_type(&served.to_string_lossy());
    let cache = if served.to_string_lossy().contains("/assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, ct),
            (axum::http::header::CACHE_CONTROL, cache),
        ],
        bytes,
    )
        .into_response()
}

// ---------- SP6B: finance signals (console) ----------

#[derive(Debug, Deserialize)]
struct SignalsParams {
    pattern: Option<String>,
}

#[derive(serde::Deserialize)]
struct HistoryParams {
    series: Option<String>,
    days: Option<i64>,
}

async fn console_signals_history(
    State(state): State<Arc<AppState>>,
    Query(p): Query<HistoryParams>,
) -> Result<Json<Value>, Response> {
    let series = p
        .series
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| hub_err(crate::error::HubError::BadRequest("series is required".into())))?;
    let days = p.days.unwrap_or(40).clamp(1, 400);
    let rows = crate::monitor::signals::series_history(&state, &series, days, 500)
        .await
        .map_err(|e| hub_err(e.into()))?;
    let points: Vec<Value> = rows.iter().map(|(t, v)| json!({"t": t, "v": v})).collect();
    Ok(Json(json!({"series": series, "days": days, "points": points})))
}

/// SP7 sweep delta: per-source direction computed ring-first from the
/// ok-sweeps-only history list (fallback: prev_* counters the scheduler
/// shifts into each health cell — monitor/geo.rs::report_health).
async fn console_monitor_delta(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, Response> {
    let cells: Option<std::collections::HashMap<String, String>> = state
        .redis_timed(
            redis::cmd("HGETALL").arg("hub:monitor:health").clone(),
            2000,
        )
        .await;
    let mut rows: Vec<Value> = Vec::new();
    for (source, raw) in cells.unwrap_or_default() {
        let Ok(cell) = serde_json::from_str::<Value>(&raw) else { continue };
        // Primary baseline: the sweep-history ring (successful sweeps only —
        // immune to restart loss and error-sweep pollution). Fallback: the
        // cell's prev_* fields (first deploys before the ring fills).
        let hist: Option<Vec<String>> = state
            .redis_timed(
                redis::cmd("LRANGE")
                    .arg(format!("hub:monitor:sweephist:{source}"))
                    .arg(0)
                    .arg(9)
                    .clone(),
                2000,
            )
            .await;
        let entries: Vec<Value> = hist
            .unwrap_or_default()
            .iter()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .collect();
        // Ring-first baseline: the ring only records SUCCESSFUL sweeps (failed
        // sweeps update the health cell but never LPUSH), so cell.last_new
        // can be 0-from-an-error-sweep while ring[0] still holds the last ok
        // sweep — mixing the two baselines made direction contradict trend
        // (gdelt 429-recovery exposed this 2026-09-12: cell=0/ring=[23,0..]
        // read as "flat" with an uptrend). Delta semantics = change between
        // the last two OK sweeps; the error state is carried by `state`.
        let new = entries
            .first()
            .and_then(|e| e.get("new").and_then(|v| v.as_i64()))
            .unwrap_or_else(|| cell.get("last_new").and_then(|v| v.as_i64()).unwrap_or(0));
        let fetched = entries
            .first()
            .and_then(|e| e.get("fetched").and_then(|v| v.as_i64()))
            .unwrap_or_else(|| cell.get("last_fetched").and_then(|v| v.as_i64()).unwrap_or(0));
        let prev_new = entries
            .get(1)
            .and_then(|e| e.get("new").and_then(|v| v.as_i64()))
            .or_else(|| cell.get("prev_new").and_then(|v| v.as_i64()));
        let prev_fetched = entries
            .get(1)
            .and_then(|e| e.get("fetched").and_then(|v| v.as_i64()))
            .or_else(|| cell.get("prev_fetched").and_then(|v| v.as_i64()));
        let direction = match prev_new {
            None => "new_source",
            Some(p) if new > p => "up",
            Some(p) if new < p => "down",
            Some(_) => "flat",
        };
        let trend: Vec<i64> = entries
            .iter()
            .rev()
            .filter_map(|e| e.get("new").and_then(|v| v.as_i64()))
            .collect();
        rows.push(json!({
            "source": source,
            "state": cell.get("state").cloned().unwrap_or(json!("unknown")),
            "new": new,
            "fetched": fetched,
            "prev_new": prev_new,
            "prev_fetched": prev_fetched,
            "direction": direction,
            "trend": trend,
            "ts": cell.get("ts").cloned().unwrap_or(Value::Null),
        }));
    }
    rows.sort_by(|a, b| {
        a.get("source").and_then(|v| v.as_str()).cmp(&b.get("source").and_then(|v| v.as_str()))
    });
    Ok(Json(json!({"count": rows.len(), "sources": rows})))
}

async fn console_signals_latest(
    State(state): State<Arc<AppState>>,
    Query(p): Query<SignalsParams>,
) -> Result<Json<Value>, Response> {
    let rows = crate::monitor::signals::latest_observations(&state, p.pattern.as_deref())
        .await
        .map_err(|e| hub_err(e.into()))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|(series, ts, value, payload)| {
            json!({"series": series, "observed_at": ts, "value": value, "payload": payload})
        })
        .collect();
    Ok(Json(json!({"count": items.len(), "series": items})))
}

#[derive(Debug, Deserialize)]
struct WatchlistBody {
    symbol: String,
    asset_class: Option<String>,
    label: Option<String>,
    enabled: Option<bool>,
}

async fn console_watchlist_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, Response> {
    crate::mcp::watchlist_inner(
        &state,
        &crate::mcp::WatchlistManageArgs {
            action: "list".into(),
            symbol: None,
            asset_class: None,
            label: None,
        },
    )
    .await
    .map(Json)
    .map_err(hub_err)
}

async fn console_watchlist_upsert(
    State(state): State<Arc<AppState>>,
    Json(b): Json<WatchlistBody>,
) -> Result<Json<Value>, Response> {
    if let Some(enabled) = b.enabled {
        // explicit enable/disable: direct update (toggle would be racy for UI state)
        let sym = b.symbol.trim().to_uppercase();
        let r = sqlx::query("UPDATE monitor_watchlist SET enabled = $2 WHERE symbol = $1")
            .bind(&sym)
            .bind(enabled)
            .execute(&state.pg)
            .await
            .map_err(crate::error::HubError::from)
            .map_err(|e| hub_err(e.into()))?;
        if r.rows_affected() == 0 {
            return Err(err(StatusCode::NOT_FOUND, format!("watchlist symbol {sym}")));
        }
        return Ok(Json(json!({"ok": true, "symbol": sym, "enabled": enabled})));
    }
    crate::mcp::watchlist_inner(
        &state,
        &crate::mcp::WatchlistManageArgs {
            action: "add".into(),
            symbol: Some(b.symbol),
            asset_class: b.asset_class,
            label: b.label,
        },
    )
    .await
    .map(Json)
    .map_err(hub_err)
}

async fn console_watchlist_remove(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(symbol): axum::extract::Path<String>,
) -> Result<Json<Value>, Response> {
    crate::mcp::watchlist_inner(
        &state,
        &crate::mcp::WatchlistManageArgs {
            action: "remove".into(),
            symbol: Some(symbol),
            asset_class: None,
            label: None,
        },
    )
    .await
    .map(Json)
    .map_err(hub_err)
}

/// D: trace propagation — walk the full call graph for one MCP request.
/// Returns cost_records + embedding_jobs + documents joined by trace_id.
/// Useful for "where did this result come from?" debugging.
async fn get_trace(
    State(state): State<Arc<AppState>>,
    Path(trace_id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    let cost: Vec<(String, f64, String, serde_json::Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT kind, amount, unit, detail, created_at FROM cost_records
         WHERE trace_id = $1 ORDER BY created_at ASC",
    )
    .bind(trace_id)
    .fetch_all(&state.pg)
    .await
    .map_err(|e| hub_err(e.into()))?;
    let jobs: Vec<(Uuid, Uuid, String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT job_id, document_id, status, reason, created_at FROM embedding_jobs
         WHERE trace_id = $1 ORDER BY created_at ASC",
    )
    .bind(trace_id)
    .fetch_all(&state.pg)
    .await
    .map_err(|e| hub_err(e.into()))?;
    let mut documents = serde_json::Map::new();
    for (_job_id, doc_id, status, reason, _created) in &jobs {
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT url_canonical, title FROM documents WHERE document_id = $1",
        )
        .bind(doc_id)
        .fetch_optional(&state.pg)
        .await
        .map_err(|e| hub_err(e.into()))?;
        if let Some((url, title)) = row {
            documents.insert(
                doc_id.to_string(),
                json!({ "url": url, "title": title, "status": status, "reason": reason }),
            );
        }
    }
    let tool_calls: Vec<(String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT tool, status, created_at FROM tool_calls
         WHERE trace_id = $1 ORDER BY created_at ASC",
    )
    .bind(trace_id.to_string())
    .fetch_all(&state.pg)
    .await
    .map_err(|e| hub_err(e.into()))?;
    Ok(Json(json!({
        "trace_id": trace_id,
        "cost_records": cost.into_iter().map(|(k, a, u, d, t)| json!({
            "kind": k, "amount": a, "unit": u, "detail": d, "at": t,
        })).collect::<Vec<_>>(),
        "embedding_jobs": jobs.into_iter().map(|(j, d, s, r, t)| json!({
            "job_id": j, "document_id": d, "status": s, "reason": r, "at": t,
        })).collect::<Vec<_>>(),
        "documents": documents,
        "tool_calls": tool_calls.into_iter().map(|(t, s, at)| json!({
            "tool": t, "status": s, "at": at,
        })).collect::<Vec<_>>(),
    })))
}
// ---------- SP9: knowledge graph REST API (read-side, backs console) ----------

#[derive(Deserialize)]
struct NeighborsQuery {
    root: String,
    depth: Option<u8>,
    at_time: Option<String>,
    /// SP10: comma-separated list of relationship types to include
    /// (e.g. "located_in,affiliated_with"). Empty/missing = no filter.
    rel_types: Option<String>,
    /// SP10: minimum edge confidence threshold (0.0..=1.0). Default 0.0.
    min_confidence: Option<f64>,
}

async fn graph_neighbors(
    State(state): State<Arc<AppState>>,
    Query(q): Query<NeighborsQuery>,
) -> impl IntoResponse {
    let entity_id = match Uuid::parse_str(&q.root) {
        Ok(x) => x,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("{e}")),
    };
    let at_time = q
        .at_time
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    let depth = q.depth.unwrap_or(2);
    // SP10: parse comma-separated rel_types and forward to backend.
    let rel_types: Option<Vec<String>> = q
        .rel_types
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());
    let min_confidence = q.min_confidence;
    match crate::graph_queries::get_neighbors(
        &state,
        entity_id,
        depth,
        rel_types,
        min_confidence,
        at_time,
    )
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
    }
}

#[derive(Deserialize)]
struct EntitiesSearchQuery {
    q: Option<String>,
    kind: Option<String>,
    limit: Option<i64>,
}

/// SP9 entity discovery endpoint — backs the /graph page entity picker.
/// When `q` is empty/missing, returns the most-recently-created entities
/// (used as the default landing view of the /graph console page so users
/// see actual content immediately, not an empty skeleton).
async fn graph_entities_search(
    State(state): State<Arc<AppState>>,
    Query(q): Query<EntitiesSearchQuery>,
) -> impl IntoResponse {
    let lim = q.limit.unwrap_or(50).clamp(1, 200);
    let result = if let Some(name) = q.q.as_deref().filter(|s| !s.trim().is_empty()) {
        crate::graph_queries::search_entity(&state, name, q.kind.as_deref(), lim).await
    } else {
        // Default landing view: most recently created entities grouped by kind.
        // SP10 will swap this for a richer "world" picker with degree counts.
        let pool = &state.pg;
        let rows: Result<Vec<(Uuid, String, String, f64)>, _> = sqlx::query_as(
            "SELECT entity_id, kind, name, \
                    (CASE WHEN lower(name) ~ '^(brics|china|russia|iran|saudi|emirates|putin|xi)' \
                          THEN 0.9 ELSE 0.5 END)::float8 AS score \
               FROM entities \
              WHERE merged_into IS NULL \
              ORDER BY created_at DESC NULLS LAST \
              LIMIT $1",
        )
        .bind(lim)
        .fetch_all(pool)
        .await;
        match rows {
            Ok(rs) => Ok(json!(rs.into_iter().map(|(id, k, n, s)| json!({
                "entity_id": id, "kind": k, "name": n, "aliases": json!([]), "score": s
            })).collect::<Vec<_>>())),
            Err(e) => Err(crate::error::HubError::Internal(format!("entity landing query: {e}"))),
        }
    };
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
    }
}

#[derive(Deserialize)]
struct TimelineQuery {
    from: Option<String>,
    to: Option<String>,
}

async fn graph_entity_timeline(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TimelineQuery>,
) -> impl IntoResponse {
    let from = q
        .from
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    let to = q
        .to
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    match crate::graph_queries::get_entity_timeline(&state, id, from, to, 100).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
    }
}

#[derive(Deserialize)]
struct PathQuery {
    from: String,
    to: String,
    max_hops: Option<u8>,
}

async fn graph_path(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PathQuery>,
) -> impl IntoResponse {
    let max_hops = q.max_hops.unwrap_or(5);
    match crate::graph_queries::find_path(&state, &q.from, &q.to, true, max_hops).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
    }
}

async fn graph_investigation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match crate::graph_queries::query_investigation_graph(&state, id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
    }
}

#[derive(Deserialize)]
struct EvidenceQuery {
    entity: String,
    relation: Option<String>,
}

async fn graph_evidence(
    State(state): State<Arc<AppState>>,
    Query(q): Query<EvidenceQuery>,
) -> impl IntoResponse {
    let entity_id = match Uuid::parse_str(&q.entity) {
        Ok(x) => x,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("{e}")),
    };
    match crate::graph_queries::list_evidence_for_entity(&state, entity_id, q.relation.as_deref()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
    }
}
