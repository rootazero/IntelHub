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
        other => err(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

// ---------- health aggregation (shared with MCP get_system_health) ----------

async fn probe<Fut>(f: impl FnOnce() -> Fut) -> Value
where
    Fut: std::future::Future<Output = bool>,
{
    let t = Instant::now();
    let ok = f().await;
    json!({ "status": if ok { "up" } else { "down" }, "latency_ms": t.elapsed().as_millis() as i64 })
}

pub async fn system_health(state: &AppState) -> Value {
    let pg = probe(|| async {
        sqlx::query("SELECT 1").execute(&state.pg).await.is_ok()
    })
    .await;

    let mut conn = state.redis.clone();
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
        },
        "containers": containers,
    })
}

/// Best-effort docker container states (native hub runs as zou ∈ docker group).
async fn docker_containers() -> Value {
    let out = tokio::process::Command::new("docker")
        .args(["ps", "-a", "--format", "{{json .}}"])
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => {
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
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateInvestigationBody>,
) -> Result<Json<Value>, Response> {
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
    crate::store::get_document(&state.pg, id).await.map(Json).map_err(hub_err)
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
