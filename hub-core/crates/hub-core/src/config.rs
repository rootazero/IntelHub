//! Configuration via environment variables (12-factor; secrets come from
//! systemd EnvironmentFile). Non-secret defaults are sensible for the
//! IntelHub deployment documented in the SP2A spec.

use std::collections::HashMap;
use std::sync::OnceLock;

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
    /// OpenSanctions API key (secrets.env, optional). Free tier is
    /// key-gated; without it the opensanctions collector stays
    /// shelved-by-design. Apply at
    /// https://www.opensanctions.org/api/ (free for open-data use).
    pub monitor_opensanctions_api_key: Option<String>,
    /// OSV watchlist (env override). Each entry is
    /// "ecosystem:package" — e.g. "PyPI:django,npm:lodash". When empty,
    /// the built-in DEFAULT_WATCH in osv.rs is used (12 packages
    /// common to OSINT infrastructure).
    pub monitor_osv_watch: Vec<String>,
    /// SEC EDGAR EFTS form filter (secrets.env, optional). Default "8-K"
    /// (current reports = material events). Other useful values: "10-K"
    /// (annual reports), "10-Q" (quarterly), "4" (insider Form 4
    /// transactions), "DEF 14A" (proxy statements).
    pub monitor_sec_form: String,
    /// SEC EDGAR User-Agent contact email (secrets.env). Required per
    /// SEC fair-access policy. Falls back to a generic placeholder
    /// when unset — operators should set this to a real contact.
    pub monitor_sec_user_agent_email: Option<String>,
    /// Etherscan API key (secrets.env, optional). Free tier is keyless
    /// (1 req/5s, last 100 txs) and sufficient for the default watchlist
    /// at 6h cadence. With key, ceiling rises to 5 req/s. Free; obtain at
    /// https://etherscan.io/apis (MyEtherscan → API Keys).
    pub monitor_etherscan_api_key: Option<String>,
    /// Etherscan watchlist override (env). Each entry is
    /// "label:0xADDRESS[:lat:lon]" — e.g. "tornado-router:0xd90e...:38.9:-77.0".
    /// Lat/lon optional, defaults to Singapore (Etherscan.io operator).
    /// When empty, the built-in DEFAULT_WATCH (Tornado router, Binance
    /// hot, Coinbase hot) is used.
    pub monitor_etherscan_watch: Vec<String>,
    /// Large-tx threshold in ETH (env, optional). Signals below this
    /// per-address-day volume are dropped. Default 100 ETH.
    pub monitor_etherscan_min_eth: Option<f64>,
    /// DefiLlama watchlist override (env). Each entry is
    /// "slug[:label[:lat:lon]]" — e.g. "aave:aave:51.5:-0.13".
    /// When empty, built-in DEFAULT_WATCH (aave/uniswap/makerdao/curve/lido)
    /// is used. DefiLlama is keyless free for all endpoints.
    pub monitor_defillama_watch: Vec<String>,
    /// DefiLlama 24h TVL change threshold (%, absolute). Signals below
    /// this delta are dropped (noise-floor filter). Default 15%.
    pub monitor_defillama_threshold_pct: Option<f64>,
    /// OTX lookback window in days (env, optional). Default 1.
    /// OTX public pulses endpoint is keyless; large windows (7-30) are
    /// useful for first-run catch-up sweeps.
    pub monitor_otx_lookback_days: Option<u32>,
    /// OTX API key (secrets.env, REQUIRED). Free signup at
    /// https://otx.alienvault.com/api — all OTX public-listing endpoints
    /// are auth-gated (verified 2026-09: /pulses/subscribed 403 without
    /// auth, /pulses/search 404, /search/pulses 403, /pulses/public 504).
    /// Without this key the OTX collector is shelved-by-design.
    pub monitor_otx_api_key: Option<String>,
    /// urlscan.io API key (secrets.env, optional). Free tier is keyless
    /// (~100 req/day, hard ceiling); with key the ceiling rises to 5k/day.
    /// Free signup at https://urlscan.io/user/signup.
    pub monitor_urlscan_api_key: Option<String>,
    /// urlscan.io search query (env, optional). Default is broad OSINT-
    /// ecosystem surveillance. Override with any urlscan-compatible
    /// Lucene query string.
    pub monitor_urlscan_query: String,
    /// GFW API token (secrets.env). Free for non-commercial use at
    /// https://globalfishingwatch.org/our-apis/. Without this the GFW
    /// collector stays shelved-by-design (visible on health board, 0 events).
    pub monitor_gfw_token: Option<String>,
    /// GFW lookback window in days (env, optional). Default 7.
    pub monitor_gfw_lookback_days: Option<u32>,
    /// GFW query body (env, optional, JSON). When set, replaces the default
    /// South-China-Sea port-visits query. Must be valid JSON for the GFW
    /// Events API v3 — date placeholders REPLACE_START and REPLACE_END are
    /// substituted from the lookback window.
    pub monitor_gfw_query: Option<String>,
    pub monitor_overpass_watch: Vec<String>,
    pub monitor_ahmia_query: Vec<String>,
    pub monitor_opencorp_api_token: Option<String>,
    pub monitor_opencorp_watch: Vec<String>,
    pub monitor_otx_create_claim: bool,
    /// Shodan InternetDB watchlist (env CSV). Each entry is an IPv4
    /// literal. Default = 10 well-known public IPs (DNS resolvers +
    /// CDN edges) shipped in shodan_internetdb.rs so day-1 emits
    /// real CPE / port / vuln signal instead of empty state.
    pub monitor_shodan_internetdb_watch: Vec<String>,
    /// crt.sh watchlist (env CSV). Each entry is a crt.sh query
    /// domain (exact match, `%sub`, or `%.domain` wildcards). Default
    /// = 5 high-traffic critical-infra domains shipped in crtsh.rs.
    pub monitor_crtsh_watch: Vec<String>,
    /// blocklist.de feed name (env, optional). Single string,
    /// not CSV — one feed at a time. Default = `ssh` (SSH
    /// brute-force attackers). Other useful feeds: `mail`,
    /// `apache`, `ftp`, `bots`, `strongips`, `all`. See
    /// https://lists.blocklist.de/lists/ for the full directory.
    pub monitor_blocklist_de_feed: Option<String>,
    /// Nominatim (OpenStreetMap) geocoding query watchlist
    /// (env CSV). Each entry is a free-form search string
    /// (e.g. "Tor Project Seattle", "NSO Group Herzliya").
    /// Default = 5 threat-actor HQ queries shipped in
    /// nominatim.rs so day-1 surfaces variety of OSM
    /// class/subtype hits.
    pub monitor_nominatim_queries: Vec<String>,
    /// RIPEstat abuse-contact-finder IP watchlist (env CSV).
    /// Each entry is an IPv4 or IPv6 address. Default = 5
    /// well-known IPs (Cloudflare DNS / Google DNS / Quad9 /
    /// OpenDNS / GitHub) shipped in ripestat.rs so day-1
    /// surfaces the IRIR contact-lookup flow.
    pub monitor_ripestat_watch: Vec<String>,
    /// Wayback Machine URL watchlist (env CSV). Each entry
    /// is a hostname (e.g. "google.com") or full URL. Default
    /// = 5 well-known hostnames shipped in wayback.rs so
    /// day-1 surfaces the archive-snapshot lookup flow.
    pub monitor_wayback_watch: Vec<String>,
    /// RIPE stat as-overview ASN watchlist (env CSV). Each
    /// entry is an AS number (without 'AS' prefix, e.g.
    /// "13335" for Cloudflare). Default = 5 well-known ASes
    /// (Cloudflare / Google / DigitalOcean / Amazon /
    /// GitHub) shipped in ripe_as_overview.rs so day-1
    /// surfaces the AS-holder attribution flow.
    pub monitor_ripe_as_overview_watch: Vec<String>,
    /// ipapi.co IP-geolocation watchlist (env CSV). Each
    /// entry is an IPv4 or IPv6 address. Default = 5
    /// well-known IPs (Cloudflare DNS / Google DNS / Quad9 /
    /// OpenDNS / GitHub) shipped in ipapi_co.rs so day-1
    /// surfaces variety of geo + ASN metadata.
    pub monitor_ipapi_co_watch: Vec<String>,
    /// ip-api.com IP-geolocation watchlist (env CSV). Same
    /// shape as ipapi_co — typically the same watchlist.
    /// Default = 5 well-known IPs (same default set as
    /// ipapi_co) shipped in ip_api_com.rs.
    pub monitor_ip_api_com_watch: Vec<String>,
    /// RIPE stat prefix-overview IP/prefix watchlist
    /// (env CSV). Each entry is an IP or CIDR. Default = 5
    /// well-known IPs (same default set as ripe_as_overview)
    /// shipped in ripe_prefix_overview.rs so day-1 surfaces
    /// variety of BGP-routing attribution.
    pub monitor_ripe_prefix_overview_watch: Vec<String>,
    /// Wikidata watchlist (env CSV). Each entry is a Wikidata Q-ID
    /// (e.g. `Q113481936` for Tornado Cash). Default = 10 sanctioned
    /// crypto mixers / APT groups / regime actors shipped in wikidata.rs.
    pub monitor_wikidata_watch: Vec<String>,
    /// CourtListener search terms (env CSV). Each entry is a free-text
    /// query against the dockets endpoint. Default = 8 high-signal
    /// terms (lockbit, tornado cash, APT names) shipped in
    /// courtlistener.rs.
    pub monitor_courtlistener_query: Vec<String>,
    /// Leaksify watchlist (env CSV). Each entry is an email or
    /// username. Default = 5 sentinel entries tied to sanctioned
    /// crypto-mixer / APT personas shipped in leaksify.rs.
    /// IMPORTANT: keep the watchlist short — every email lookup
    /// surfaces in geo_events metadata (target_hash only — not the
    /// literal email), so large watchlists add radar noise.
    pub monitor_leaksify_query: Vec<String>,
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
            monitor_opensanctions_api_key: std::env::var("HUB_OPENSANCTIONS_API_KEY").ok().filter(|s| !s.is_empty()),
            monitor_osv_watch: env_list("HUB_OSV_WATCH"),
            monitor_sec_form: env_or("HUB_SEC_FORM", "8-K"),
            monitor_sec_user_agent_email: std::env::var("HUB_SEC_USER_AGENT_EMAIL").ok().filter(|s| !s.is_empty()),
            monitor_etherscan_api_key: std::env::var("ETHERSCAN_API_KEY").ok().filter(|s| !s.is_empty()),
            monitor_etherscan_watch: env_list("HUB_ETHERSCAN_WATCH"),
            monitor_etherscan_min_eth: std::env::var("HUB_ETHERSCAN_MIN_ETH")
                .ok()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok()),
            monitor_defillama_watch: env_list("HUB_DEFILLAMA_WATCH"),
            monitor_defillama_threshold_pct: std::env::var("HUB_DEFILLAMA_THRESHOLD_PCT")
                .ok()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok()),
            monitor_otx_lookback_days: std::env::var("HUB_OTX_DAYS")
                .ok()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok()),
            monitor_otx_api_key: std::env::var("OTX_API_KEY").ok().filter(|s| !s.is_empty()),
            monitor_urlscan_api_key: std::env::var("URLSCAN_API_KEY").ok().filter(|s| !s.is_empty()),
            monitor_urlscan_query: env_or("HUB_URLSCAN_QUERY", ""),
            monitor_gfw_token: std::env::var("GFW_API_TOKEN").ok().filter(|s| !s.is_empty()),
            monitor_overpass_watch: env_list("HUB_OVERPASS_WATCH"),
            monitor_ahmia_query: env_list("HUB_AHMIA_QUERY"),
            monitor_opencorp_api_token: std::env::var("OPENCORP_API_TOKEN").ok().filter(|s| !s.is_empty()),
            monitor_opencorp_watch: env_list("HUB_OPENCORP_WATCH"),
            monitor_otx_create_claim: std::env::var("HUB_OTX_CREATE_CLAIM")
                .ok()
                .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(false),
            monitor_shodan_internetdb_watch: env_list("HUB_SHODAN_INTERNETDB_WATCH"),
            monitor_crtsh_watch: env_list("HUB_CRTSH_WATCH"),
            monitor_blocklist_de_feed: std::env::var("HUB_BLOCKLIST_DE_FEED").ok(),
            monitor_nominatim_queries: env_list("HUB_NOMINATIM_QUERIES"),
            monitor_ripestat_watch: env_list("HUB_RIPESTAT_WATCH"),
            monitor_wayback_watch: env_list("HUB_WAYBACK_WATCH"),
            monitor_ripe_as_overview_watch: env_list("HUB_RIPE_AS_OVERVIEW_WATCH"),
            monitor_ipapi_co_watch: env_list("HUB_IPAPI_CO_WATCH"),
            monitor_ip_api_com_watch: env_list("HUB_IP_API_COM_WATCH"),
            monitor_ripe_prefix_overview_watch: env_list("HUB_RIPE_PREFIX_OVERVIEW_WATCH"),
            monitor_wikidata_watch: env_list("HUB_WIKIDATA_WATCH"),
            monitor_courtlistener_query: env_list("HUB_COURTLISTENER_QUERY"),
            monitor_leaksify_query: env_list("HUB_LEAKSIFY_QUERY"),
            monitor_gfw_lookback_days: std::env::var("HUB_GFW_LOOKBACK_DAYS")
                .ok()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok()),
            monitor_gfw_query: std::env::var("HUB_GFW_QUERY").ok().filter(|s| !s.is_empty()),
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

