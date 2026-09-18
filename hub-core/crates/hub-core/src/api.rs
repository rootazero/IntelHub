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
use crate::types::{AgentIdentity, RequestTrace};
use axum::Extension;

// GEV P8: `/api/v1/annotations/*` lives in its own module (submodule of this
// file → `src/api/annotations.rs`) because it owns a route table + a
// pool-scoped handler-local Bearer guard.
pub mod annotations;

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
        .route("/api/v1/series", get(list_series))
        .route("/api/v1/tasks", get(console_tasks))
        .route("/api/v1/investigations/{id}/workspace", get(console_workspace))
        // SP5
        .route("/api/v1/evidence", axum::routing::post(post_evidence))
        // SP4
        .route("/api/v1/radar/events", get(console_radar_events))
        // Globe P1 (read-only, auth middleware inherited)
        .route("/api/v1/globe/aircraft", get(console_globe_aircraft))
        .route("/api/v1/globe/satellites", get(console_globe_satellites))
        // GEV P2: earthquake layer source (geo_events kind='quake')
        .route("/api/v1/gev/earthquakes", get(gev_earthquakes))
        // GEV P2: satellites layer source (PG TLE catalog + starlink proxy)
        .route("/api/v1/gev/celestrak/{group}", get(gev_celestrak))
        // GEV P3: traffic layer sources (overpass proxy + tomtom flow tiles)
        .route("/api/v1/gev/overpass", axum::routing::post(crate::gev_traffic::gev_overpass))
        .route("/api/v1/gev/tomtom/status", get(crate::gev_traffic::gev_tomtom_status))
        // axum forbids two params in one segment, so the route drops the
        // engine's ".pbf" suffix (the console adapter strips it in the
        // prefix rewrite — traffic.ts rewriteTrafficPath).
        .route("/api/v1/gev/tomtom/flow/{z}/{x}/{y}", get(crate::gev_traffic::gev_tomtom_flow))
        // GEV P3: military installations layer source (PG catalog → Overpass elements)
        .route("/api/v1/gev/installations", get(crate::gev_installations::gev_installations))
        // GEV P3: vessels layer source (AIS live snapshot + per-MMSI track)
        .route("/api/v1/gev/ais-live", get(crate::gev_vessels::gev_ais_live))
        .route("/api/v1/gev/ais-live/track", get(crate::gev_vessels::gev_ais_live_track))
        // GEV P3: cctv layer source (camera catalog + probe health; frames/media proxied at T12)
        .route("/api/v1/gev/cctv/sources", get(crate::gev_cctv::gev_cctv_sources))
        .route("/api/v1/gev/cctv/health", get(crate::gev_cctv::gev_cctv_health))
        // GEV P3 T12: frame proxy (10s cache) + mp4 media stream (cap 4, 429);
        // hls → 501 until the P4 playlist proxy. SSRF-impossible: ids only.
        .route("/api/v1/gev/cctv/frame/{id}", get(crate::gev_cctv::gev_cctv_frame))
        .route("/api/v1/gev/cctv/media/{id}", get(crate::gev_cctv::gev_cctv_media))
        // GEV P7: location search geocode proxy (photon, keyless, cached 1h)
        .route("/api/v1/gev/geocode", get(crate::gev_geocode::gev_geocode))
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
        // graph_v2 write-side: batch claim extractor (calls emit_claim_audit)
        .route("/api/v1/v2/extract_claims", axum::routing::post(crate::graph_v2::extract::extract_claims))
        // GEV P8: persisted globe annotations (pin/line/area), CRUD + bbox query.
        // Write verbs carry a handler-local Bearer guard on top of the global
        // auth middleware (see api/annotations.rs module docs).
        .merge(crate::api::annotations::router::<Arc<AppState>>())
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
    // OSINT Framework bridge (2026-09-16): structured per-collector health
    // for the 5 collectors that fill the osintframework.com gaps. Hardcoded
    // list — adding a new bridge collector is a deliberate registry change,
    // not a data-driven one. Each entry reads its Redis cell and surfaces
    // cadence_hours so the console Monitor page and accept-sp6.py can render
    // "next sweep ETA" without a separate config call. Cheap (~5 redis
    // HGETs) and only runs as part of system_health().
    let osint_bridge_collectors: &[(&str, u32, &str)] = &[
        ("etherscan",  6, "financial"),
        ("defillama",  4, "financial"),
        ("otx",        4, "cyber"),
        ("urlscan",    4, "cyber"),
        ("gfw",       12, "transport"),
        ("overpass",  24, "geolocation"),
        ("ahmia",     12, "cyber"),
        ("opencorp",  24, "financial"),
        ("wikidata",  24, "financial"),
        ("courtlistener", 12, "sanction"),
        ("leaksify",     12, "cyber"),
        // SP8-E cyber plane collectors (already in registry) now surfaced
        // in the bridge JSON: cisakev (KEV — actively exploited),
        // nvd (full CVE corpus with CVSS v3), osv (open-source ecosystem
        // vulns), opensanctions (compliance plane, shelved-by-design
        // without API key).
        ("cisa-kev",      6, "cyber"),
        ("nvd",           6, "cyber"),
        ("osv",           6, "cyber"),
        ("opensanctions", 24, "sanction"),
        // OSINT Framework bridge 4 (2026-09-17): tor_exit (Tor exit-node
        // daily bulk list) + ipsum (stamparm threat-IP aggregator).
        ("tor_exit",     24, "cyber"),
        ("ipsum",        12, "cyber"),
        // OSINT Framework bridge 5 (2026-09-17): crt.sh (cert
        // transparency log search, keyless) + OpenPhish (phishing
        // URL catalog, keyless — replaces PhishTank's retired
        // public feed) + Shodan InternetDB (per-IP CPE / port /
        // vuln enrichment, keyless).
        ("crtsh",                6, "cyber"),
        ("openphish",            4, "cyber"),
        ("shodan_internetdb",   12, "cyber"),
        // OSINT Framework bridge 6 (2026-09-17): Spamhaus DROP
        // (authoritative netblock blocklist) + blocklist.de (German
        // fail2ban community per-attack-type IP blocklists). Both
        // free + keyless, no registration required. RDAP originally
        // planned but Verisign RDAP rejects reqwest <-> Verisign
        // interop from datacenter egress (HTTP 400); tracked
        // separately.
        ("spamhaus_drop",    24, "cyber"),
        ("blocklist_de",     12, "cyber"),
        // OSINT Framework bridge 7 (2026-09-17): three free
        // keyless collectors — Nominatim (OSM geocoding for
        // threat-actor HQ anchoring) + RIPEstat (RIPE
        // abuse-contact lookup for incident response) +
        // Wayback Machine (URL archive snapshots for
        // phishing forensics).
        ("nominatim",       24, "cyber"),
        ("ripestat",        24, "cyber"),
        ("wayback",         12, "cyber"),
        // OSINT Framework bridge 8 (2026-09-17): three free
        // keyless network-attribution collectors —
        // aws_ip_ranges (AWS public IP-range feed for
        // cloud-IP attribution) + gcp_ip_ranges (GCP
        // public IP-range feed) + ripe_as_overview (RIPE
        // stat per-ASN holder lookup for AS-topology
        // attribution).
        ("aws_ip_ranges",   24, "cyber"),
        ("gcp_ip_ranges",   24, "cyber"),
        ("ripe_as_overview",24, "cyber"),
        // OSINT Framework bridge 9 (2026-09-17): three free
        // keyless per-IP attribution collectors —
        // ipapi_co (rich IP metadata: city/country/lat/lon/
        // ASN/org) + ip_api_com (IP geolocation + ASN/ISP,
        // redundant with ipapi_co for cross-validation) +
        // ripe_prefix_overview (RIPE stat per-prefix BGP
        // info: prefix, AS path, RPKI status).
        ("ipapi_co",        24, "cyber"),
        ("ip_api_com",      24, "cyber"),
        ("ripe_prefix_overview", 24, "cyber"),
        // OSINT Framework bridge 10 (2026-09-17): three free
        // keyless sentinel / active-threat / curated-
        // blocklist collectors — misp_dynamic_dns (MISP
        // dynamic-DNS sentinel list for OSINT false-positive
        // suppression) + urlhaus (abuse.ch malware URL feed)
        // + firehol_level1 (FireHOL curated IP blocklist).
        ("misp_dynamic_dns", 24, "cyber"),
        ("urlhaus",          4, "cyber"),
        ("firehol_level1",  12, "cyber"),
        // OSINT Framework bridge 11 (2026-09-17): three free
        // keyless sentinel + rich-Tor collectors —
        // misp_rfc5735 (RFC 5735 Special-Use IPv4 sentinel)
        // + misp_rfc6761 (RFC 6761 Special-Use TLD sentinel)
        // + tor_exit_details (rich Tor exit-addresses feed
        // with fingerprint + timestamps, complementary to
        // the existing tor_exit which uses torbulkexitlist).
        ("misp_rfc5735",    24, "cyber"),
        ("misp_rfc6761",    24, "cyber"),
        ("tor_exit_details",12, "cyber"),
        // OSINT Framework bridge 12 (2026-09-17): three free
        // keyless collectors — romainmarcoux_malicious_ip
        // (40K most-malicious IPs aggregator, metadata-
        // pattern) + ihr_hegemony (IIJ Lab AS hegemony API
        // for internet-topology shift detection — default
        // watch Cloudflare + Akamai) + misp_second_level_tlds
        // (MISP sentinel of 10,315 Mozilla-PSL 2nd-level TLDs).
        ("romainmarcoux_malicious_ip", 24, "cyber"),
        ("ihr_hegemony",               24, "cyber"),
        ("misp_second_level_tlds",     24, "cyber"),
    ];
    let mut osint_bridge: Vec<Value> = Vec::with_capacity(osint_bridge_collectors.len());
    for (name, cadence_hours, kind) in osint_bridge_collectors {
        let cell: Option<String> = state
            .redis_timed(redis::cmd("HGET").arg("hub:monitor:health").arg(*name).clone(), 1500)
            .await;
        let entry = match cell {
            Some(raw) => {
                let parsed: Option<Value> = serde_json::from_str(&raw).ok();
                let state_str = parsed
                    .as_ref()
                    .and_then(|p| p.get("state"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let last_fetched = parsed
                    .as_ref()
                    .and_then(|p| p.get("last_fetched"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
                let last_new = parsed
                    .as_ref()
                    .and_then(|p| p.get("last_new"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
                let ts = parsed
                    .as_ref()
                    .and_then(|p| p.get("ts"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                json!({
                    "name": name,
                    "kind": kind,
                    "cadence_hours": cadence_hours,
                    "state": state_str,
                    "last_fetched": last_fetched,
                    "last_new": last_new,
                    "ts": ts,
                })
            }
            None => json!({
                "name": name,
                "kind": kind,
                "cadence_hours": cadence_hours,
                "state": "absent",
                "last_fetched": 0,
                "last_new": 0,
                "ts": "",
            }),
        };
        osint_bridge.push(entry);
    }
    let osint_bridge_present = osint_bridge.iter()
        .filter(|c| c.get("state").and_then(|s| s.as_str()) == Some("ok"))
        .count();
    let osint_bridge = json!({
        "summary": {
            "present": osint_bridge_present,
            "total": osint_bridge.len(),
        },
        "collectors": osint_bridge,
    });
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
        "osint_bridge": osint_bridge,
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
    Extension(agent): Extension<AgentIdentity>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let tier = agent.tier;
    let stream = futures::stream::unfold(rx, move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    // Tier isolation: never stream bus events for a monitor
                    // source above the subscriber's tier.
                    if !crate::access::bus_event_allowed(tier, &ev) {
                        continue;
                    }
                    let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    return Some((Ok(SseEvent::default().event(ev.event_type.clone()).data(data)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    return Some((Ok(SseEvent::default().event("LAGGED").data(format!("{n} events dropped"))), rx))
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(25)).text("keepalive"))
}

async fn events_recent(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Query(p): Query<ListParams>,
) -> Result<Json<Value>, Response> {
    let mut events = crate::store::recent_events(&state.pg, p.limit.unwrap_or(50).clamp(1, 200))
        .await
        .map_err(hub_err)?;
    // The `events` bus table has no tier_required column; drop monitor bus
    // events whose source exceeds the caller's tier and recompute `count`.
    crate::access::filter_bus_items(&mut events, agent.tier);
    Ok(Json(events))
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

async fn console_overview(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
) -> Result<Json<Value>, Response> {
    crate::console::overview(&state, agent.tier).await.map(Json).map_err(hub_err)
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
    Extension(agent): Extension<AgentIdentity>,
    Extension(trace): Extension<RequestTrace>,
    Query(p): Query<RadarParams>,
) -> Result<Json<Value>, Response> {
    let trace_id = Uuid::parse_str(&trace.trace_id).ok();
    crate::console::radar_events(
        &state,
        p.from.as_deref(),
        p.to.as_deref(),
        p.severity.as_deref(),
        p.source.as_deref(),
        p.kind.as_deref(),
        p.limit.unwrap_or(500).min(2000),
        &agent,
        trace_id,
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

// ---------- Globe P1: live layers ----------

async fn console_globe_aircraft(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, Response> {
    // Ruling 3 (GEV P2 T13): two independent snapshots — adsb.lol rotating
    // (`hub:globe:aircraft`, TTL 300s) and OpenSky OAuth full vectors
    // (`hub:globe:aircraft:opensky`, TTL 120s) — merged by hex at read time
    // (fresher age_s wins; adsb priority without age info).
    let adsb_blob: Option<String> = state
        .redis_timed(
            redis::cmd("GET")
                .arg(crate::monitor::sources::adsb::AIRCRAFT_KEY)
                .clone(),
            2000,
        )
        .await;
    let opensky_blob: Option<String> = state
        .redis_timed(
            redis::cmd("GET")
                .arg(crate::monitor::sources::opensky::OPENSKY_AIRCRAFT_KEY)
                .clone(),
            2000,
        )
        .await;
    let adsb = adsb_blob.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let opensky = opensky_blob.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    // Both missing/corrupt = both collectors dead (adsb >300s, opensky >120s).
    // 200 + stale flag, never 5xx — the frontend degrades visibly (spec §5).
    Ok(Json(crate::monitor::sources::adsb::merge_globe_snapshots(
        adsb.as_ref(),
        opensky.as_ref(),
    )))
}

#[derive(Debug, Deserialize)]
struct GlobeSatParams {
    category: Option<String>,
}

async fn console_globe_satellites(
    State(state): State<Arc<AppState>>,
    Query(p): Query<GlobeSatParams>,
) -> Result<Json<Value>, Response> {
    let cats: Option<Vec<String>> = p.category.as_deref().map(|c| {
        c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    });
    type Row = (i32, String, String, String, String, chrono::DateTime<chrono::Utc>);
    let result = match &cats {
        Some(c) => sqlx::query_as::<_, Row>(
            "SELECT norad_id, name, category, tle_line1, tle_line2, epoch FROM satellites WHERE category = ANY($1) ORDER BY category, name",
        )
        .bind(c)
        .fetch_all(&state.pg)
        .await,
        None => sqlx::query_as::<_, Row>(
            "SELECT norad_id, name, category, tle_line1, tle_line2, epoch FROM satellites ORDER BY category, name",
        )
        .fetch_all(&state.pg)
        .await,
    };
    let rows = result.map_err(|e| hub_err(crate::error::HubError::sensor(format!("globe satellites: {e}"))))?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(norad_id, name, category, tle1, tle2, epoch)| {
            json!({ "norad_id": norad_id, "name": name, "category": category, "tle1": tle1, "tle2": tle2, "epoch": epoch })
        })
        .collect();
    Ok(Json(json!({ "count": items.len(), "items": items })))
}

// ---------- GEV P2: earthquake layer source (geo_events kind='quake') ----------

/// Map one `geo_events` quake row to the row shape the vendored GEV earthquakes
/// layer destructures — `{stableId, usgsId, lon, lat, depthKm, mag, place, time}`
/// (console/gev-engine/src/layers/earthquakes/index.js:86, mirrors
/// `normalizeEarthquakeSnapshot` in layers/earthquakes/records.js).
///
/// `payload` is the raw USGS `properties` object (usgs.rs `Signal::payload`), so
/// it carries `mag`/`place`/`time` (ms epoch) but NOT `depthKm`: depth lives in
/// GeoJSON `geometry.coordinates[2]` and was never copied into properties. Hence
/// `depthKm` is null for every USGS row — the engine tolerates it
/// (`depthColor(depthKm || 0)`, `num(raw?.depth) -> null`).
///
/// `time` is payload `time` when present, else `occurred_at` parsed to ms epoch,
/// else null — the field is number|null in every branch (the engine reads it as
/// USGS epoch ms via `num()`), so it never leaks an ISO string.
fn quake_row(
    stable_id: String,
    usgs_id: String,
    lon: f64,
    lat: f64,
    payload: Option<Value>,
    occurred: String,
) -> Value {
    let p = payload.unwrap_or_default();
    let time = p.get("time").cloned().unwrap_or_else(|| {
        chrono::DateTime::parse_from_rfc3339(&occurred)
            .map(|d| json!(d.timestamp_millis()))
            .unwrap_or(Value::Null)
    });
    json!({
        "stableId": stable_id,
        "usgsId": usgs_id,
        "lon": lon,
        "lat": lat,
        "depthKm": p.get("depthKm").cloned().unwrap_or(Value::Null),
        "mag": p.get("mag").cloned().unwrap_or(Value::Null),
        "place": p.get("place").cloned().unwrap_or(Value::Null),
        "time": time,
    })
}

async fn gev_earthquakes(State(state): State<Arc<AppState>>) -> Result<Json<Value>, Response> {
    // external_id is the USGS feature id (stableId + usgsId are the same value:
    // IntelHub has no second id for a quake, and the engine uses stableId only as
    // the Cesium entity key / dedupe key).
    // T4 review carry-forward (T5 commit): every row is M2.5+ by construction
    // of the upstream USGS `2.5_day.geojson` feed — THAT upstream contract is
    // what guarantees magnitude, not any filtering here or in the engine (the
    // `mag < 2.5` discard in the engine's records.js is only a second gate).
    // The NOT NULL guard rejects rows whose payload lost `mag` upstream, which
    // the engine would drop as unverifiable anyway.
    type Row = (String, String, f64, f64, Option<Value>, String);
    let rows = sqlx::query_as::<_, Row>(
        r#"SELECT external_id, external_id, lon, lat, payload,
                  to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
             FROM geo_events
            WHERE kind = 'quake'
              AND occurred_at > now() - interval '24 hours'
              AND lat IS NOT NULL AND lon IS NOT NULL
              AND payload->>'mag' IS NOT NULL
            ORDER BY occurred_at DESC
            LIMIT 500"#,
    )
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        hub_err(crate::error::HubError::sensor(format!(
            "gev earthquakes query: {e}"
        )))
    })?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(stable_id, usgs_id, lon, lat, payload, occurred)| {
            quake_row(stable_id, usgs_id, lon, lat, payload, occurred)
        })
        .collect();
    Ok(Json(Value::Array(out)))
}

// ---------- GEV P2: satellites layer source (PG TLE catalog + starlink proxy) ----------

/// GEV six core groups — mirrors the celestrak collector's default
/// `celestrak_groups()` and the engine's CATALOG_GROUPS
/// (gev-engine/src/layers/satellites/policy.js). starlink is served via proxy,
/// not from PG (see `starlink_tle`).
const GEV_TLE_GROUPS: &[&str] = &["stations", "visual", "gps-ops", "glo-ops", "galileo", "geo"];

const STARLINK_TLE_KEY: &str = "hub:globe:tle:starlink";
const STARLINK_TLE_URL: &str =
    "https://celestrak.org/NORAD/elements/gp.php?GROUP=starlink&FORMAT=tle";
const STARLINK_TLE_TTL_SECS: u64 = 6 * 3600;

/// Three-line TLE block per record (name / line1 / line2), blocks joined by a
/// single newline with a trailing newline — byte-compatible with CelesTrak's
/// own gp.php?FORMAT=tle layout, which is what the engine's `parseTLE` consumes.
fn tle_text(rows: &[(String, String, String)]) -> String {
    rows.iter()
        .map(|(n, a, b)| format!("{n}\n{a}\n{b}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn tle_plain(text: String) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        text,
    )
        .into_response()
}

/// HTTP client for monitor-style upstream calls from the REST layer —
/// identical config to `monitor::Ctx::new` (the celestrak collector's fetch
/// path): browser UA (celestrak.org sits behind Cloudflare; honest bot UAs get
/// challenged) + 25s timeout. Deliberately NOT `state.http`, which carries the
/// honest `intelhub-core` UA for our own API peers.
fn monitor_http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(25))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36")
            .build()
            .expect("monitor http client build")
    })
}

async fn gev_celestrak(
    State(state): State<Arc<AppState>>,
    Path(group): Path<String>,
) -> Result<Response, Response> {
    if group == "starlink" {
        return starlink_tle(&state).await;
    }
    if !GEV_TLE_GROUPS.contains(&group.as_str()) {
        return Err(err(StatusCode::NOT_FOUND, "unknown group".into()));
    }
    type Row = (String, String, String);
    let rows = sqlx::query_as::<_, Row>(
        "SELECT name, tle_line1, tle_line2 FROM satellites WHERE category = $1 ORDER BY name",
    )
    .bind(&group)
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        hub_err(crate::error::HubError::sensor(format!(
            "gev celestrak {group}: {e}"
        )))
    })?;
    Ok(tle_plain(tle_text(&rows)))
}

/// starlink is deliberately absent from the PG catalog (P1 spec decision,
/// carried into GEV P2): the ~7000-sat constellation is too volatile for the
/// 6h per-category full-replace cadence and would dwarf the six core groups.
/// Serve it as a thin proxied passthrough cached in Redis for one collector
/// interval. Upstream failure passes through as 502 and never poisons the cache.
async fn starlink_tle(state: &AppState) -> Result<Response, Response> {
    let cached: Option<String> = state
        .redis_timed(redis::cmd("GET").arg(STARLINK_TLE_KEY).clone(), 2000)
        .await;
    if let Some(text) = cached {
        return Ok(tle_plain(text));
    }
    let resp = monitor_http().get(STARLINK_TLE_URL).send().await;
    let text = match resp {
        Ok(r) if r.status().is_success() => match r.text().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "celestrak starlink body read failed");
                return Err(err(StatusCode::BAD_GATEWAY, "celestrak upstream failed".into()));
            }
        },
        Ok(r) => {
            tracing::warn!(status = %r.status(), "celestrak starlink upstream failed");
            return Err(err(StatusCode::BAD_GATEWAY, "celestrak upstream failed".into()));
        }
        Err(e) => {
            tracing::warn!(error = %e, "celestrak starlink upstream failed");
            return Err(err(StatusCode::BAD_GATEWAY, "celestrak upstream failed".into()));
        }
    };
    // A 200 with no parseable TLE triples is an upstream anomaly (error page,
    // format change) — treat it like a failure and do NOT cache it.
    if crate::monitor::sources::celestrak::parse_tle_catalog(&text).is_empty() {
        tracing::warn!(bytes = text.len(), "celestrak starlink returned no TLE triples");
        return Err(err(StatusCode::BAD_GATEWAY, "celestrak upstream failed".into()));
    }
    let _: Option<String> = state
        .redis_timed(
            redis::cmd("SETEX")
                .arg(STARLINK_TLE_KEY)
                .arg(STARLINK_TLE_TTL_SECS)
                .arg(&text)
                .clone(),
            2000,
        )
        .await;
    Ok(tle_plain(text))
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
        Some("wasm") => "application/wasm",
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

// ---------- SP-series-catalog: GET /api/v1/series ----------

#[derive(Debug, Deserialize)]
struct SeriesQuery {
    source: Option<String>,
    unit: Option<String>,
}

/// Returns the static series catalog. No auth required — this is
/// metadata only (descriptors, no values). MCP agents, frontend
/// authors, and external tools discover series IDs here.
///
/// Optional filters:
///   ?source=fred       — limit to one source
///   ?unit=Percent      — limit to one unit
async fn list_series(Query(q): Query<SeriesQuery>) -> Json<Value> {
    use crate::series::{Source, Unit, CATALOG};

    let src_filter = q.source.as_deref().and_then(|s| match s {
        "fred" => Some(Source::Fred),
        "eia" => Some(Source::Eia),
        "treasury" => Some(Source::Treasury),
        "comtrade" => Some(Source::Comtrade),
        "gscpi" => Some(Source::Gscpi),
        "nasa" => Some(Source::Nasa),
        "noaa" => Some(Source::Noaa),
        "quote" => Some(Source::Quote),
        "sentiment" => Some(Source::Sentiment),
        _ => None,
    });
    let unit_filter = q.unit.as_deref().and_then(|u| match u {
        "Percent" => Some(Unit::Percent),
        "Usd" => Some(Unit::Usd),
        "Bbl" => Some(Unit::Bbl),
        "K" => Some(Unit::K),
        "YoyPct" => Some(Unit::YoyPct),
        "SpotUsdBbl" => Some(Unit::SpotUsdBbl),
        "AnomC" => Some(Unit::AnomC),
        "Ppm" => Some(Unit::Ppm),
        "Index" => Some(Unit::Index),
        "Count" => Some(Unit::Count),
        "Symbol" => Some(Unit::Symbol),
        _ => None,
    });

    let filtered: Vec<_> = CATALOG
        .iter()
        .filter(|d| src_filter.is_none_or(|s| d.source == s))
        .filter(|d| unit_filter.is_none_or(|u| d.unit == u))
        .map(|d| {
            json!({
                "source": d.source.prefix(),
                "upstream_id": d.upstream_id,
                "normalized_id": d.normalized_id,
                "display_name": d.display_name,
                "unit": format!("{:?}", d.unit),
                "description": d.description,
            })
        })
        .collect();

    Json(json!({
        "count": filtered.len(),
        "series": filtered,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quake_row_maps_geo_event() {
        let ev = serde_json::json!({"mag": 5.1, "place": "10km S of X", "time": 1758000000000i64});
        let row = quake_row("ext-1".into(), "ext-1".into(), 140.1, 35.2, Some(ev), "2026-09-17T00:00:00Z".into());
        assert_eq!(row["mag"], 5.1);
        assert_eq!(row["stableId"], "ext-1");
        assert_eq!(row["time"], 1758000000000i64);
        assert!(row["depthKm"].is_null());
    }

    #[test]
    fn quake_row_keeps_contract_shape_for_null_payload() {
        // jsonb is NOT NULL in geo_events, but the mapping must still produce
        // every contract key (never `undefined`) if a legacy/empty payload lands.
        let row = quake_row("us7000abcd".into(), "us7000abcd".into(), -117.0, 34.1, None, "2026-09-17T00:00:00Z".into());
        for key in [
            "stableId", "usgsId", "lon", "lat", "depthKm", "mag", "place", "time",
        ] {
            assert!(row.get(key).is_some(), "missing key {key}");
        }
        assert!(row["mag"].is_null());
        assert!(row["place"].is_null());
        assert!(row["depthKm"].is_null());
        // `time` falls back to occurred_at as ms epoch, never an ISO string.
        assert_eq!(row["time"], 1789603200000i64);
    }

    #[test]
    fn quake_row_passes_through_payload_fields() {
        // depthKm is absent from USGS properties; when a producer does supply it
        // the mapping must not silently drop it.
        let ev = serde_json::json!({"mag": 6.2, "place": "off Honshu", "time": 1757500000000i64, "depthKm": 40.5});
        let row = quake_row("us1".into(), "us1".into(), 142.3, 38.3, Some(ev), "2026-09-17T00:00:00Z".into());
        assert_eq!(row["lon"], 142.3);
        assert_eq!(row["lat"], 38.3);
        assert_eq!(row["depthKm"], 40.5);
        assert_eq!(row["usgsId"], "us1");
        assert_eq!(row["mag"], 6.2);
        assert_eq!(row["place"], "off Honshu");
    }

    #[test]
    fn tle_text_three_line_blocks() {
        let s = tle_text(&[("ISS".into(), "1 ..".into(), "2 ..".into())]);
        assert_eq!(s, "ISS\n1 ..\n2 ..\n");
        // Byte-compatible with CelesTrak gp.php?FORMAT=tle layout.
        let s = tle_text(&[
            ("A".into(), "1 a".into(), "2 a".into()),
            ("B".into(), "1 b".into(), "2 b".into()),
        ]);
        assert_eq!(s, "A\n1 a\n2 a\nB\n1 b\n2 b\n");
    }
}

