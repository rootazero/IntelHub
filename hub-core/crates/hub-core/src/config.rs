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
        }
    }
}