/// Caller identity tier. Used in `agents.tier` and applied by `apply_tier_filter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Free,
    Paid,
    Admin,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Free => "free",
            Tier::Paid => "paid",
            Tier::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "free" => Some(Tier::Free),
            "paid" => Some(Tier::Paid),
            "admin" => Some(Tier::Admin),
            _ => None,
        }
    }
}

/// License classification. Decoupled from Tier so a paid-API source could
/// theoretically still be Open-licensed (rare; reserved for future cases).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseClass {
    Open,           // public domain, ODbL, CC-BY, MIT-style
    FairUse,        // fair-use bounded; redistribution restricted
    Restricted,     // paid API; redistribution prohibited
    NonCommercial,  // CC-BY-NC or equivalent; blocks commercial use
}

impl LicenseClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            LicenseClass::Open => "open",
            LicenseClass::FairUse => "fair-use",
            LicenseClass::Restricted => "restricted",
            LicenseClass::NonCommercial => "non-commercial",
        }
    }
}

/// Per-monitor metadata. Source of truth for tier/license/staleness.
#[derive(Debug, Clone, Copy)]
pub struct MonitorMeta {
    pub tier_required: Tier,
    pub license_class: LicenseClass,
    pub data_age_hours: u32,
    pub doc_url: &'static str,
}

