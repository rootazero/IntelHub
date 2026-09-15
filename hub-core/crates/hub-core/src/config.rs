//! Configuration via environment variables (12-factor; secrets come from
//! systemd EnvironmentFile). Non-secret defaults are sensible for the
//! IntelHub deployment documented in the SP2A spec.

#[derive(Clone, Debug)]
pub struct Config {
    pub listen_addr: String,
    pub database_url: String,
    pub redis_url: String,
    pub neo4j_uri: String,
    pub neo4j_user: String,
    pub neo4j_password: String,
    pub qdrant_url: String,
    pub qdrant_collection: String,
    pub searxng_url: String,
    pub crawl4ai_url: String,
    pub crawl4ai_api_token: String,
    pub embedding_base_url: String,
    pub embedding_model: String,
    pub embedding_api_key: Option<String>,
    pub embedding_dims: u32,
    pub raw_dir: String,
    pub rate_limit_rpm: u32,
    pub simhash_max_hamming: u32,
    pub mcp_allowed_hosts: Vec<String>,
    // ── SP2B governance ─────────────────────────────────────────────
    /// Level-3 admin token (secrets.env). None = Level 3 permanently denied.
    pub admin_token: Option<String>,
    pub budget_agent_tool_calls: f64,
    pub budget_agent_crawl_pages: f64,
    pub budget_agent_embed_tokens: f64,
    pub budget_global_crawl_pages: f64,
    pub budget_global_embed_tokens: f64,
    pub alert_webhook_url: Option<String>,
    pub alert_webhook_min_severity: String,
    pub manifests_dir: String,
    pub scripts_dir: String,
    /// §40: documents shorter than this are not auto-embedded.
    pub embed_min_words: u32,
    pub embed_enabled: bool,
    /// Cross-encoder rerank stage (e2e audit 2026-09-13). Runs after RRF in
    /// hybrid_inner and after vector search in semantic_inner. Uses the
    /// same T8star relay as embed (reuses EMBEDDING_API_KEY +
    /// EMBEDDING_BASE_URL). Off by default — flip HUB_RERANK_ENABLED=true
    /// to enable. Failures degrade gracefully (return pre-rerank order).
    pub rerank_enabled: bool,
    /// Default `BAAI/bge-reranker-v2-m3` (multilingual en+zh, fits OSINT).
    pub rerank_model: String,
    /// Top-K retrieval candidates fed into the rerank model per call.
    /// Higher = better ordering, slower. 50 is the sweet spot for OSINT.
    pub rerank_top_k: usize,
    /// HTTP timeout per `/v1/rerank` call. Anything > this is treated as
    /// a transient failure and the pre-rerank order is returned.
    pub rerank_timeout_secs: u64,
    /// A: Redis-backed cache over hybrid_search / semantic_search /
    /// keyword_search. Identical (query, limit, url_contains) within
    /// TTL returns the cached response blob, skipping PG + Qdrant +
    /// T8star entirely. Set HUB_QUERY_CACHE_ENABLED=false to disable
    /// (e.g. while debugging a freshness issue).
    pub query_cache_enabled: bool,
    /// TTL in seconds. Short by design (300s) — this is a perf layer,
    /// not a replacement for fresh retrieval. The cache key changes
    /// when corpus embeds complete so longer TTL isn't needed.
    pub query_cache_ttl_secs: u64,
    /// B: LLM-driven planner for investigate() (2026-09-13). Falls back
    /// to rule-based when the rule plan is "trivial" (single SearchHybrid).
    /// Off by default — enable after T8star /v1/chat/completions is
    /// reachable and you've verified token cost on a few real questions.
    pub llm_enabled: bool,
    /// Chat model. Cheap, fast: `gpt-4.1-mini` is the sweet spot for
    /// structured JSON plan output. Anything stronger burns tokens for
    /// no quality gain at this task complexity.
    pub llm_model: String,
    /// Hard timeout on the planner HTTP call. Must be short — a slow
    /// LLM blocks the agent. 5s is the budget.
    pub llm_timeout_ms: u64,
    /// Output cap for the plan response. The model is asked for JSON
    /// only, so 800 tokens is plenty (4 steps × ~150 chars each).
    pub llm_max_tokens: u32,
    /// One-shot entity seeder on hub-core boot. Gates the populate-the-
    /// graph step so clean-test deployments don't accumulate seed rows.
    pub seed_enabled: bool,
    /// SP3: directory holding the built console SPA (index.html + assets).
    pub console_dir: String,
    // ── SP4 ────────────────────────────────────────────────────────
    /// Prometheus base URL for §52 metrics aggregation.
    pub prometheus_url: String,
    /// SP5: Telegram alert channel (secrets.env). None = channel disabled.
    pub alert_telegram_bot_token: Option<String>,
    pub alert_telegram_chat_id: Option<String>,
    /// SP5: SpiderFoot base URL (LAN-bound sensor port).
    pub spiderfoot_url: String,
    // ── SP6 native monitor ─────────────────────────────────────────
    pub monitor_enabled: bool,
    /// "all" or a comma-separated subset of source names.
    pub monitor_sources: Vec<String>,
    pub monitor_geo_retention_days: u32,
    /// Source keys (secrets.env): None = that source degrades by design.
    pub monitor_firms_key: Option<String>,
    pub monitor_acled_email: Option<String>,
    pub monitor_acled_password: Option<String>,
    pub monitor_reliefweb_appname: Option<String>,
    /// NVD 2.0 API key (secrets.env, optional). None = 5 req/30s ceiling
    /// (sufficient for a 6h cadence on the HIGH+CRITICAL filter). With
    /// key set, ceiling rises to 50 req/30s. Free, request at
    /// https://nvd.nist.gov/developers/request-an-api-key.
    pub monitor_nvd_api_key: Option<String>,
    // ── SP6B finance collectors (secrets.env; None = collector degrades) ──
    pub fred_api_key: Option<String>,
    pub comtrade_api_key: Option<String>,
    pub bls_api_key: Option<String>,
    pub fmp_api_key: Option<String>,
    pub finnhub_api_key: Option<String>,
    pub financialdatasets_api_key: Option<String>,
    pub eia_api_key: Option<String>,
    // ── SP8-C social plane (watchlists: "handle|kind,handle|kind" — env
    //    override of the built-in defaults; kind is the pre-classifier default) ──
    pub bsky_watch: Vec<String>,
    pub tg_watch: Vec<String>,
    pub x_bearer_token: Option<String>,
    pub x_watch: Vec<String>,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

impl Config {
    pub fn from_env() -> Self {
        let embedding_model = env_or("EMBEDDING_MODEL", "text-embedding-3-small");
        let embedding_dims: u32 = env_or("EMBEDDING_DIMS", "1536").parse().unwrap_or(1536);
        let qdrant_collection = env_or(
            "QDRANT_COLLECTION",
            &format!(
                "evidence__{}__{}",
                embedding_model.replace(['/', ':', '.'], "-"),
                embedding_dims
            ),
        );
        Self {
            listen_addr: env_or("HUB_LISTEN_ADDR", "10.10.10.41:8800"),
            database_url: env_or(
                "DATABASE_URL",
                "postgres://intelhub:intelhub@172.30.2.10:5432/intelhub",
            ),
            redis_url: env_or("REDIS_URL", "redis://172.30.2.11:6379"),
            neo4j_uri: env_or("NEO4J_URI", "bolt://172.30.2.12:7687"),
            neo4j_user: env_or("NEO4J_USER", "neo4j"),
            neo4j_password: env_or("NEO4J_PASSWORD", "neo4j"),
            qdrant_url: env_or("QDRANT_URL", "http://172.30.2.13:6333"),
            qdrant_collection,
            searxng_url: env_or("SEARXNG_URL", "http://10.10.10.41:8080"),
            crawl4ai_url: env_or("CRAWL4AI_URL", "http://10.10.10.41:11235"),
            crawl4ai_api_token: env_or("CRAWL4AI_API_TOKEN", ""),
            embedding_base_url: env_or("EMBEDDING_BASE_URL", "https://ai.t8star.org/v1"),
            embedding_model,
            embedding_api_key: std::env::var("EMBEDDING_API_KEY").ok(),
            embedding_dims,
            raw_dir: env_or("HUB_RAW_DIR", "/home/zou/IntelHub/data/raw"),
            rate_limit_rpm: env_or("HUB_RATE_LIMIT_RPM", "120").parse().unwrap_or(120),
            simhash_max_hamming: env_or("SIMHASH_MAX_HAMMING", "3").parse().unwrap_or(3),
            mcp_allowed_hosts: env_or(
                "HUB_MCP_ALLOWED_HOSTS",
                "10.10.10.41,10.10.10.41:8800,localhost,127.0.0.1,::1,intel.local",
            )
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
            admin_token: std::env::var("HUB_ADMIN_TOKEN").ok().filter(|s| !s.is_empty()),
            budget_agent_tool_calls: env_or("HUB_BUDGET_AGENT_TOOL_CALLS", "2000").parse().unwrap_or(2000.0),
            budget_agent_crawl_pages: env_or("HUB_BUDGET_AGENT_CRAWL_PAGES", "300").parse().unwrap_or(300.0),
            budget_agent_embed_tokens: env_or("HUB_BUDGET_AGENT_EMBED_TOKENS", "1000000").parse().unwrap_or(1e6),
            budget_global_crawl_pages: env_or("HUB_BUDGET_GLOBAL_CRAWL_PAGES", "1000").parse().unwrap_or(1000.0),
            budget_global_embed_tokens: env_or("HUB_BUDGET_GLOBAL_EMBED_TOKENS", "5000000").parse().unwrap_or(5e6),
            alert_webhook_url: std::env::var("HUB_ALERT_WEBHOOK_URL").ok().filter(|s| !s.is_empty()),
            alert_webhook_min_severity: env_or("HUB_ALERT_WEBHOOK_MIN_SEVERITY", "warning"),
            manifests_dir: env_or("HUB_MANIFESTS_DIR", "/home/zou/IntelHub/manifests"),
            scripts_dir: env_or("HUB_SCRIPTS_DIR", "/home/zou/IntelHub/scripts"),
            // OSINT is naturally short-form (headlines, alert snippets, RSS
            // titles, Telegram messages). The previous 300-word floor rejected
            // 98% of our corpus (verified via PG: 461 docs <30w, 439 docs 30-99w,
            // only 5 docs ≥300w). 50 words covers tweet-equivalents and still
            // gates noise. Override via HUB_EMBED_MIN_WORDS env.
            embed_min_words: env_or("HUB_EMBED_MIN_WORDS", "50").parse().unwrap_or(50),
            embed_enabled: env_or("HUB_EMBED_WORKER_ENABLED", "true") == "true",
            rerank_enabled: env_or("HUB_RERANK_ENABLED", "false") == "true",
            rerank_model: env_or("HUB_RERANK_MODEL", "BAAI/bge-reranker-v2-m3"),
            rerank_top_k: env_or("HUB_RERANK_TOP_K", "50").parse().unwrap_or(50),
            rerank_timeout_secs: env_or("HUB_RERANK_TIMEOUT_SECS", "10").parse().unwrap_or(10),
            query_cache_enabled: env_or("HUB_QUERY_CACHE_ENABLED", "true") == "true",
            query_cache_ttl_secs: env_or("HUB_QUERY_CACHE_TTL_SECS", "300").parse().unwrap_or(300),
            llm_enabled: env_or("HUB_LLM_ENABLED", "false") == "true",
            llm_model: env_or("HUB_LLM_MODEL", "gpt-4.1-mini"),
            llm_timeout_ms: env_or("HUB_LLM_TIMEOUT_MS", "5000").parse().unwrap_or(5000),
            llm_max_tokens: env_or("HUB_LLM_MAX_TOKENS", "800").parse().unwrap_or(800),
            seed_enabled: env_or("HUB_SEED_ENABLED", "true") == "true",
            console_dir: env_or("HUB_CONSOLE_DIR", "/home/zou/IntelHub/console/dist"),
            prometheus_url: env_or("HUB_PROMETHEUS_URL", "http://172.30.3.20:9090"),
            alert_telegram_bot_token: std::env::var("HUB_ALERT_TELEGRAM_BOT_TOKEN").ok().filter(|s| !s.is_empty()),
            alert_telegram_chat_id: std::env::var("HUB_ALERT_TELEGRAM_CHAT_ID").ok().filter(|s| !s.is_empty()),
            spiderfoot_url: env_or("HUB_SPIDERFOOT_URL", "http://10.10.10.41:5001"),
            monitor_enabled: env_or("HUB_MONITOR_ENABLED", "true") == "true",
            monitor_sources: env_or("HUB_MONITOR_SOURCES", "all")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            monitor_geo_retention_days: env_or("HUB_MONITOR_GEO_RETENTION_DAYS", "30")
                .parse()
                .unwrap_or(30),
            monitor_firms_key: std::env::var("FIRMS_MAP_KEY").ok().filter(|s| !s.is_empty()),
            monitor_acled_email: std::env::var("ACLED_EMAIL").ok().filter(|s| !s.is_empty()),
            monitor_acled_password: std::env::var("ACLED_PASSWORD").ok().filter(|s| !s.is_empty()),
            monitor_reliefweb_appname: std::env::var("RELIEFWEB_APPNAME").ok().filter(|s| !s.is_empty()),
            monitor_nvd_api_key: std::env::var("HUB_NVD_API_KEY").ok().filter(|s| !s.is_empty()),
            fred_api_key: std::env::var("FRED_API_KEY").ok().filter(|s| !s.is_empty()),
            comtrade_api_key: std::env::var("COMTRADE_API_KEY").ok().filter(|s| !s.is_empty()),
            bls_api_key: std::env::var("BLS_API_KEY").ok().filter(|s| !s.is_empty()),
            fmp_api_key: std::env::var("FMP_API_KEY").ok().filter(|s| !s.is_empty()),
            finnhub_api_key: std::env::var("FINNHUB_API_KEY").ok().filter(|s| !s.is_empty()),
            financialdatasets_api_key: std::env::var("FINANCIALDATASETS_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            eia_api_key: std::env::var("EIA_API_KEY").ok().filter(|s| !s.is_empty()),
            bsky_watch: env_list("BSKY_WATCH"),
            tg_watch: env_list("TG_WATCH"),
            x_bearer_token: std::env::var("X_BEARER_TOKEN").ok().filter(|s| !s.is_empty()),
            x_watch: env_list("X_WATCH"),
        }
    }
}

/// Comma-separated env list → trimmed non-empty entries (empty when unset).
fn env_list(var: &str) -> Vec<String> {
    std::env::var(var)
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
