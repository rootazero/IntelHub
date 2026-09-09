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
    service::RequestContext,
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

fn agent_of(ctx: &RequestContext<RoleServer>) -> AgentIdentity {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|p| p.extensions.get::<AgentIdentity>().cloned())
        .unwrap_or_else(|| AgentIdentity {
            agent_id: Uuid::nil(),
            name: "unknown".to_string(),
            key_id: Uuid::nil(),
        })
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
        other => McpError::internal_error(other.to_string(), None),
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
    /// Optional investigation to attach the crawl task to
    pub investigation_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FetchDocumentArgs {
    /// Canonical or original URL to fetch (from store if present, else crawled fresh)
    pub url: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DocIdArgs {
    /// Document UUID
    pub document_id: Uuid,
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
    pub investigation_id: Uuid,
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
    pub document_id: Uuid,
    /// "supports" (default) or "contradicts"
    pub relation: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateFindingArgs {
    /// Investigation UUID this finding belongs to
    pub investigation_id: Uuid,
    /// Short finding title
    pub title: String,
    /// The claim text (agent inference — NOT verified fact, directive §29)
    pub claim_text: String,
    /// Evidence links — at least one required
    pub evidence: Vec<EvidenceLink>,
    /// Optional task UUID to attribute
    pub task_id: Option<Uuid>,
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
    pub task_id: Uuid,
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
    }

    #[tool(description = "Search the web via the SearXNG federated sensor. Returns normalized results (url/title/snippet/engine).")]
    async fn search_web(
        &self,
        Parameters(args): Parameters<SearchWebArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let limit = args.limit.unwrap_or(10).min(25);
        let result = crate::sensors::search_web(&self.state, &args.query, limit).await;
        let out = match result {
            Ok(items) => ok_text(json!({ "query": args.query, "count": items.len(), "results": items })),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "search_web", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Crawl a URL via Crawl4AI and ingest it as evidence (normalized, deduplicated, provenance-tracked). Returns document_id and dedupe status.")]
    async fn crawl_url(
        &self,
        Parameters(args): Parameters<CrawlUrlArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let agent = agent_of(&ctx);
        let result = self.crawl_and_ingest(&agent, &args.url, args.investigation_id).await;
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
        )
        .await?;

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

        Ok(json!({
            "task_id": task_id,
            "document_id": outcome.document_id,
            "content_hash": outcome.content_hash,
            "duplicate": outcome.duplicate,
            "near_duplicate_of": outcome.near_duplicate_of,
            "title": page.title,
            "links_count": page.links_count,
        }))
    }

    #[tool(description = "Fetch a document by URL: returns the stored evidence if already collected, otherwise crawls and ingests it first.")]
    async fn fetch_document(
        &self,
        Parameters(args): Parameters<FetchDocumentArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let canonical = crate::ingest::canonicalize_url(&args.url);
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT document_id FROM documents WHERE url_canonical = $1 ORDER BY retrieved_at DESC LIMIT 1",
        )
        .bind(&canonical)
        .fetch_optional(&self.state.pg)
        .await
        .map_err(map_err)?;
        let out = if let Some(doc_id) = existing {
            let doc = crate::store::get_document(&self.state.pg, doc_id).await.map_err(map_err)?;
            ok_text(json!({ "source": "store", "document": doc }))
        } else {
            let agent = agent_of(&ctx);
            match self.crawl_and_ingest(&agent, &args.url, None).await {
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
        let out = match self.get_evidence_inner(args.document_id).await {
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
        let out = match crate::store::get_document(&self.state.pg, args.document_id).await {
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
        let limit = args.limit.unwrap_or(10).clamp(1, 50);
        let out = match crate::store::keyword_search(&self.state.pg, &args.query, limit).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "keyword_search", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Hybrid retrieval over evidence: keyword + metadata filters (SP2A). The vector/semantic channel is schema-reserved and activates in SP2B.")]
    async fn hybrid_search(
        &self,
        Parameters(args): Parameters<KeywordSearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let limit = args.limit.unwrap_or(10).clamp(1, 50);
        let base = crate::store::keyword_search(&self.state.pg, &args.query, limit).await;
        let out = match base {
            Ok(mut v) => {
                if let Some(filter) = &args.url_contains {
                    if let Some(items) = v.get_mut("items").and_then(|i| i.as_array_mut()) {
                        items.retain(|it| {
                            it.get("url").and_then(|u| u.as_str()).map(|u| u.contains(filter.as_str())).unwrap_or(false)
                        });
                    }
                }
                v["mode"] = json!("keyword+metadata");
                v["vector_channel"] = json!("reserved (SP2B)");
                ok_text(v)
            }
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "hybrid_search", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Semantic similarity search over evidence. SP2A: returns an explicit keyword fallback (the embedding pipeline and vector channel activate in SP2B — results are NOT semantic yet).")]
    async fn semantic_search(
        &self,
        Parameters(args): Parameters<KeywordSearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let limit = args.limit.unwrap_or(10).clamp(1, 50);
        let base = crate::store::keyword_search(&self.state.pg, &args.query, limit).await;
        let out = match base {
            Ok(mut v) => {
                v["mode"] = json!("keyword-fallback");
                v["note"] = json!("semantic channel activates in SP2B; these are keyword results, not vector similarity");
                ok_text(v)
            }
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "semantic_search", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Query entities in the relationship graph by name (read-only, parameterized — no arbitrary Cypher).")]
    async fn query_entity(
        &self,
        Parameters(args): Parameters<QueryEntityArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
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
        let out = match crate::graph::find_path(&self.state, &args.from, &args.to).await {
            Ok(v) => ok_text(v),
            Err(e) => Err(map_err(e)),
        };
        self.record(&ctx, "find_path", started, if out.is_ok() { "ok" } else { "error" }).await;
        out
    }

    #[tool(description = "Create a new investigation (first-class object: target/question/hypothesis).")]
    async fn create_investigation(
        &self,
        Parameters(args): Parameters<CreateInvestigationArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
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
        if let Some(s) = &args.status {
            if !["open", "paused", "closed"].contains(&s.as_str()) {
                return Err(McpError::invalid_params("status must be open|paused|closed", None));
            }
        }
        let res = crate::store::update_investigation(
            &self.state.pg,
            args.investigation_id,
            args.title.as_deref(),
            args.question.as_deref(),
            args.target.as_deref(),
            args.hypothesis.as_deref(),
            args.status.as_deref(),
        )
        .await;
        let out = match res {
            Ok(true) => ok_text(json!({ "investigation_id": args.investigation_id, "updated": true })),
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
        let agent = agent_of(&ctx);
        let evidence: Vec<(Uuid, String)> = args
            .evidence
            .iter()
            .map(|e| (e.document_id, e.relation.clone().unwrap_or_else(|| "supports".to_string())))
            .collect();
        let agent_id = if agent.agent_id.is_nil() { None } else { Some(agent.agent_id) };
        let res = crate::store::create_finding(
            &self.state.pg,
            args.investigation_id,
            &args.title,
            &args.claim_text,
            &evidence,
            &format!("agent:{}", agent.name),
            agent_id,
            args.task_id,
        )
        .await;
        let out = match res {
            Ok(id) => {
                crate::events::publish(
                    &self.state,
                    BusEvent::new("FINDING_CREATED", &format!("agent:{}", agent.name), json!({ "finding_id": id, "title": args.title, "evidence_count": evidence.len() })).with_investigation(args.investigation_id),
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
        let out = match crate::store::get_task(&self.state.pg, args.task_id).await {
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
        let out = ok_text(crate::api::system_health(&self.state).await);
        self.record(&ctx, "get_system_health", started, "ok").await;
        out
    }
}

#[tool_handler]
impl ServerHandler for HubMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("intelhub-core", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "IntelHub data & tool plane (SP2A). Agent-neutral capability tools: \
                 search_web, crawl_url, fetch_document, get_evidence, get_document, \
                 keyword_search, hybrid_search, semantic_search (SP2A keyword fallback), \
                 query_entity, query_relationship, find_path (graph read-only, parameterized), \
                 create_investigation, update_investigation, create_finding (evidence-bound), \
                 list_investigations, get_task_status, get_system_health.".to_string(),
            )
    }
}