/// Returns the per-monitor metadata map. Once-initialized, lock-free reads.
///
/// The license/tier classification here is the **single source of truth**.
/// `monitor/mod.rs` registry, SQL migrations' backfill, REST/MCP handlers,
/// and the `/api/v1/sources` endpoint all derive from this map.
pub fn monitor_metadata() -> &'static HashMap<&'static str, MonitorMeta> {
    static CACHE: OnceLock<HashMap<&'static str, MonitorMeta>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut m: HashMap<&'static str, MonitorMeta> = HashMap::with_capacity(32);

        // US-gov / UN / WHO — public domain
        m.insert("usgs",          MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://earthquake.usgs.gov/earthquakes/feed/v1.0/geojson.php" });
        m.insert("noaa",          MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "https://www.weather.gov/documentation/services-web-api" });
        m.insert("epa-radnet",    MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.epa.gov/enviro/web-services" });
        m.insert("eonet",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "https://eonet.gsfc.nasa.gov/api/v3/" });
        m.insert("who",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.who.int/" });
        m.insert("climate",       MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.ncei.noaa.gov/access/monitoring/climate-at-a-glance/" });
        m.insert("radiation",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "https://www.epa.gov/radnet" });

        // US-gov economic data — public domain
        m.insert("firms",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 3,  doc_url: "https://firms.modaps.eosdis.nasa.gov/api/" });
        m.insert("bls",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.bls.gov/developers/" });
        m.insert("fred",          MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://fred.stlouisfed.org/docs/api/" });
        m.insert("eia",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.eia.gov/opendata/" });
        m.insert("treasury",      MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://home.treasury.gov/developers" });
        m.insert("comtrade",      MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 720,doc_url: "https://comtrade.un.org/data/doc/api" });
        m.insert("usaspending",   MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.usaspending.gov/disbursement/Transparency" });
        m.insert("gscpi",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 720,doc_url: "https://www.newyorkfed.org/markets/global-supply-chain-pressure-index" });
        m.insert("sec-edgar",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://efts.sec.gov/LATEST/search-index" });
        m.insert("ofac",          MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://sanctionssearch.ofac.treas.gov/" });

        // ODbL / CC-BY
        m.insert("gdelt",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "https://blog.gdeltproject.org/gdelt-2-0-english-translation-api/" });
        m.insert("acled",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://acleddata.com/api-documentation/" });
        m.insert("reliefweb",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://reliefweb.int/help/api" });
        m.insert("kiwisdr",       MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "http://kiwisdr.com/" });
        m.insert("nvd",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://nvd.nist.gov/developers" });
        m.insert("cisa-kev",      MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.cisa.gov/known-exploited-vulnerabilities-catalog" });
        m.insert("osv",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://google.github.io/osv.dev/" });
        m.insert("opensanctions", MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.opensanctions.org/api/" });
        m.insert("etherscan",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://etherscan.io/apis" });
        m.insert("defillama",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 4,  doc_url: "https://api.llama.fi/" });
        m.insert("otx",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 4,  doc_url: "https://otx.alienvault.com/api" });
        m.insert("urlscan",       MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::FairUse, data_age_hours: 4,  doc_url: "https://urlscan.io/docs/api/" });
        m.insert("gfw",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::FairUse, data_age_hours: 12, doc_url: "https://globalfishingwatch.org/our-apis/" });

        // Paid APIs — redistributability restricted
        m.insert("x",             MonitorMeta { tier_required: Tier::Paid, license_class: LicenseClass::Restricted, data_age_hours: 1, doc_url: "https://developer.twitter.com/en/docs/twitter-api" });
        m.insert("bluesky",       MonitorMeta { tier_required: Tier::Paid, license_class: LicenseClass::Restricted, data_age_hours: 1, doc_url: "https://docs.bsky.app/docs/api" });
        m.insert("finintel",      MonitorMeta { tier_required: Tier::Paid, license_class: LicenseClass::Restricted, data_age_hours: 1, doc_url: "https://finnhub.io/docs/api" });
        m.insert("markets",       MonitorMeta { tier_required: Tier::Paid, license_class: LicenseClass::Restricted, data_age_hours: 1, doc_url: "https://financialmodelingprep.com/developer/docs/" });

        // Fair-use (free tier, redistribution limited)
        m.insert("telegram-watch", MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::FairUse, data_age_hours: 1, doc_url: "https://core.telegram.org/api" });
        m.insert("rss",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::FairUse, data_age_hours: 1, doc_url: "internal://rss-aggregator" });

        // Admin-only — NC clause (per prior wontfix analysis)
        m.insert("opensky",       MonitorMeta { tier_required: Tier::Admin, license_class: LicenseClass::NonCommercial, data_age_hours: 1, doc_url: "https://opensky-network.org/apidoc/" });

        m
    })
}
