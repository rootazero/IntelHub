use std::sync::Arc;
use tokio::sync::broadcast;

use crate::types::BusEvent;
use crate::{Config, Result};

/// Shared application state: connection pools + config + event broadcast.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pg: sqlx::PgPool,
    pub redis: redis::aio::MultiplexedConnection,
    pub neo4j: Arc<neo4rs::Graph>,
    pub http: reqwest::Client,
    pub event_tx: broadcast::Sender<BusEvent>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);

        let pg = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect(&config.database_url)
            .await?;
        sqlx::migrate!("../../migrations")
            .run(&pg)
            .await
            .map_err(|e| crate::error::HubError::internal(format!("migration failed: {e}")))?;

        let redis_client = redis::Client::open(config.redis_url.clone())?;
        let redis = redis_client.get_multiplexed_async_connection().await?;

        let neo4j = neo4rs::Graph::new(
            &config.neo4j_uri,
            &config.neo4j_user,
            &config.neo4j_password,
        )
        .await?;

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .user_agent("intelhub-core/0.1 (SP2A)")
            .build()?;

        let (event_tx, _) = broadcast::channel(1024);

        Ok(Self {
            config,
            pg,
            redis,
            neo4j: Arc::new(neo4j),
            http,
            event_tx,
        })
    }
}
