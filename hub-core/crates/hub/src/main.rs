use std::sync::Arc;

use anyhow::Result;
use hub_core::{AppState, Config};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,rmcp=warn".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let cfg = Config::from_env();

    match args.get(1).map(String::as_str) {
        Some("create-agent") => {
            // hub create-agent --name <name> [--version <ver>]
            let name = flag_value(&args, "--name")
                .ok_or_else(|| anyhow::anyhow!("usage: hub create-agent --name <name> [--version <ver>]"))?;
            let version = flag_value(&args, "--version");
            hub_core::admin::create_agent_cli(&cfg, &name, version.as_deref()).await
        }
        Some("rotate-agent-key") => {
            // hub rotate-agent-key --name <name>
            // Soft-revoke the agent's previous api_key and mint a new one.
            // agent_id is preserved; clients see a fresh bearer key.
            let name = flag_value(&args, "--name")
                .ok_or_else(|| anyhow::anyhow!("usage: hub rotate-agent-key --name <name>"))?;
            hub_core::admin::rotate_agent_key_cli(&cfg, &name).await
        }
        Some("set-tier") => {
            // hub set-tier <agent-name> <free|paid|admin>
            // Privileged: only the host operator can run this (filesystem
            // permission on the binary + secrets.env). Writes an audit row.
            let usage = "usage: core/hub set-tier <agent-name> <free|paid|admin>";
            let name = args.get(2).cloned().unwrap_or_else(|| {
                eprintln!("{usage}");
                std::process::exit(2);
            });
            let tier = args.get(3).cloned().unwrap_or_else(|| {
                eprintln!("{usage}");
                std::process::exit(2);
            });
            hub_core::admin::set_tier_cli(&cfg, &name, &tier).await
        }
        Some("dump-series-catalog") => {
            // hub dump-series-catalog > series_catalog.json
            // Emits the static series catalog as JSON for the console
            // build pipeline (build-console.sh calls this before npm run build).
            // No DB / state — the catalog is a const slice.
            hub_core::series::dump_catalog_json()
        }
        _ => {
            let state = Arc::new(AppState::new(cfg).await?);
            hub_core::server::serve(state).await
        }
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
