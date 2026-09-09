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

    // SP4 components (§68): crucix signal layer + observability stack.
    let crucix = probe(|| async {
        let url = format!("{}/api/health", state.config.crucix_url);
        matches!(state.http.get(&url).send().await, Ok(r) if r.status().is_success())
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
            "crucix": crucix,
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
}

async fn create_investigation(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Json(body): Json<CreateInvestigationBody>,
) -> Result<Json<Value>, Response> {
    // SP2B: REST goes through the same governance gate as MCP.
    crate::policy::preflight(&state, &agent, "create_investigation")
        .await
        .map_err(hub_err)?;
    let id = crate::store::create_investigation(
        &state.pg,
        &body.title,
        body.question.as_deref(),
        body.target.as_deref(),
        body.hypothesis.as_deref(),
        &format!("agent:{}", agent.name),
    )
    .await
    .map_err(hub_err)?;
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
    Ok(Json(json!({ "investigation_id": id })))
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
        .map_err(hub_err)?;
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
        .map_err(hub_err)?;
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
    )
    .await
    .map_err(hub_err)?;
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
