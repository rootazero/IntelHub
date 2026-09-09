//! Component Lifecycle — read plane (directive §68–69) + controlled Level 3
//! actions. Status/versions/update-available are aggregated from manifests,
//! `docker ps` and registry tag queries (Redis-cached 6h). Mutations are NOT
//! orchestrated by the hub: whitelisted actions invoke the pinned VM scripts
//! (upgrade.sh/rollback.sh/backup.sh) with enumerated arguments only.

use serde_json::{json, Value};

use crate::error::{HubError, Result};
use crate::state::AppState;

/// (name, hub repo for tag checks, tag prefix filter, required suffix)
const COMPONENTS: &[(&str, &str, &str, &str)] = &[
    ("postgres", "library/postgres", "17.", "-trixie"),
    ("redis", "library/redis", "7.", "-alpine"),
    ("neo4j", "library/neo4j", "5.", "-community"),
    ("qdrant", "qdrant/qdrant", "v1.", ""),
    ("searxng", "searxng/searxng", "", ""),
    ("crawl4ai", "unclecode/crawl4ai", "", ""),
];

/// Components the pinned scripts know how to mutate (§6 whitelist).
const SCRIPT_COMPONENTS: &[&str] = &[
    "postgres", "redis", "neo4j", "qdrant", "searxng", "crawl4ai",
];
const ACTIONS: &[&str] = &["upgrade", "rollback", "backup"];

/// Docker Hub v2 tags API → latest tag matching (prefix, suffix, numeric-only).
async fn latest_tag(state: &AppState, repo: &str, prefix: &str, suffix: &str) -> Option<String> {
    let cache = format!("hub:component_latest:{repo}");
    let mut conn = state.redis.clone();
    if let Ok(Some(c)) = redis::cmd("GET")
        .arg(&cache)
        .query_async::<Option<String>>(&mut conn)
        .await
    {
        return if c.is_empty() { None } else { Some(c) };
    }
    let url = format!(
        "https://hub.docker.com/v2/repositories/{repo}/tags?page_size=100&ordering=last_updated"
    );
    let body: Value = state.http.get(&url).send().await.ok()?.json().await.ok()?;
    let mut best: Option<(Vec<u64>, String)> = None;
    if let Some(results) = body.get("results").and_then(|r| r.as_array()) {
        for t in results {
            let Some(name) = t.get("name").and_then(|n| n.as_str()) else { continue };
            if !name.starts_with(prefix) || !name.ends_with(suffix) {
                continue;
            }
            // Numeric core must be pure digits+dots (excludes rc/beta/snapshot).
            let core = name
                .trim_start_matches('v')
                .trim_end_matches(suffix)
                .to_string();
            if core.is_empty() || !core.chars().all(|c| c.is_ascii_digit() || c == '.') {
                continue;
            }
            let key: Vec<u64> = core.split('.').filter_map(|p| p.parse().ok()).collect();
            if key.is_empty() {
                continue;
            }
            let better = match &best {
                None => true,
                Some((bk, _)) => key > *bk,
            };
            if better {
                best = Some((key, name.to_string()));
            }
        }
    }
    let found = best.map(|(_, n)| n).unwrap_or_default();
    let _: redis::RedisResult<()> = redis::cmd("SET")
        .arg(&cache)
        .arg(&found)
        .arg("EX")
        .arg(21600) // 6h
        .query_async(&mut conn)
        .await;
    if found.is_empty() { None } else { Some(found) }
}

/// Version-ish comparison: numeric cores only; false when not comparable.
fn is_newer(latest: &str, installed: &str) -> bool {
    let core = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split(['.', '-'])
            .take_while(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let (l, i) = (core(latest), core(installed));
    !l.is_empty() && !i.is_empty() && l > i
}

/// Full component listing for /api/v1/components (§68–69).
pub async fn list_components(state: &AppState) -> Value {
    // Live container states (native hub is in the docker group).
    let ps = tokio::process::Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}|{{.Image}}|{{.Status}}|{{.State}}"])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    // Manifests (best effort).
    let mut manifests: serde_json::Map<String, Value> = serde_json::Map::new();
    if let Ok(mut rd) = tokio::fs::read_dir(&state.config.manifests_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            if let Ok(text) = tokio::fs::read_to_string(entry.path()).await {
                if let Ok(y) = serde_yaml::from_str::<serde_yaml::Value>(&text) {
                    if let Ok(j) = serde_json::to_value(y) {
                        if let Some(name) = j.get("name").and_then(|n| n.as_str()) {
                            manifests.insert(name.to_string(), j);
                        }
                    }
                }
            }
        }
    }

    let mut items = Vec::new();
    for (name, repo, prefix, suffix) in COMPONENTS {
        let container = format!("intelhub-{name}");
        let (installed, state_str, status) = ps
            .lines()
            .find(|l| l.starts_with(&format!("{container}|")))
            .map(|l| {
                let mut parts = l.split('|');
                let _ = parts.next();
                let image = parts.next().unwrap_or("");
                let status = parts.next().unwrap_or("").to_string();
                let st = parts.next().unwrap_or("unknown").to_string();
                let tag = image.rsplit(':').next().unwrap_or("").to_string();
                (tag, st, status)
            })
            .unwrap_or_else(|| ("".into(), "absent".into(), "".into()));
        let latest = latest_tag(state, repo, prefix, suffix).await;
        let update_available = match (&latest, installed.is_empty()) {
            (Some(l), false) => is_newer(l, &installed),
            _ => false,
        };
        items.push(json!({
            "name": name,
            "container": container,
            "state": state_str,
            "status": status,
            "installed_version": if installed.is_empty() { Value::Null } else { json!(installed) },
            "latest_known_version": latest,
            "update_available": update_available,
            "manifest": manifests.get(*name).cloned().unwrap_or(Value::Null),
        }));
    }
    // Optional-profile / special components with no registry check.
    for special in ["spiderfoot", "huginn"] {
        let container = format!("intelhub-{special}");
        let line = ps.lines().find(|l| l.starts_with(&format!("{container}|")));
        items.push(json!({
            "name": special,
            "container": container,
            "state": line.map(|l| l.split('|').nth(3).unwrap_or("unknown")).unwrap_or("absent"),
            "status": line.map(|l| l.split('|').nth(2).unwrap_or("")).unwrap_or(""),
            "installed_version": line.map(|l| l.split('|').nth(1).unwrap_or("")),
            "latest_known_version": Value::Null,
            "update_available": false,
            "note": if special == "spiderfoot" { "local pinned build" } else { "digest-pinned (ghcr)" },
            "manifest": manifests.get(special).cloned().unwrap_or(Value::Null),
        }));
    }
    json!({ "count": items.len(), "items": items })
}

