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
    // ── SP6B finance collectors (secrets.env; None = collector degrades) ──
    pub fred_api_key: Option<String>,
    pub fmp_api_key: Option<String>,
    pub finnhub_api_key: Option<String>,
    pub financialdatasets_api_key: Option<String>,
    pub eia_api_key: Option<String>,
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
            embed_min_words: env_or("HUB_EMBED_MIN_WORDS", "300").parse().unwrap_or(300),
            embed_enabled: env_or("HUB_EMBED_WORKER_ENABLED", "true") == "true",
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
            fred_api_key: std::env::var("FRED_API_KEY").ok().filter(|s| !s.is_empty()),
            fmp_api_key: std::env::var("FMP_API_KEY").ok().filter(|s| !s.is_empty()),
            finnhub_api_key: std::env::var("FINNHUB_API_KEY").ok().filter(|s| !s.is_empty()),
            financialdatasets_api_key: std::env::var("FINANCIALDATASETS_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            eia_api_key: std::env::var("EIA_API_KEY").ok().filter(|s| !s.is_empty()),
        }
    }
}
