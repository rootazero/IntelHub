use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::types::BusEvent;
use crate::{Config, Result};

/// Shared application state: connection pools + config + event broadcast.
///
/// Redis goes through a supervised slot: a silently-dead (half-open) TCP
/// connection to the bridge IP must never wedge request paths (observed in
/// SP3 acceptance: stale MultiplexedConnection never errored, every command
/// hung forever). `redis()` hands out the current live connection; the
/// supervisor (server.rs) PINGs with a hard timeout and replaces the slot on
/// failure.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pg: sqlx::PgPool,
    pub redis_client: redis::Client,
    redis_slot: Arc<RwLock<redis::aio::MultiplexedConnection>>,
    pub neo4j: Arc<neo4rs::Graph>,
    pub http: reqwest::Client,
    pub event_tx: broadcast::Sender<BusEvent>,
}

impl AppState {
    /// Current live redis connection (cheap clone of the multiplexed handle).
    pub async fn redis(&self) -> redis::aio::MultiplexedConnection {
        self.redis_slot.read().await.clone()
    }

    /// Timed redis command — degrades to None instead of hanging while the
    /// connection is mid-failure (the supervisor restores it within seconds).
    pub async fn redis_timed<T>(&self, cmd: redis::Cmd, ms: u64) -> Option<T>
    where
        T: redis::FromRedisValue + Send,
    {
        let mut conn = self.redis().await;
        tokio::time::timeout(std::time::Duration::from_millis(ms), cmd.query_async::<T>(&mut conn))
            .await
            .ok()
            .and_then(|r| r.ok())
    }

    /// Replace the connection with a fresh one (called by the supervisor when
    /// the current one fails a timed PING).
    pub async fn redis_reconnect(&self) -> bool {
        match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            self.redis_client.get_multiplexed_async_connection(),
        )
        .await
        {
            Ok(Ok(conn)) => {
                *self.redis_slot.write().await = conn;
                tracing::warn!("redis connection replaced after failure");
                true
            }
            _ => false,
        }
    }

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

        // PG migrations are durable above; now ensure Neo4j constraints/indexes.
        // Idempotent (IF NOT EXISTS) so re-runs are safe. Failures bubble up.
        crate::neo4j_init::ensure_neo4j_schema(&neo4j).await?;
        tracing::info!(target: "hub.boot", "neo4j schema ensured");

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .user_agent("intelhub-core/0.1 (SP3)")
            // Per-host pool cap (fix/deploy-stampede Layer 1). The shared
            // reqwest client is used by 74 monitor sources + cctv health
            // probes (33 cameras sharing hosts) + gev_cctv frame/media
            // proxy. Default reqwest pool is unbounded — on hub-core
            // restart, all 74 sources spawn concurrently and saturate
            // openclash on 10.10.10.1 (fake-IP), which cascades into the
            // PVE40 host network outage documented in
            // docs/superpowers/execution/2026-09-20-deploy-stampede-postmortem.md.
            // 8 per host caps the worst-case fan-out while still letting
            // one source drain a host at full TCP-window speed.
            .pool_max_idle_per_host(8)
            .build()?;

        let (event_tx, _) = broadcast::channel(1024);

        Ok(Self {
            config,
            pg,
            redis_client,
            redis_slot: Arc::new(RwLock::new(redis)),
            neo4j: Arc::new(neo4j),
            http,
            event_tx,
        })
    }
}
