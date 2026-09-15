//! Admin CLI helpers (agent provisioning + key rotation). Keys are printed
//! ONCE; only the SHA-256 hash is stored.

use crate::config::Config;
use crate::config::Tier;
use crate::store::update_agent_tier;

pub async fn create_agent_cli(cfg: &Config, name: &str, version: Option<&str>) -> anyhow::Result<()> {
    let pg = sqlx::PgPool::connect(&cfg.database_url).await?;
    sqlx::migrate!("../../migrations").run(&pg).await?;
    let (agent_id, key) = crate::store::create_agent(&pg, name, version).await?;
    println!("agent created");
    println!("agent_id: {agent_id}");
    println!("name:     {name}");
    println!("api_key:  {key}");
    eprintln!("NOTE: the api_key is shown once; store it in the agent's MCP client config.");
    Ok(())
}

pub async fn rotate_agent_key_cli(cfg: &Config, name: &str) -> anyhow::Result<()> {
    let pg = sqlx::PgPool::connect(&cfg.database_url).await?;
    sqlx::migrate!("../../migrations").run(&pg).await?;
    let (agent_id, key) = crate::store::rotate_agent_key(&pg, name).await?;
    println!("agent key rotated");
    println!("agent_id: {agent_id}");
    println!("name:     {name}");
    println!("api_key:  {key}");
    eprintln!("NOTE: the previous api_key is now revoked. Store the new one in the agent's MCP client config.");
    Ok(())
}

/// Set the tier of an existing agent. Privileged: must be invoked from
/// the systemd hub-core CLI dispatcher (which only the host operator can
/// run via sudo). Does NOT check who's calling — privilege is gated by
/// filesystem permission on the hub-core binary + secrets.env.
///
/// Writes an audit_log row via direct INSERT (no caller agent exists for
/// CLI invocations, so api_key_prefix='cli').
pub async fn set_tier_cli(cfg: &Config, name: &str, tier_str: &str) -> anyhow::Result<()> {
    let tier = Tier::parse(tier_str).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid tier '{tier_str}' (expected 'free' | 'paid' | 'admin')"
        )
    })?;

    let pg = sqlx::PgPool::connect(&cfg.database_url).await?;
    sqlx::migrate!("../../migrations").run(&pg).await?;

    let agent_id = update_agent_tier(&pg, name, tier).await?;

    // Audit row (best-effort).
    let _ = sqlx::query(
        "INSERT INTO audit_log
            (ts, agent_id, api_key_prefix, action, source_attempted, request_path, trace_id, blocked)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(chrono::Utc::now())
    .bind(agent_id)
    .bind("cli")
    .bind("set_tier")
    .bind(Some(tier.as_str()))
    .bind(None::<&str>)
    .bind(None::<uuid::Uuid>)
    .bind(false)
    .execute(&pg)
    .await;

    println!("agent tier updated");
    println!("agent_id: {agent_id}");
    println!("name:     {name}");
    println!("tier:     {}", tier.as_str());
    Ok(())
}
