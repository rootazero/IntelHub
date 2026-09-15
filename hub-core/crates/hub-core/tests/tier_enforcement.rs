//! Tier enforcement integration tests (quant-readability PR1 / Task 9).
//!
//! These tests run against a **live, deployed** hub-core. `cargo check --tests`
//! is the only expectation on a dev machine; Task 12 provisions the deployment,
//! seeds the test agents/rows, and runs them for real.
//!
//! # Endpoint choice (Ruling 15, 2026-09-15)
//!
//! The original plan targeted `/api/v1/events/recent` and asserted a free agent
//! sees zero rows. That endpoint reads the `events` BUS table, not `geo_events`:
//! its tier filter is the Rust-side `access::filter_bus_items`, keyed off
//! `payload.source` / `actor` via `config::monitor_metadata`. The actual
//! `geo_events` REST surface is `/api/v1/radar/events`
//! (`console::radar_events`), whose SQL is rewritten by
//! `access::apply_tier_filter`. This file proves **both** surfaces:
//!
//! * `geo_events` SQL rewrite  → `free_cannot_read_paid_geo_events`,
//!   `paid_reads_paid_geo_events`, `admin_reads_paid_geo_events`.
//! * `events` bus Rust filter  → `bus_filter_hides_paid_from_free`.
//! * audit trail               → `audit_log_records_tier_mismatch`.
//!
//! # Why `monitor:x`
//!
//! `apply_tier_filter` only filters rows whose `tier_required` column is above
//! the caller tier, but the audit row is written only when the *queried source*
//! resolves to a `monitor_metadata` entry above the caller tier.
//! `access::candidate_source` takes the last `:`-delimited segment, so
//! `monitor:test` resolves to `"test"` — which is not in `monitor_metadata`
//! and therefore never audits. `monitor:x` resolves to `"x"`, a real
//! `Tier::Paid` source, so it exercises filtering **and** auditing.
//!
//! # Setup (Task 12 — on the VM, once hub-core is deployed)
//!
//! ```bash
//! cd /home/zou/IntelHub
//! set -a; . core/hub.env; . core/secrets.env; set +a
//!
//! # Agents + tiers
//! for a in free paid admin; do
//!   name="tier-test-$a"
//!   grep -qE "^name:\s+$name\$" core/agent-keys.txt 2>/dev/null || {
//!     echo "=== $name ===" >> core/agent-keys.txt
//!     ./core/hub create-agent --name "$name" | grep -E '^(agent_id|name|api_key):' >> core/agent-keys.txt
//!   }
//!   ./core/hub set-tier "$name" "$a"
//! done
//! ```
//!
//! Then (keys + live hub):
//! ```bash
//! set -a; . core/hub.env; set +a          # exports DATABASE_URL
//! cd hub-core
//! TIER_TEST_HUB=http://10.10.10.45:8800 \
//! TIER_TEST_KEYS=/home/zou/IntelHub/core/agent-keys.txt \
//! DATABASE_URL="$DATABASE_URL" \
//!   cargo test --package hub-core --test tier_enforcement -- --nocapture --test-threads=1
//! ```
//!
//! # Environment
//!
//! | Var | Meaning |
//! |-----|---------|
//! | `TIER_TEST_HUB` | hub base URL (default `http://10.10.10.45:8800`) |
//! | `TIER_TEST_FREE_KEY` / `TIER_TEST_PAID_KEY` / `TIER_TEST_ADMIN_KEY` | explicit bearer keys (highest priority) |
//! | `TIER_TEST_KEYS` | keys file (default `/tmp/tier-keys.txt`), `=== <name> ===` block format with an `api_key:` line |
//! | `DATABASE_URL` | when set, tests self-seed and verify `audit_log`; when unset, external seed is assumed and the audit/bus assertions print `SKIP` |
//!
//! Rows are self-seeded when `DATABASE_URL` is present (idempotent via
//! `ON CONFLICT`), so repeated runs are safe.

use reqwest::{Client, StatusCode};
use serde_json::Value;
use sqlx::PgPool;

/// A real `Tier::Paid` monitor source (`config::monitor_metadata` key `"x"`).
/// Stored in `geo_events` with the `monitor:` prefix.
const PAID_SOURCE: &str = "monitor:x";

/// Unique title so the radar response (which does not return `external_id`)
/// can be matched unambiguously.
const GEO_TITLE: &str = "tier-enforcement paid seed (task 9)";
/// Stable `(source, external_id)` upsert key.
const GEO_EXTERNAL_ID: &str = "tier-enforcement:paid:x";
/// Stable bus event id for the `/api/v1/events/recent` surface.
const BUS_EVENT_ID: &str = "00000009-0000-4000-8000-000000000009";

fn hub_base() -> String {
    std::env::var("TIER_TEST_HUB").unwrap_or_else(|_| "http://10.10.10.45:8800".to_string())
}