/// Background task: raise an info alert when an update first becomes available.
pub async fn run_update_watcher(state: AppState, ct: tokio_util::sync::CancellationToken) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        tokio::select! {
            _ = ct.cancelled() => break,
            _ = tick.tick() => {
                let listing = list_components(&state).await;
                if let Some(items) = listing.get("items").and_then(|i| i.as_array()) {
                    for it in items {
                        if it.get("update_available").and_then(|u| u.as_bool()) == Some(true) {
                            let name = it["name"].as_str().unwrap_or("?");
                            let _ = crate::alerts::raise(
                                &state,
                                crate::alerts::NewAlert {
                                    severity: "info",
                                    source: "infra",
                                    title: &format!(
                                        "Update available: {} {} → {}",
                                        name,
                                        it["installed_version"].as_str().unwrap_or("?"),
                                        it["latest_known_version"].as_str().unwrap_or("?")
                                    ),
                                    body: None,
                                    task_id: None,
                                    investigation_id: None,
                                    entity_name: None,
                                    evidence_id: None,
                                    recommended_action: Some(
                                        "run_component_action upgrade (Level 3) after reviewing changelog",
                                    ),
                                    dedupe_key: Some(&format!(
                                        "update:{name}:{}",
                                        it["latest_known_version"].as_str().unwrap_or("?")
                                    )),
                                },
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }
}

/// Level 3 controlled action (default DENY — policy.rs gates the admin token).
/// Whitelisted component × action → enumerated script invocation. No arbitrary
/// arguments can reach the shell: the command is fully constructed here.
pub async fn run_action(
    state: &AppState,
    component: &str,
    action: &str,
    actor: &str,
) -> Result<Value> {
    if !SCRIPT_COMPONENTS.contains(&component) {
        return Err(HubError::bad_request(format!(
            "component '{component}' not in lifecycle whitelist: {SCRIPT_COMPONENTS:?}"
        )));
    }
    if !ACTIONS.contains(&action) {
        return Err(HubError::bad_request(format!(
            "action '{action}' not whitelisted: {ACTIONS:?}"
        )));
    }

    // Resolve arguments: upgrade needs the target version from the registry.
    let script = format!("{}/{action}.sh", state.config.scripts_dir);
    let mut args: Vec<String> = vec![script.clone()];
    match action {
        "upgrade" => {
            let (_, repo, prefix, suffix) = COMPONENTS
                .iter()
                .find(|(n, _, _, _)| *n == component)
                .ok_or_else(|| HubError::bad_request("no registry mapping for component"))?;
            let latest = latest_tag(state, repo, prefix, suffix)
                .await
                .ok_or_else(|| HubError::internal("could not resolve latest version"))?;
            args.push(component.to_string());
            args.push(latest);
        }
        "rollback" => args.push(component.to_string()),
        _ => {} // backup takes no arguments
    }

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(900),
        tokio::process::Command::new("bash").args(&args).output(),
    )
    .await
    .map_err(|_| HubError::internal("component action timed out (900s)"))?
    .map_err(HubError::Io)?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let tail: String = stdout.chars().rev().take(2000).collect::<String>().chars().rev().collect();
    let ok = out.status.success();

    let _ = crate::store::record_audit(
        &state.pg,
        actor,
        &format!("component_{action}"),
        Some("component"),
        Some(component),
        Some("Level 3 lifecycle action via pinned script"),
        Some("run_component_action"),
        if ok { "ok" } else { "error" },
        json!({ "args": args, "exit": out.status.code(), "stderr_tail": stderr.chars().rev().take(500).collect::<String>().chars().rev().collect::<String>() }),
    )
    .await;
    let _ = crate::alerts::raise(
        state,
        crate::alerts::NewAlert {
            severity: if ok { "info" } else { "critical" },
            source: "infra",
            title: &format!(
                "Component action {action} on {component} {}",
                if ok { "succeeded" } else { "FAILED" }
            ),
            body: Some(&tail),
            task_id: None,
            investigation_id: None,
            entity_name: None,
            evidence_id: None,
            recommended_action: if ok { None } else { Some("Inspect script output and consider rollback") },
            dedupe_key: None,
        },
    )
    .await;

    if !ok {
        return Err(HubError::internal(format!(
            "script failed (exit {:?}): {}",
            out.status.code(),
            tail.chars().take(400).collect::<String>()
        )));
    }
    Ok(json!({
        "component": component, "action": action, "ok": true,
        "args": args, "output_tail": tail,
    }))
}
