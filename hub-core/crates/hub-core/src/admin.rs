//! Admin CLI helpers (agent provisioning). Keys are printed ONCE; only the
//! SHA-256 hash is stored.

use crate::config::Config;

pub async fn create_agent_cli(cfg: &Config, name: &str, version: Option<&str>) -> anyhow::Result<()> {
    let pg = sqlx::PgPool::connect(&cfg.database_url).await?;
    sqlx::migrate!("../../../migrations").run(&pg).await?;
    let (agent_id, key) = crate::store::create_agent(&pg, name, version).await?;
    println!("agent created");
    println!("agent_id: {agent_id}");
    println!("name:     {name}");
    println!("api_key:  {key}");
    eprintln!("NOTE: the api_key is shown once; store it in the agent's MCP client config.");
    Ok(())
}