/// Resolve a bearer key: explicit env var first, then the `=== <agent> ===`
/// block in `TIER_TEST_KEYS` (default `/tmp/tier-keys.txt`).
fn key_for(env_name: &str, agent_name: &str) -> String {
    if let Ok(k) = std::env::var(env_name) {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return k;
        }
    }

    let path = std::env::var("TIER_TEST_KEYS").unwrap_or_else(|_| "/tmp/tier-keys.txt".to_string());
    let contents = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("key for '{agent_name}': set {env_name}, or fix TIER_TEST_KEYS file {path}: {e}")
    });

    let mut in_block = false;
    for line in contents.lines() {
        let t = line.trim();
        if t.starts_with("===") && t.ends_with("===") {
            in_block = t.trim_matches('=').trim() == agent_name;
            continue;
        }
        if in_block {
            if let Some(k) = t.strip_prefix("api_key:") {
                let k = k.trim();
                if !k.is_empty() {
                    return k.to_string();
                }
            }
        }
    }

    panic!("key for agent '{agent_name}' not found in {path} — set {env_name} or seed that block");
}

/// Connect to PG when `DATABASE_URL` is available. `None` means the caller
/// must rely on externally-seeded rows (Task 12 seeds from the VM shell).
async fn pg_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    Some(PgPool::connect(&url).await.expect("DATABASE_URL connect"))
}

/// Idempotent seed of the paid-tier `geo_events` row.
async fn seed_geo(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO geo_events
             (source, external_id, kind, title, lat, lon, severity, occurred_at, payload, tier_required)
         VALUES ($1, $2, 'news', $3, 51.5, -0.1, 'routine', now(), '{}'::jsonb, 'paid')
         ON CONFLICT (source, external_id)
         DO UPDATE SET tier_required = 'paid', occurred_at = now(), title = EXCLUDED.title",
    )
    .bind(PAID_SOURCE)
    .bind(GEO_EXTERNAL_ID)
    .bind(GEO_TITLE)
    .execute(pool)
    .await
    .expect("seed geo_events paid row");
}

/// Idempotent seed of a paid-source row on the `events` bus table.
async fn seed_bus(pool: &PgPool) -> uuid::Uuid {
    let id = uuid::Uuid::parse_str(BUS_EVENT_ID).expect("BUS_EVENT_ID is a valid uuid");
    sqlx::query(
        "INSERT INTO events (event_id, event_type, ts, actor, payload)
         VALUES ($1, 'monitor_event', now(), 'hub:monitor:x', '{\"source\":\"monitor:x\"}'::jsonb)
         ON CONFLICT (event_id)
         DO UPDATE SET ts = now(), payload = EXCLUDED.payload",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed events bus row");
    id
}

/// GET `path_and_query` with bearer auth and require HTTP 200 + a JSON body.
async fn get_json(hub: &str, client: &Client, path_and_query: &str, key: &str) -> Value {
    let url = format!("{hub}{path_and_query}");
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {url}: {e}"));
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .unwrap_or_else(|e| panic!("GET {url}: expected JSON, got {status}: {e}"));
    assert_eq!(status, StatusCode::OK, "GET {url}: expected 200, got {status}: {body}");
    body
}

/// `items` array of a `{count, items}` response.
fn items(body: &Value) -> &[Value] {
    body.get("items")
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_else(|| panic!("response has no `items` array: {body}"))
}

/// Does the radar result set contain the seeded paid `geo_events` row?
fn has_geo_seed(rows: &[Value]) -> bool {
    rows.iter()
        .any(|it| it.get("title").and_then(|t| t.as_str()) == Some(GEO_TITLE))
}

/// Does the bus result set contain the seeded paid bus row?
fn has_bus_seed(rows: &[Value], seeded: &uuid::Uuid) -> bool {
    let id = seeded.to_string();
    rows.iter()
        .any(|it| it.get("event_id").and_then(|v| v.as_str()) == Some(id.as_str()))
}

/// Free tier must NOT retrieve a paid-source row from the `geo_events` REST
/// surface. Per design the handler returns 200 + an empty (for that source)
/// list, not 403 — a 403 would leak source existence.
#[tokio::test]
async fn free_cannot_read_paid_geo_events() {
    let hub = hub_base();
    let client = Client::new();
    if let Some(pool) = pg_pool().await {
        seed_geo(&pool).await;
    }
    let key = key_for("TIER_TEST_FREE_KEY", "tier-test-free");

    let body = get_json(
        &hub,
        &client,
        &format!("/api/v1/radar/events?source={PAID_SOURCE}&limit=200"),
        &key,
    )
    .await;
    let rows = items(&body);
    assert_eq!(
        body.get("count").and_then(Value::as_u64),
        Some(rows.len() as u64),
        "radar `count` must equal `items.len()`: {body}"
    );
    assert!(
        !has_geo_seed(rows),
        "free agent must NOT see paid-tier geo_events row (title {GEO_TITLE:?}): {body}"
    );
}

