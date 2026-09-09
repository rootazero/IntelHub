use thiserror::Error;

pub type Result<T> = std::result::Result<T, HubError>;

#[derive(Debug, Error)]
pub enum HubError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("neo4j error: {0}")]
    Neo4j(#[from] neo4rs::Error),

    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unauthorized")]
    Unauthorized,

    #[error("rate limited")]
    RateLimited,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("upstream sensor error: {0}")]
    Sensor(String),

    #[error("graph service unavailable: {0}")]
    GraphUnavailable(String),

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error("budget denied ({state}): {reason}")]
    BudgetDenied { state: String, reason: String },

    #[error("internal error: {0}")]
    Internal(String),
}

impl HubError {
    pub fn policy_denied(msg: impl Into<String>) -> Self {
        HubError::PolicyDenied(msg.into())
    }

impl HubError {
    pub fn internal(msg: impl Into<String>) -> Self {
        HubError::Internal(msg.into())
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        HubError::BadRequest(msg.into())
    }
    pub fn sensor(msg: impl Into<String>) -> Self {
        HubError::Sensor(msg.into())
    }
}
