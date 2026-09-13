//! MCP Gateway: rmcp Streamable HTTP. Agent-neutral capability tools
//! (directive §32). Identity is read from request extensions (inserted by
//! the auth middleware — unspoofable); every call is recorded in tool_calls.

use std::sync::Arc;
use std::time::Instant;

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    service::{NotificationContext, RequestContext},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::state::AppState;
use crate::types::{AgentIdentity, BusEvent, RequestTrace};

#[derive(Clone)]
pub struct HubMcp {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

// ---------- helpers ----------

fn agent_of_ext(ext: &Extensions) -> AgentIdentity {
    ext.get::<axum::http::request::Parts>()
        .and_then(|p| p.extensions.get::<AgentIdentity>().cloned())
        .unwrap_or_else(|| AgentIdentity {
            agent_id: Uuid::nil(),
            name: "unknown".to_string(),
            key_id: Uuid::nil(),
            admin: false,
        })
}

fn agent_of(ctx: &RequestContext<RoleServer>) -> AgentIdentity {
    agent_of_ext(&ctx.extensions)
}

fn session_of(ctx: &RequestContext<RoleServer>) -> Option<String> {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|p| p.headers.get("mcp-session-id"))
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

fn trace_of(ctx: &RequestContext<RoleServer>) -> RequestTrace {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|p| p.extensions.get::<RequestTrace>().cloned())
        .unwrap_or_default()
}

fn ok_text(value: serde_json::Value) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(&value)
        .unwrap_or_else(|_| value.to_string());
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn map_err(e: crate::error::HubError) -> McpError {
    use crate::error::HubError::*;
    match e {
        BadRequest(m) => McpError::invalid_params(m, None),
        NotFound(m) => McpError::invalid_params(format!("not_found: {m}"), None),
        GraphUnavailable(m) => McpError::internal_error(format!("graph_unavailable: {m}"), None),
        PolicyDenied(m) => McpError::internal_error(format!("policy_denied: {m}"), None),
        BudgetDenied { state, reason } => {
            McpError::internal_error(format!("budget_denied ({state}): {reason}"), None)
        }
        other => McpError::internal_error(other.to_string(), None),
    }
}

/// UUIDs cross the MCP boundary as strings; parse centrally.
fn parse_uuid(s: &str, field: &str) -> Result<Uuid, McpError> {
    Uuid::parse_str(s).map_err(|_| McpError::invalid_params(format!("invalid uuid in {field}"), None))
}

fn parse_uuid_opt(s: &Option<String>, field: &str) -> Result<Option<Uuid>, McpError> {
    match s {
        Some(v) if !v.is_empty() => Ok(Some(parse_uuid(v, field)?)),
        _ => Ok(None),
    }
}