/// Paid tier must retrieve paid-source rows.
#[tokio::test]
async fn paid_reads_paid_geo_events() {
    let hub = hub_base();
    let client = Client::new();
    if let Some(pool) = pg_pool().await {
        seed_geo(&pool).await;
    }
    let key = key_for("TIER_TEST_PAID_KEY", "tier-test-paid");

    let body = get_json(
        &hub,
        &client,
        &format!("/api/v1/radar/events?source={PAID_SOURCE}&limit=200"),
        &key,
    )
    .await;
    let rows = items(&body);
    assert!(
        has_geo_seed(rows),
        "paid agent must see the paid-tier geo_events row (title {GEO_TITLE:?}): {body}"
    );
}

/// Admin tier must retrieve paid-source rows (superset of free+paid).
#[tokio::test]
async fn admin_reads_paid_geo_events() {
    let hub = hub_base();
    let client = Client::new();
    if let Some(pool) = pg_pool().await {
        seed_geo(&pool).await;
    }
    let key = key_for("TIER_TEST_ADMIN_KEY", "tier-test-admin");

    let body = get_json(
        &hub,
        &client,
        &format!("/api/v1/radar/events?source={PAID_SOURCE}&limit=200"),
        &key,
    )
    .await;
    let rows = items(&body);
    assert!(
        has_geo_seed(rows),
        "admin agent must see the paid-tier geo_events row (title {GEO_TITLE:?}): {body}"
    );
}

/// A free-tier read of a paid source must leave an `audit_log` breadcrumb
/// (`action='tier_mismatch_query'`, `blocked=true`). Requires direct PG access.
#[tokio::test]
async fn audit_log_records_tier_mismatch() {
    let Some(pool) = pg_pool().await else {
        eprintln!("SKIP audit_log_records_tier_mismatch: DATABASE_URL unset (no direct PG access)");
        return;
    };
    let hub = hub_base();
    let client = Client::new();
    let key = key_for("TIER_TEST_FREE_KEY", "tier-test-free");
    let prefix: String = key.chars().take(12).collect();

    const AUDIT_SQL: &str = "SELECT count(*) FROM audit_log \
         WHERE api_key_prefix = $1 AND action = 'tier_mismatch_query' \
           AND source_attempted = $2 AND request_path = '/api/v1/radar/events' \
           AND blocked = true";

    let before: i64 = sqlx::query_scalar(AUDIT_SQL)
        .bind(&prefix)
        .bind(PAID_SOURCE)
        .fetch_one(&pool)
        .await
        .expect("audit_log baseline query");

    let _ = get_json(
        &hub,
        &client,
        &format!("/api/v1/radar/events?source={PAID_SOURCE}"),
        &key,
    )
    .await;

    // `audit_on_reject` is awaited before the response is written, so the row
    // is normally already committed; poll briefly anyway to absorb any lag.
    let mut after = before;
    for _ in 0..30 {
        after = sqlx::query_scalar(AUDIT_SQL)
            .bind(&prefix)
            .bind(PAID_SOURCE)
            .fetch_one(&pool)
            .await
            .expect("audit_log poll query");
        if after > before {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        after > before,
        "expected >=1 audit_log tier_mismatch_query row for key prefix {prefix} on {PAID_SOURCE} \
         (before={before}, after={after})"
    );
}

/// Secondary surface: `/api/v1/events/recent` reads the `events` BUS table and
/// applies the Rust-side `filter_bus_items`. The same paid source must be
/// hidden from a free caller and visible to a paid caller. Requires PG to seed
/// the bus row.
#[tokio::test]
async fn bus_filter_hides_paid_from_free() {
    let Some(pool) = pg_pool().await else {
        eprintln!("SKIP bus_filter_hides_paid_from_free: DATABASE_URL unset (cannot seed events bus)");
        return;
    };
    let hub = hub_base();
    let client = Client::new();
    let free = key_for("TIER_TEST_FREE_KEY", "tier-test-free");
    let paid = key_for("TIER_TEST_PAID_KEY", "tier-test-paid");

    let seeded = seed_bus(&pool).await;

    let free_body = get_json(&hub, &client, "/api/v1/events/recent?limit=200", &free).await;
    let free_rows = items(&free_body);
    assert_eq!(
        free_body.get("count").and_then(Value::as_u64),
        Some(free_rows.len() as u64),
        "bus `count` must be recomputed after filtering: {free_body}"
    );
    assert!(
        !has_bus_seed(free_rows, &seeded),
        "free agent must NOT see paid bus event {seeded}: {free_body}"
    );

    let paid_body = get_json(&hub, &client, "/api/v1/events/recent?limit=200", &paid).await;
    assert!(
        has_bus_seed(items(&paid_body), &seeded),
        "paid agent must see paid bus event {seeded}: {paid_body}"
    );
}