// ---------- argument structs ----------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchWebArgs {
    /// The web search query
    pub query: String,
    /// Max results (default 10, max 25)
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CrawlUrlArgs {
    /// URL to crawl and ingest as evidence
    pub url: String,
    /// Optional investigation UUID to attach the crawl task to
    pub investigation_id: Option<String>,
    /// Embedding decision (§40): "auto" (default, decision rules) | "force" | "skip"
    pub embed: Option<String>,
    /// If true (default false), block up to 30s waiting for the embed worker
    /// to finish this document so a follow-up semantic_search returns it.
    /// Returns `embedding_status` and `embed_waited_ms` in the response.
    /// When `embed=skip`, this flag is ignored.
    pub await_embed: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FetchDocumentArgs {
    /// Canonical or original URL to fetch (from store if present, else crawled fresh)
    pub url: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DocIdArgs {
    /// Document UUID
    pub document_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct KeywordSearchArgs {
    /// Keyword query over stored evidence
    pub query: String,
    /// Max results (default 10, max 50)
    pub limit: Option<i64>,
    /// Optional: only documents whose canonical URL contains this string
    pub url_contains: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryEntityArgs {
    /// Entity name (case-insensitive contains match)
    pub name: String,
    /// Optional entity kind filter (person|org|domain|ip|location|event|...)
    pub kind: Option<String>,
    /// Max results (default 20, max 100)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryRelationshipArgs {
    /// Entity name (exact, case-insensitive)
    pub name: String,
    /// Max results (default 50, max 200)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindPathArgs {
    /// Start entity name (exact, case-insensitive)
    pub from: String,
    /// Target entity name (exact, case-insensitive)
    pub to: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ToolSchemaArgs {
    /// Tool name (e.g. "create_claim", "find_path")
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateInvestigationArgs {
    /// Investigation title
    pub title: String,
    /// The question this investigation answers
    pub question: Option<String>,
    /// The target (person/org/domain/event/...)
    pub target: Option<String>,
    /// Initial hypothesis
    pub hypothesis: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateInvestigationArgs {
    /// Investigation UUID
    pub investigation_id: String,
    pub title: Option<String>,
    pub question: Option<String>,
    pub target: Option<String>,
    pub hypothesis: Option<String>,
    /// open|paused|closed
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EvidenceLink {
    /// Document UUID supporting/contradicting the finding
    pub document_id: String,
    /// "supports" (default) or "contradicts"
    pub relation: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateFindingArgs {
    /// Investigation UUID this finding belongs to
    pub investigation_id: String,
    /// Short finding title
    pub title: String,
    /// The claim text (agent inference — NOT verified fact, directive §29)
    pub claim_text: String,
    /// Evidence links — at least one required
    pub evidence: Vec<EvidenceLink>,
    /// Optional task UUID to attribute
    pub task_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListInvestigationsArgs {
    /// Filter by status (open|paused|closed); omit for all
    pub status: Option<String>,
    /// Max results (default 20, max 100)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskIdArgs {
    /// Task UUID
    pub task_id: String,
}

// ---------- SP2B argument structs ----------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateEntityArgs {
    /// person|org|domain|ip|location|event|infrastructure|software|handle|email|phone|crypto_wallet
    pub kind: String,
    /// Entity name (normalized)
    pub name: String,
    /// Optional aliases (max 10)
    pub aliases: Option<Vec<String>>,
    /// Optional attributes (allowlist: description,url,country,confidence,severity,first_seen,last_seen,tags,source)
    pub attributes: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ClaimEntityRefArgs {
    pub kind: String,
    pub name: String,
    /// subject|object|mentioned (default mentioned)
    pub role: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateClaimArgs {
    /// Claim text (8–4000 chars)
    pub text: String,
    /// Entities the claim is about (created if missing)
    pub entities: Option<Vec<ClaimEntityRefArgs>>,
    /// §43: at least one evidence document UUID is REQUIRED
    pub evidence_document_ids: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateRelationshipArgs {
    pub from_kind: String,
    pub from_name: String,
    pub to_kind: String,
    pub to_name: String,
    /// controls|owns|communicates_with|resolves_to|located_in|affiliated_with|uses|hosts|registered_by|related_to
    pub rel_type: String,
    pub attributes: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListAlertsArgs {
    /// open|ack|muted (omit for all)
    pub status: Option<String>,
    /// osint|agent|sensor|infra|budget|security
    pub source: Option<String>,
    /// info|warning|critical
    pub severity: Option<String>,
    /// Max results (default 50, max 200)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AlertIdArgs {
    /// Alert UUID
    pub alert_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ComponentActionArgs {
    /// postgres|redis|neo4j|qdrant|searxng|crawl4ai|spiderfoot|huginn
    pub component: String,
    /// upgrade|rollback|backup
    pub action: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SignalQueryArgs {
    /// Series name or LIKE pattern, e.g. "fred:%", "quote:AAPL", "sentiment:%"
    pub series: String,
    /// ISO8601 lower bound (optional, default: no bound)
    pub from: Option<String>,
    /// ISO8601 upper bound (optional)
    pub to: Option<String>,
    /// Max points (default 100, max 1000), newest first
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FinancialsFetchArgs {
    /// US ticker, e.g. AAPL (financialdatasets.ai coverage: US equities)
    pub ticker: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WatchlistManageArgs {
    /// add|remove|toggle|list
    pub action: String,
    /// Symbol (required for add/remove/toggle), e.g. AAPL, BTCUSD
    pub symbol: Option<String>,
    /// us_stock|etf|index|crypto (add only, default us_stock)
    pub asset_class: Option<String>,
    /// Human label (add only)
    pub label: Option<String>,
}

// ---------- tool implementations ----------

#[tool_router]
impl HubMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    async fn record(
        &self,
        ctx: &RequestContext<RoleServer>,
        tool: &str,
        started: Instant,
        status: &str,
    ) {
        let agent = agent_of(ctx);
        let trace = trace_of(ctx);
        let agent_id = if agent.agent_id.is_nil() { None } else { Some(agent.agent_id) };
        if let Err(e) = crate::store::record_tool_call(
            &self.state.pg,
            agent_id,
            session_of(ctx).as_deref(),
            &trace.request_id,
            &trace.trace_id,
            tool,
            None,
            status,
            started.elapsed().as_millis() as i32,
        )
        .await
        {
            tracing::warn!(error = %e, "tool_call record failed");
        }
        // SP2B §59: every tool call is metered into the gas model.
        if let Some(aid) = agent_id {
            let _ = crate::cost::record_cost(
                &self.state,
                Some(aid),
                None,
                "tool_call",
                1.0,
                "call",
                serde_json::json!({ "tool": tool, "status": status }),
            )
            .await;
        }
    }

    /// SP2B governance gate — the single enforcement point. Every tool passes
    /// through policy (§61 levels) + budget (§60 states) before executing.
    async fn gate(
        &self,
        ctx: &RequestContext<RoleServer>,
        tool: &str,
    ) -> Result<crate::cost::BudgetState, McpError> {
        let agent = agent_of(ctx);
        crate::policy::preflight(&self.state, &agent, tool)
            .await
            .map_err(map_err)
    }

    #[tool(description = "Search the web via the SearXNG federated sensor. Returns normalized results (url/title/snippet/engine).")]
    async fn search_web(
        &self,
        Parameters(args): Parameters<SearchWebArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let bs = self.gate(&ctx, "search_web").await?;
        let mut limit = args.limit.unwrap_or(10).min(25);
        // §60 YELLOW: reduce exploration breadth.
        if bs.state >= crate::cost::State::Yellow {
            limit = (limit / 2).max(3);
        }
        let result = crate::sensors::search_web(&self.state, &args.query, limit).await;
        let out = match result {
            Ok(items) => ok_text(json!({ "query": args.query, "count": items.len(), "results": items })),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "search_web", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Crawl a URL via Crawl4AI and ingest it as evidence (normalized, deduplicated, provenance-tracked). Returns document_id and dedupe status. When `await_embed=true`, blocks up to 30s for the embed worker to finish so a follow-up semantic_search returns this document.")]
    async fn crawl_url(
        &self,
        Parameters(args): Parameters<CrawlUrlArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "crawl_url").await?;
        let agent = agent_of(&ctx);
        let investigation_id = parse_uuid_opt(&args.investigation_id, "investigation_id")?;
        let embed_mode = match args.embed.as_deref() {
            None | Some("auto") => "auto",
            Some("force") => "force",
            Some("skip") => "skip",
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!("embed must be auto|force|skip, got '{other}'"),
                    None,
                ))
            }
        };
        let await_embed = args.await_embed.unwrap_or(false);
        let result = self
            .crawl_and_ingest(&agent, &args.url, investigation_id, embed_mode, await_embed)
            .await;
        let out = match result {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "crawl_url", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    async fn crawl_and_ingest(
        &self,
        agent: &AgentIdentity,
        url: &str,
        investigation_id: Option<Uuid>,
        embed_mode: &str,
        await_embed: bool,
    ) -> crate::error::Result<serde_json::Value> {
        let parsed = url::Url::parse(url)
            .map_err(|_| crate::error::HubError::bad_request("invalid URL"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(crate::error::HubError::bad_request("only http(s) URLs are allowed"));
        }

        let task_id = crate::store::create_task(
            &self.state.pg,
            investigation_id,
            "crawl",
            &format!("agent:{}", agent.name),
            json!({ "url": url }),
        )
        .await?;
        crate::events::publish(
            &self.state,
            BusEvent::new("TASK_STARTED", &format!("agent:{}", agent.name), json!({ "task_id": task_id, "kind": "crawl", "url": url })),
        )
        .await;

        let page = crate::sensors::crawl_url(&self.state, url).await?;
        let outcome = crate::ingest::ingest_content(
            &self.state,
            "crawl4ai",
            &page.url,
            page.title.as_deref(),
            &page.markdown,
            None,
            page.metadata.clone(),
            json!({ "sensor": "crawl4ai", "agent": agent.name, "task_id": task_id }),
            Some(task_id),
            embed_mode,
        )
        .await?;

        // §59: meter the actual crawl (dedupe hits cost no sensor work).
        if !outcome.duplicate {
            let agent_id = if agent.agent_id.is_nil() { None } else { Some(agent.agent_id) };
            let _ = crate::cost::record_cost(
                &self.state,
                agent_id,
                Some(task_id),
                "crawl_page",
                1.0,
                "page",
                json!({ "url": url }),
            )
            .await;
        }

        crate::store::finish_task(
            &self.state.pg,
            task_id,
            "completed",
            json!({ "document_id": outcome.document_id, "duplicate": outcome.duplicate }),
        )
        .await?;
        crate::events::publish(
            &self.state,
            BusEvent::new("TASK_COMPLETED", &format!("agent:{}", agent.name), json!({ "task_id": task_id })),
        )
        .await;

        let mut response = json!({
            "task_id": task_id,
            "document_id": outcome.document_id,
            "content_hash": outcome.content_hash,
            "duplicate": outcome.duplicate,
            "near_duplicate_of": outcome.near_duplicate_of,
            "title": page.title,
            "links_count": page.links_count,
        });

        // `await_embed` (e2e audit fix 2026-09-13): close the gap between
        // ingest and semantic_search recall. Polls up to 30s for the embed
        // worker to process this specific doc. No-op when embed_mode=skip
        // or when the doc was a duplicate (no job queued).
        if await_embed && embed_mode != "skip" {
            let wait = crate::mcp::wait_for_embed(&self.state.pg, outcome.document_id, 30).await;
            response["embedding_status"] = json!(wait.status);
            response["embed_waited_ms"] = json!(wait.waited_ms);
            if let Some(reason) = wait.reason {
                response["embed_skip_reason"] = json!(reason);
            }
        }

        Ok(response)
    }

    #[tool(description = "Fetch a document by URL: returns the stored evidence if already collected, otherwise crawls and ingests it first.")]
    async fn fetch_document(
        &self,
        Parameters(args): Parameters<FetchDocumentArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "fetch_document").await?;
        let canonical = crate::ingest::canonicalize_url(&args.url);
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT document_id FROM documents WHERE url_canonical = $1 ORDER BY retrieved_at DESC LIMIT 1",
        )
        .bind(&canonical)
        .fetch_optional(&self.state.pg)
        .await
        .map_err(|e| map_err(e.into()))?;
        let out = if let Some(doc_id) = existing {
            let doc = crate::store::get_document(&self.state.pg, doc_id).await.map_err(map_err)?;
            ok_text(json!({ "source": "store", "document": doc }))
        } else {
            let agent = agent_of(&ctx);
            match self.crawl_and_ingest(&agent, &args.url, None, "auto", false).await {
                Ok(v) => {
                    let doc = crate::store::get_document(
                        &self.state.pg,
                        Uuid::parse_str(v["document_id"].as_str().unwrap_or_default())
                            .unwrap_or_default(),
                    )
                    .await
                    .map_err(map_err)?;
                    ok_text(json!({ "source": "crawled", "crawl": v, "document": doc }))
                }
                Err(e) => Err(map_err(e)),
            }
        };
        self.record(&ctx, "fetch_document", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Get a stored evidence document including provenance chain (source, retrieval time, content hash) and the findings that reference it.")]
    async fn get_evidence(
        &self,
        Parameters(args): Parameters<DocIdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "get_evidence").await?;
        let doc_id = parse_uuid(&args.document_id, "document_id")?;
        let out = match self.get_evidence_inner(doc_id).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "get_evidence", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    async fn get_evidence_inner(&self, document_id: Uuid) -> crate::error::Result<serde_json::Value> {
        let doc = crate::store::get_document(&self.state.pg, document_id).await?;
        let findings: Vec<serde_json::Value> = sqlx::query(
            "SELECT f.finding_id, f.title, f.created_by, fe.relation FROM findings f \
             JOIN finding_evidence fe ON fe.finding_id = f.finding_id WHERE fe.document_id = $1",
        )
        .bind(document_id)
        .fetch_all(&self.state.pg)
        .await?
        .iter()
        .map(|r| {
            use sqlx::Row;
            json!({
                "finding_id": r.get::<Uuid, _>(0),
                "title": r.get::<String, _>(1),
                "created_by": r.get::<String, _>(2),
                "relation": r.get::<String, _>(3),
            })
        })
        .collect();
        Ok(json!({
            "document": doc,
            "referenced_by_findings": findings,
            "provenance_chain": format!("Document → Source({}) → Retrieved({})", doc["provenance"], doc["retrieved_at"]),
        }))
    }

    #[tool(description = "Get a stored document by id (content + metadata + provenance).")]
    async fn get_document(
        &self,
        Parameters(args): Parameters<DocIdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "get_document").await?;
        let doc_id = parse_uuid(&args.document_id, "document_id")?;
        let out = match crate::store::get_document(&self.state.pg, doc_id).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "get_document", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Keyword full-text search over stored evidence documents.")]
    async fn keyword_search(
        &self,
        Parameters(args): Parameters<KeywordSearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "keyword_search").await?;
        let limit = args.limit.unwrap_or(10).clamp(1, 50);
        let out = match crate::store::keyword_search(&self.state.pg, &args.query, limit).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "keyword_search", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Hybrid retrieval over evidence: keyword + vector channels fused with reciprocal-rank fusion (§41 candidate set → rerank → evidence set).")]
    async fn hybrid_search(
        &self,
        Parameters(args): Parameters<KeywordSearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "hybrid_search").await?;
        let agent = agent_of(&ctx);
        let limit = args.limit.unwrap_or(10).clamp(1, 50);
        let out = match self
            .hybrid_inner(&agent, &args.query, limit, args.url_contains.as_deref())
            .await
        {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "hybrid_search", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    /// RRF (k=10) fusion of the keyword channel and the vector channel.
    /// A vector-channel outage degrades to keyword-only — never an error.
    /// Scores are reported both raw (RRF reciprocal-rank) and min-max
    /// normalized to [0,1] so downstream code can threshold without
    /// knowing the K constant.
    async fn hybrid_inner(
        &self,
        agent: &AgentIdentity,
        query: &str,
        limit: i64,
        url_contains: Option<&str>,
    ) -> crate::error::Result<serde_json::Value> {
        const K: f64 = 10.0;
        let kw = crate::store::keyword_search(&self.state.pg, query, (limit * 3).clamp(10, 150)).await?;
        let mut kw_ranks: Vec<uuid::Uuid> = Vec::new();
        if let Some(items) = kw.get("items").and_then(|i| i.as_array()) {
            for it in items {
                if let Some(id) = it
                    .get("document_id")
                    .and_then(|d| d.as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                {
                    if !kw_ranks.contains(&id) {
                        kw_ranks.push(id);
                    }
                }
            }
        }
        let mut scored: Vec<RrfScore> = rrf_fuse(&kw_ranks, &[], K);
        let mut vec_ok = false;
        match crate::embed::query_embedding(&self.state, query).await {
            Ok((qvec, tokens)) => {
                if tokens > 0 {
                    let agent_id = if agent.agent_id.is_nil() { None } else { Some(agent.agent_id) };
                    let _ = crate::cost::record_cost(
                        &self.state, agent_id, None, "embedding_tokens", tokens as f64, "token",
                        json!({ "purpose": "hybrid_query" }),
                    ).await;
                }
                match crate::vector::search(&self.state, &qvec, (limit * 3) as u64).await {
                    Ok(hits) if !hits.is_empty() => {
                        let vec_ranks: Vec<uuid::Uuid> = hits.iter().map(|(id, _)| *id).collect();
                        scored = rrf_fuse(&kw_ranks, &vec_ranks, K);
                        vec_ok = true;
                    }
                    _ => {}
                }
            }
            Err(_) => {}
        }
        let mut items = Vec::new();
        for s in scored {
            if items.len() >= limit as usize {
                break;
            }
            let row: Option<(String, Option<String>, String)> = sqlx::query_as(
                "SELECT url_canonical, title, left(content_text, 400) FROM documents WHERE document_id = $1",
            )
            .bind(s.doc_id)
            .fetch_optional(&self.state.pg)
            .await?;
            if let Some((url, title, excerpt)) = row {
                if let Some(f) = url_contains {
                    if !url.contains(f) {
                        continue;
                    }
                }
                items.push(json!({
                    "document_id": s.doc_id,
                    "rrf_score": (s.raw * 1e6).round() / 1e6,
                    "rrf_norm": (s.norm * 1e6).round() / 1e6,
                    "url": url, "title": title, "excerpt": excerpt,
                }));
            }
        }
        Ok(json!({
            "query": query,
            "mode": if vec_ok { "hybrid-rrf" } else { "keyword (vector channel unavailable or empty)" },
            "count": items.len(),
            "items": items,
        }))
    }

    #[tool(description = "Semantic similarity search over embedded evidence (vector channel, §39–42). Falls back to labeled keyword results only when nothing is embedded yet.")]
    async fn semantic_search(
        &self,
        Parameters(args): Parameters<KeywordSearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "semantic_search").await?;
        let agent = agent_of(&ctx);
        let limit = args.limit.unwrap_or(10).clamp(1, 50);
        let out = match self.semantic_inner(&agent, &args.query, limit).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "semantic_search", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    async fn semantic_inner(
        &self,
        agent: &AgentIdentity,
        query: &str,
        limit: i64,
    ) -> crate::error::Result<serde_json::Value> {
        let (qvec, tokens) = crate::embed::query_embedding(&self.state, query).await?;
        if tokens > 0 {
            let agent_id = if agent.agent_id.is_nil() { None } else { Some(agent.agent_id) };
            let _ = crate::cost::record_cost(
                &self.state, agent_id, None, "embedding_tokens", tokens as f64, "token",
                json!({ "purpose": "query", "query": query }),
            ).await;
        }
        let hits = crate::vector::search(&self.state, &qvec, (limit * 3) as u64).await?;
        if hits.is_empty() {
            // Honest fallback: nothing embedded yet — label it, never pretend.
            let mut v = crate::store::keyword_search(&self.state.pg, query, limit).await?;
            v["mode"] = json!("keyword-fallback");
            v["note"] = json!("no embedded documents yet; these are keyword results, not vector similarity");
            return Ok(v);
        }
        let mut items = Vec::new();
        let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (doc_id, score) in hits.iter() {
            if items.len() >= limit as usize {
                break;
            }
            let row: Option<(String, Option<String>, String)> = sqlx::query_as(
                "SELECT url_canonical, title, left(content_text, 400) FROM documents WHERE document_id = $1",
            )
            .bind(doc_id)
            .fetch_optional(&self.state.pg)
            .await?;
            if let Some((url, title, excerpt)) = row {
                // Dedupe page snapshots: repeated crawls of a changing page
                // store one document per content hash; without this a single
                // page can occupy every result slot (hits are best-score
                // ordered, so the first occurrence is the best match).
                if !seen_urls.insert(url.clone()) {
                    continue;
                }
                items.push(json!({
                    "document_id": doc_id, "score": score, "url": url,
                    "title": title, "excerpt": excerpt,
                }));
            }
        }
        Ok(json!({ "query": query, "mode": "vector", "count": items.len(), "items": items }))
    }

    #[tool(description = "Query entities in the relationship graph by name (read-only, parameterized — no arbitrary Cypher).")]
    async fn query_entity(
        &self,
        Parameters(args): Parameters<QueryEntityArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "query_entity").await?;
        let limit = args.limit.unwrap_or(20);
        let out = match crate::graph::query_entity(&self.state, &args.name, args.kind.as_deref(), limit).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "query_entity", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "List relationships of an entity in the graph (read-only, parameterized).")]
    async fn query_relationship(
        &self,
        Parameters(args): Parameters<QueryRelationshipArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "query_relationship").await?;
        let limit = args.limit.unwrap_or(50);
        let out = match crate::graph::query_relationship(&self.state, &args.name, limit).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "query_relationship", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Find the shortest path (max 5 hops) between two entities in the graph (read-only, parameterized).")]
    async fn find_path(
        &self,
        Parameters(args): Parameters<FindPathArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "find_path").await?;
        let out = match crate::graph::find_path(&self.state, &args.from, &args.to).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "find_path", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    /// MCP discoverability (e2e audit fix 2026-09-13): many tool arg field
    /// names aren't surfaced prominently by every MCP client. These two
    /// zero-cost tools let callers introspect the surface at runtime
    /// instead of guessing parameter names.
    #[tool(description = "List all available MCP tools (name + one-line description). Use this first when an error says 'missing field X' to discover the canonical argument name. Read-only, no budget cost, not audited.")]
    async fn list_tools(&self) -> Result<CallToolResult, McpError> {
        let tools = self.tool_router.list_all();
        let items: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description.as_deref().unwrap_or(""),
                })
            })
            .collect();
        ok_text(json!({ "count": items.len(), "tools": items }))
    }

    #[tool(description = "Return the full input schema (param names/types/required/description) for one MCP tool by name. Read-only, no budget cost, not audited.")]
    async fn tool_schema(
        &self,
        Parameters(args): Parameters<ToolSchemaArgs>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        match self.tool_router.get(&args.name) {
            Some(t) => ok_text(json!({
                "name": t.name,
                "description": t.description.as_deref().unwrap_or(""),
                "input_schema": t.input_schema.as_ref(),
            })),
            None => {
                let known: Vec<String> = self.tool_router.list_all().iter().map(|t| t.name.to_string()).collect();
                Err(McpError::invalid_params(
                    format!("unknown tool '{}'; known: {}", args.name, known.join(", ")),
                    None,
                ))
            }
        }
    }

    #[tool(description = "Create a new investigation (first-class object: target/question/hypothesis).")]
    async fn create_investigation(
        &self,
        Parameters(args): Parameters<CreateInvestigationArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "create_investigation").await?;
        let agent = agent_of(&ctx);
        let res = crate::store::create_investigation(
            &self.state.pg,
            &args.title,
            args.question.as_deref(),
            args.target.as_deref(),
            args.hypothesis.as_deref(),
            &format!("agent:{}", agent.name),
        )
        .await;
        let out = match res {
            Ok(id) => {
                crate::events::publish(
                    &self.state,
                    BusEvent::new("TASK_CREATED", &format!("agent:{}", agent.name), json!({ "investigation_id": id, "title": args.title })).with_investigation(id),
                )
                .await;
                ok_text(json!({ "investigation_id": id, "title": args.title }))
            }
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "create_investigation", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Update an investigation's fields (title/question/target/hypothesis/status).")]
    async fn update_investigation(
        &self,
        Parameters(args): Parameters<UpdateInvestigationArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "update_investigation").await?;
        if let Some(s) = &args.status {
            if !["open", "paused", "closed"].contains(&s.as_str()) {
                return Err(McpError::invalid_params("status must be open|paused|closed", None));
            }
        }
        let id = parse_uuid(&args.investigation_id, "investigation_id")?;
        let res = crate::store::update_investigation(
            &self.state.pg,
            id,
            args.title.as_deref(),
            args.question.as_deref(),
            args.target.as_deref(),
            args.hypothesis.as_deref(),
            args.status.as_deref(),
        )
        .await;
        let out = match res {
            Ok(true) => ok_text(json!({ "investigation_id": id, "updated": true })),
            Ok(false) => Err(McpError::invalid_params("investigation not found", None)),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "update_investigation", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Create an agent finding bound to evidence (directive §28/§29: claims must cite supporting/contradicting evidence; findings are agent inference, never presented as verified fact).")]
    async fn create_finding(
        &self,
        Parameters(args): Parameters<CreateFindingArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "create_finding").await?;
        let agent = agent_of(&ctx);
        let investigation_id = parse_uuid(&args.investigation_id, "investigation_id")?;
        let task_id = parse_uuid_opt(&args.task_id, "task_id")?;
        let mut evidence: Vec<(Uuid, String)> = Vec::with_capacity(args.evidence.len());
        for e in &args.evidence {
            evidence.push((
                parse_uuid(&e.document_id, "evidence.document_id")?,
                e.relation.clone().unwrap_or_else(|| "supports".to_string()),
            ));
        }
        let agent_id = if agent.agent_id.is_nil() { None } else { Some(agent.agent_id) };
        let res = crate::store::create_finding(
            &self.state.pg,
            investigation_id,
            &args.title,
            &args.claim_text,
            &evidence,
            &format!("agent:{}", agent.name),
            agent_id,
            task_id,
        )
        .await;
        let out = match res {
            Ok(id) => {
                crate::events::publish(
                    &self.state,
                    BusEvent::new("FINDING_CREATED", &format!("agent:{}", agent.name), json!({ "finding_id": id, "title": args.title, "evidence_count": evidence.len() })).with_investigation(investigation_id),
                )
                .await;
                ok_text(json!({ "finding_id": id, "evidence_links": evidence.len() }))
            }
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "create_finding", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "List investigations, optionally filtered by status.")]
    async fn list_investigations(
        &self,
        Parameters(args): Parameters<ListInvestigationsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "list_investigations").await?;
        let limit = args.limit.unwrap_or(20).clamp(1, 100);
        let out = match crate::store::list_investigations(&self.state.pg, args.status.as_deref(), limit).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "list_investigations", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Get the status of a task (search/crawl/ingest/analysis).")]
    async fn get_task_status(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "get_task_status").await?;
        let task_id = parse_uuid(&args.task_id, "task_id")?;
        let out = match crate::store::get_task(&self.state.pg, task_id).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "get_task_status", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Aggregate system health: Postgres, Redis, Neo4j, Qdrant, SearXNG, Crawl4AI latencies + docker container states.")]
    async fn get_system_health(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "get_system_health").await?;
        let out = ok_text(crate::api::system_health(&self.state).await);
        self.record(&ctx, "get_system_health", started, "ok").await;
        out
    }

    // ---------- SP2B: governance & write plane ----------

    #[tool(description = "Create an entity in the intelligence graph (typed intent → schema validation → PG canonical + Neo4j relationship memory). Level 2 governed.")]
    async fn create_entity(
        &self,
        Parameters(args): Parameters<CreateEntityArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "create_entity").await?;
        let agent = agent_of(&ctx);
        let actor = format!("agent:{}", agent.name);
        let out = match crate::graphw::create_entity(
            &self.state,
            &actor,
            crate::graphw::EntityIntent {
                kind: args.kind,
                name: args.name,
                aliases: args.aliases,
                attributes: args.attributes,
            },
        )
        .await
        {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "create_entity", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Create a claim bound to evidence (§43: at least one evidence document REQUIRED). Optionally links entities (created if missing). Level 2 governed.")]
    async fn create_claim(
        &self,
        Parameters(args): Parameters<CreateClaimArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "create_claim").await?;
        let agent = agent_of(&ctx);
        let actor = format!("agent:{}", agent.name);
        let out = match crate::graphw::create_claim(
            &self.state,
            &actor,
            crate::graphw::ClaimIntent {
                text: args.text,
                entities: args.entities.map(|v| {
                    v.into_iter()
                        .map(|e| crate::graphw::ClaimEntityRef { kind: e.kind, name: e.name, role: e.role })
                        .collect()
                }),
                evidence_document_ids: args.evidence_document_ids,
            },
        )
        .await
        {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "create_claim", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Create a typed relationship between two existing entities (endpoints must exist — create them first). Level 2 governed.")]
    async fn create_relationship(
        &self,
        Parameters(args): Parameters<CreateRelationshipArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "create_relationship").await?;
        let agent = agent_of(&ctx);
        let actor = format!("agent:{}", agent.name);
        let out = match crate::graphw::create_relationship(
            &self.state,
            &actor,
            crate::graphw::RelationshipIntent {
                from_kind: args.from_kind,
                from_name: args.from_name,
                to_kind: args.to_kind,
                to_name: args.to_name,
                rel_type: args.rel_type,
                attributes: args.attributes,
            },
        )
        .await
        {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "create_relationship", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "List alerts from the unified alert center (§54): severity, source, links, recommended action.")]
    async fn list_alerts(
        &self,
        Parameters(args): Parameters<ListAlertsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "list_alerts").await?;
        let out = match crate::alerts::list(
            &self.state,
            args.status.as_deref(),
            args.source.as_deref(),
            args.severity.as_deref(),
            args.limit.unwrap_or(50),
        )
        .await
        {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "list_alerts", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Acknowledge an open alert.")]
    async fn acknowledge_alert(
        &self,
        Parameters(args): Parameters<AlertIdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "acknowledge_alert").await?;
        let agent = agent_of(&ctx);
        let id = parse_uuid(&args.alert_id, "alert_id")?;
        let out = match crate::alerts::set_status(&self.state, id, "ack", &format!("agent:{}", agent.name)).await {
            Ok(true) => ok_text(json!({ "alert_id": id, "status": "ack" })),
            Ok(false) => Err(McpError::invalid_params("alert not found or not open", None)),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "acknowledge_alert", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Mute an open alert (stops dedupe-bumping and webhook noise for this alert).")]
    async fn mute_alert(
        &self,
        Parameters(args): Parameters<AlertIdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "mute_alert").await?;
        let agent = agent_of(&ctx);
        let id = parse_uuid(&args.alert_id, "alert_id")?;
        let out = match crate::alerts::set_status(&self.state, id, "muted", &format!("agent:{}", agent.name)).await {
            Ok(true) => ok_text(json!({ "alert_id": id, "status": "muted" })),
            Ok(false) => Err(McpError::invalid_params("alert not found or not open", None)),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "mute_alert", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Get YOUR current budget state (GREEN/YELLOW/RED/KILL) with today's usage vs limits (§58–60). Self-throttle accordingly.")]
    async fn get_budget_status(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "get_budget_status").await?;
        let agent = agent_of(&ctx);
        let out = match crate::cost::budget_state(&self.state, &agent).await {
            Ok(bs) => ok_text(serde_json::to_value(&bs).unwrap_or_else(|_| json!({}))),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "get_budget_status", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "LEVEL 3 (requires X-Admin-Token, default DENY per §61): run a whitelisted lifecycle action on a component — upgrade|rollback|backup via the pinned VM scripts.")]
    async fn run_component_action(
        &self,
        Parameters(args): Parameters<ComponentActionArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "run_component_action").await?;
        let agent = agent_of(&ctx);
        let out = match crate::components::run_action(
            &self.state,
            &args.component,
            &args.action,
            &format!("agent:{} (admin)", agent.name),
        )
        .await
        {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "run_component_action", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Query monitor time-series observations (macro FRED/EIA/Treasury, market quotes, sentiment). Series are self-describing with units, e.g. fred:VIXCLS, fred:CPIAUCSL_YOY, quote:AAPL, sentiment:AAPL, eia:WTI_SPOT_USD_BBL.")]
    async fn signal_query(
        &self,
        Parameters(args): Parameters<SignalQueryArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "signal_query").await?;
        let out = match signal_query_inner(&self.state, &args).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "signal_query", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Fetch deep fundamentals for a US ticker via financialdatasets.ai (8 endpoints: profile/statements/segments/13F holders/insider KPI/guidance). Level 2 governed; responses are cached 30 days in Postgres (pay-per-request provider — a cache hit costs nothing).")]
    async fn financials_fetch(
        &self,
        Parameters(args): Parameters<FinancialsFetchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "financials_fetch").await?;
        let out = match crate::monitor::fd::fetch_with_cache(&self.state, &args.ticker).await {
            Ok(o) => ok_text(serde_json::json!({
                "cached": o.cached,
                "fetched_at": o.fetched_at,
                "brief": o.brief,
            })),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "financials_fetch", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Manage the monitor market watchlist (add/remove/toggle/list). The markets collector quotes enabled symbols every 6h; finintel tracks their news/insiders/sentiment hourly. Level 2 governed.")]
    async fn watchlist_manage(
        &self,
        Parameters(args): Parameters<WatchlistManageArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        self.gate(&ctx, "watchlist_manage").await?;
        let out = match watchlist_inner(&self.state, &args).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "watchlist_manage", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }
}

#[tool_handler]
impl ServerHandler for HubMcp {
    /// Dual-axis identity (SP8): the API key is the auth/budget axis; the
    /// clientInfo handshake is the observability axis. Absorb whatever the
    /// client declares (name+version) onto the KEY-agent's row — zero config
    /// per agent, no provider enumeration, call attribution never orphans
    /// from the row that owns the budget.
    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        let agent = agent_of_ext(&context.extensions);
        if agent.agent_id.is_nil() {
            return;
        }
        let Some(info) = context.peer.peer_info() else { return };
        let client = format!("{} {}", info.client_info.name, info.client_info.version);
        let state = self.state.clone();
        let aid = agent.agent_id;
        tokio::spawn(async move {
            let _ = sqlx::query(
                "UPDATE agents SET version = $2, last_seen_at = now() WHERE agent_id = $1",
            )
            .bind(aid)
            .bind(client)
            .execute(&state.pg)
            .await;
        });
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("intelhub-core", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "IntelHub Hub Core (SP2B). Data & tool plane: search_web, crawl_url (embed:auto|force|skip), \
                 fetch_document, get_evidence, get_document, keyword_search, hybrid_search (RRF), \
                 semantic_search (vector), query_entity, query_relationship, find_path, \
                 create_investigation, update_investigation, create_finding (evidence-bound), \
                 list_investigations, get_task_status, get_system_health. \
                 Governance & write plane: create_entity, create_claim (evidence REQUIRED), \
                 create_relationship (typed intents → PG canonical + Neo4j), \
                 list_alerts, acknowledge_alert, mute_alert, get_budget_status (self-throttle), \
                 run_component_action (LEVEL 3: needs X-Admin-Token). \
                 Monitor finance plane: signal_query (macro/quote/sentiment series), \
                 financials_fetch (deep fundamentals, 30d-cached), watchlist_manage. \
                 Every call is policy-gated (§61) and budget-metered (§58–60).".to_string(),
            )
    }
}

// ---------- SP6B finance tool helpers ----------

async fn signal_query_inner(
    state: &AppState,
    args: &SignalQueryArgs,
) -> crate::error::Result<serde_json::Value> {
    let limit = args.limit.unwrap_or(100).clamp(1, 1000);
    let pattern = if args.series.contains('%') || args.series.contains('*') {
        args.series.replace('*', "%")
    } else {
        args.series.clone()
    };
    let rows: Vec<(String, chrono::DateTime<chrono::Utc>, f64, serde_json::Value)> = sqlx::query_as(
        "SELECT series, observed_at, value, payload FROM signal_observations
         WHERE series LIKE $1
           AND ($2::text IS NULL OR observed_at >= $2::timestamptz)
           AND ($3::text IS NULL OR observed_at <= $3::timestamptz)
         ORDER BY observed_at DESC LIMIT $4",
    )
    .bind(&pattern)
    .bind(&args.from)
    .bind(&args.to)
    .bind(limit)
    .fetch_all(&state.pg)
    .await?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|(series, ts, value, payload)| {
            serde_json::json!({
                "series": series, "observed_at": ts, "value": value, "payload": payload,
            })
        })
        .collect();
    // Distinguish "series never observed" from "no points inside the window"
    // — a silent empty list for a typo'd series name wastes agent turns.
    let series_seen = if items.is_empty() {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM signal_observations WHERE series LIKE $1)",
        )
        .bind(&pattern)
        .fetch_one(&state.pg)
        .await?
    } else {
        true
    };
    Ok(serde_json::json!({
        "series_pattern": pattern,
        "series_seen": series_seen,
        "count": items.len(),
        "observations": items,
    }))
}

pub(crate) async fn watchlist_inner(
    state: &AppState,
    args: &WatchlistManageArgs,
) -> crate::error::Result<serde_json::Value> {
    match args.action.as_str() {
        "list" => {
            let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
                "SELECT symbol, asset_class, label, enabled FROM monitor_watchlist ORDER BY symbol",
            )
            .fetch_all(&state.pg)
            .await?;
            let items: Vec<serde_json::Value> = rows
                .iter()
                .map(|(s, c, l, e)| serde_json::json!({"symbol": s, "asset_class": c, "label": l, "enabled": e}))
                .collect();
            Ok(serde_json::json!({"count": items.len(), "watchlist": items}))
        }
        "add" => {
            let sym = args.symbol.as_deref().unwrap_or("").trim().to_uppercase();
            if sym.is_empty() || sym.len() > 12 || !sym.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '^') {
                return Err(crate::error::HubError::BadRequest("invalid symbol".into()));
            }
            let class = args.asset_class.as_deref().unwrap_or("us_stock");
            if !["us_stock", "etf", "index", "crypto"].contains(&class) {
                return Err(crate::error::HubError::BadRequest(
                    "asset_class must be us_stock|etf|index|crypto".into(),
                ));
            }
            sqlx::query(
                "INSERT INTO monitor_watchlist (symbol, asset_class, label) VALUES ($1,$2,$3)
                 ON CONFLICT (symbol) DO UPDATE SET asset_class=$2, label=$3, enabled=true",
            )
            .bind(&sym)
            .bind(class)
            .bind(args.label.as_deref().unwrap_or(""))
            .execute(&state.pg)
            .await?;
            Ok(serde_json::json!({"ok": true, "action": "add", "symbol": sym}))
        }
        "remove" => {
            let sym = args.symbol.as_deref().unwrap_or("").trim().to_uppercase();
            let r = sqlx::query("DELETE FROM monitor_watchlist WHERE symbol = $1")
                .bind(&sym)
                .execute(&state.pg)
                .await?;
            Ok(serde_json::json!({"ok": r.rows_affected() > 0, "action": "remove", "symbol": sym}))
        }
        "toggle" => {
            let sym = args.symbol.as_deref().unwrap_or("").trim().to_uppercase();
            let r = sqlx::query(
                "UPDATE monitor_watchlist SET enabled = NOT enabled WHERE symbol = $1 RETURNING enabled",
            )
            .bind(&sym)
            .fetch_optional(&state.pg)
            .await?;
            match r {
                Some(row) => {
                    let enabled: bool = sqlx::Row::get(&row, "enabled");
                    Ok(serde_json::json!({"ok": true, "action": "toggle", "symbol": sym, "enabled": enabled}))
                }
                None => Err(crate::error::HubError::NotFound(format!("watchlist symbol {sym}"))),
            }
        }
        other => Err(crate::error::HubError::BadRequest(format!(
            "unknown action {other} — use add|remove|toggle|list"
        ))),
    }
}

// ---------- Embed wait helper (extracted from crawl_and_ingest for testability) ----------

#[derive(Debug, Clone)]
pub(crate) struct EmbedWaitOutcome {
    pub status: String,        // "DONE" | "SKIPPED" | "FAILED" | "PENDING" (timeout) | "no_job"
    pub waited_ms: u64,
    pub reason: Option<String>, // set when status=SKIPPED/FAILED
}

/// Poll `documents.embedding_status` every 1s up to `timeout_secs`, so a
/// caller that just crawled a URL can issue semantic_search immediately.
/// Returns `no_job` if no `embedding_jobs` row was ever queued for this
/// document (typically a duplicate or `embed_mode=skip`).
pub(crate) async fn wait_for_embed(pg: &sqlx::PgPool, document_id: Uuid, timeout_secs: u64) -> EmbedWaitOutcome {
    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(timeout_secs);
    loop {
        // Check the job queue first — if nothing was ever queued, return early
        // instead of spinning for `timeout_secs` on a duplicate.
        let job_row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT status, reason FROM embedding_jobs WHERE document_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(document_id)
        .fetch_optional(pg)
        .await
        .ok()
        .flatten();
        let Some((job_status, job_reason)) = job_row else {
            return EmbedWaitOutcome { status: "no_job".into(), waited_ms: start.elapsed().as_millis() as u64, reason: None };
        };
        // Done / failed / skipped on first attempt → return immediately.
        if matches!(job_status.as_str(), "DONE" | "FAILED" | "SKIPPED") {
            return EmbedWaitOutcome {
                status: job_status,
                waited_ms: start.elapsed().as_millis() as u64,
                reason: job_reason,
            };
        }
        if start.elapsed() >= deadline {
            return EmbedWaitOutcome {
                status: "PENDING".into(),
                waited_ms: start.elapsed().as_millis() as u64,
                reason: Some(format!("timed out after {}s; job still {}", timeout_secs, job_status)),
            };
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

// ---------- RRF helper (extracted from hybrid_inner for testability) ----------

/// Reciprocal-Rank Fusion score for one document — emitted both raw (so the
/// constant `K` is recoverable) and min-max normalized to [0,1] (so callers
/// can threshold without knowing `K`).
#[derive(Debug, Clone, Copy)]
pub struct RrfScore {
    pub doc_id: uuid::Uuid,
    pub raw: f64,
    pub norm: f64,
}

/// Reciprocal-Rank Fusion (RRF): score(d) = Σ_c 1/(K + rank_c(d)).
/// Returns candidates sorted by raw score descending. Empty input channels
/// are tolerated (keyword-only / vector-only paths still work).
pub fn rrf_fuse(keyword_ranks: &[uuid::Uuid], vector_ranks: &[uuid::Uuid], k: f64) -> Vec<RrfScore> {
    let mut scores: std::collections::HashMap<uuid::Uuid, f64> = std::collections::HashMap::new();
    for (rank, id) in keyword_ranks.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (k + rank as f64 + 1.0);
    }
    for (rank, id) in vector_ranks.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (k + rank as f64 + 1.0);
    }
    let mut ranked: Vec<(uuid::Uuid, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let len = ranked.len();
    let (min, max) = ranked
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, s)| (lo.min(*s), hi.max(*s)));
    let span = (max - min).max(f64::EPSILON);
    ranked
        .into_iter()
        .map(|(doc_id, raw)| {
            let norm = if len == 1 { 1.0 } else { (raw - min) / span };
            RrfScore { doc_id, raw, norm }
        })
        .collect()
}

// Tests moved to /tests/rrf_integration.rs (separate binary — sidesteps
// pre-existing unit-test compile failures in epa.rs/fred.rs from main).
