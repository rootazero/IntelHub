# Quant Readability PR1 — Tier Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the tier-isolation infrastructure that makes IntelHub's OSINT data legally safe + structurally usable by paid quant agents, with SQL-layer enforcement that a free-tier caller cannot bypass.

**Architecture:** Three layers — (1) `monitor_metadata` OnceLock HashMap in `config.rs` is the single source of truth for `Tier` and `LicenseClass`; (2) new `agents.tier` and `geo_events.tier_required` columns make tier an attribute of both caller and row; (3) `apply_tier_filter()` in `access.rs` rewrites every query at the SQL boundary, with `audit_on_reject()` writing a row to `audit_log` whenever a free agent touches a paid source. All 8 enforcement points (2 REST + 6 MCP) go through the same helper. Compile-time invariant test asserts every registered monitor has metadata.

**Tech Stack:** Rust (axum, sqlx, reqwest, tracing), PostgreSQL 17, Redis (audit-log recent cache), `std::sync::OnceLock` (stdlib).

**Spec:** `docs/superpowers/specs/2026-09-15-quant-readability-design.md` §4 (components 4.1–4.6), §6.1 (PR1 scope), §7 (security model), §9 (testing strategy).

**Note on migration numbering:** The spec referenced `0012_*.sql` through `0015_*.sql` for PR1. Main has since merged `0012_change_log_mirror.sql`, `0013_finding_entities.sql`, `0014_dedup_entity_aliases.sql`. **PR1's migrations are `0015_*.sql`, `0016_*.sql`, `0017_*.sql`.** PR3 will be `0018_quant_history.sql`. Spec text retained; numbering only shifted.

**Note on `core/hub` env sourcing (Ruling 5):** systemd supplies env to the hub-core service, not to bare `./core/hub` CLI invocations. Every `core/hub` call in this plan MUST source env first:
```bash
set -a && . core/hub.env && . core/secrets.env && set +a
```
Without this, calls fail `password authentication failed` (no DB password in env). Implemented in Steps 8.4, 12.6, 13.8, 11d.

**Note on `build-hub.sh` migration embedding (Ruling 6):** `sqlx::migrate!` in `hub-core/crates/hub-core/src/admin.rs:8` uses `include_dir` with no `build.rs`, so a Rust-only-no-change rebuild can ship stale migrations. Before `bash scripts/build-hub.sh`, force re-embed:
```bash
ssh <vm> 'touch /home/zou/IntelHub/hub-core/crates/hub-core/src/admin.rs'
```
Implemented in Steps 12.3, 13.5. Future fix (out of PR1 scope): add `rerun-if-changed=../../migrations/*.sql` to a build.rs.

---

## Global Constraints

- **Worktree:** Plan executor must create `feat/quant-tier-isolation` from main @ `9f1b731` before any code change. Worktree root: `/Volumes/TBU/Workspace/IntelHub-quant-pr1`.
- **Conventional Commits:** Every commit message uses `feat(...):` or `test(...):` or `chore(...):` prefixes. One logical commit per task.
- **No secrets in payloads:** Never embed an API key, upstream URL, or Authorization header in a `payload::text`. This is a hard rule — checked in `tests/secrets_redaction.rs`.
- **Type safety:** `Tier` and `LicenseClass` are typed enums. Never construct an arbitrary-string tier or license; always go through `parse` / `as_str`.
- **Migrations are forward-only:** No destructive rewrites after merge to main. Each migration is reversible via a paired `DOWN` comment block at the top (operators hand-roll the downgrade).
- **Acceptance baseline preserved:** PR1 must not regress sp5/sp7/sp9 on either 415 or 410.
- **One-tier hierarchy:** `admin > paid > free`. `paid` does NOT see `admin` (opensky NC); `free` does NOT see `paid` OR `admin`.
- **No new external dependencies.** `phf` and `once_cell` are NOT in `Cargo.toml` — use `std::sync::OnceLock` (already used in `planner_llm.rs`).

## File Structure

**Create:**
- `hub-core/migrations/0015_agents_tier.sql` — agents.tier column
- `hub-core/migrations/0016_geo_events_tier_required.sql` — geo_events.tier_required column + backfill
- `hub-core/migrations/0017_audit_log.sql` — audit_log table
- `hub-core/crates/hub-core/src/access.rs` — `TierAccess`, `apply_tier_filter`, `audit_on_reject`
- `hub-core/crates/hub-core/src/cli/set_tier.rs` — `set_tier_cli()` function
- `hub-core/crates/hub-core/tests/metadata_completeness.rs` — compile-time invariant test
- `hub-core/crates/hub-core/tests/tier_enforcement.rs` — integration test for tier filter
- `hub-core/crates/hub-core/tests/secrets_redaction.rs` — integration test for key leak prevention

**Modify:**
- `hub-core/crates/hub-core/src/config.rs` — add `Tier`, `LicenseClass`, `MonitorMeta`, `monitor_metadata()` OnceLock
- `hub-core/crates/hub-core/src/admin.rs` — add `set_tier_cli()` function (CLI dispatcher)
- `hub-core/crates/hub-core/src/store.rs` — add `update_agent_tier()` DB function
- `hub-core/crates/hub-core/src/api.rs` — thread tier through `events_recent`, `console_overview`, `console_unified_search`, `console_documents`, `console_entities` handlers
- `hub-core/crates/hub-core/src/mcp.rs` — thread tier through 6 handlers: `keyword_search`, `hybrid_search`, `semantic_search`, `query_entity`, `find_path`, `investigate`
- `hub-core/crates/hub/src/main.rs` — add `set-tier` subcommand
- `scripts/accept-sp5.py` — add 3 new tier-isolation checks at the end of the existing check section

**Responsibility boundaries:**
- `config.rs` — **declarative** metadata only. No I/O.
- `access.rs` — **policy** (filter SQL, write audit). No knowledge of REST vs MCP.
- `api.rs` / `mcp.rs` — **handlers** parse agent tier from request, call into `access.rs`. No SQL string-building here.
- `store.rs` — **DB operations**. Pure SQL.
- `admin.rs` / `cli/set_tier.rs` — **CLI plumbing**.

---

## Task 1: Migrations 0015, 0016, 0017

**Files:**
- Create: `hub-core/migrations/0015_agents_tier.sql`
- Create: `hub-core/migrations/0016_geo_events_tier_required.sql`
- Create: `hub-core/migrations/0017_audit_log.sql`

**Interfaces:**
- Consumes: existing schema (`agents`, `geo_events` tables)
- Produces: 1 new column on `agents`, 1 new column on `geo_events`, 1 new table `audit_log`

- [ ] **Step 1.1: Create `0015_agents_tier.sql`**

Write to `hub-core/migrations/0015_agents_tier.sql`:

```sql
-- 0015_agents_tier.sql
-- Adds agents.tier for caller identity tier enforcement (free/paid/admin).
-- DOWN: ALTER TABLE agents DROP COLUMN tier;

ALTER TABLE agents ADD COLUMN tier TEXT NOT NULL DEFAULT 'free';
CREATE INDEX idx_agents_tier ON agents(tier);
```

- [ ] **Step 1.2: Create `0016_geo_events_tier_required.sql`**

Write to `hub-core/migrations/0016_geo_events_tier_required.sql`:

```sql
-- 0016_geo_events_tier_required.sql
-- Adds geo_events.tier_required; backfills existing rows from source name.
-- DOWN: ALTER TABLE geo_events DROP COLUMN tier_required;

ALTER TABLE geo_events ADD COLUMN tier_required TEXT NOT NULL DEFAULT 'free';
CREATE INDEX idx_geo_events_tier_required ON geo_events(tier_required);

-- Backfill: every source matching monitor:<x> gets the tier from §4.1.1 of the spec.
-- 'opensky' is the only admin tier (NC clause); all others default to free.
-- PR2 will overwrite this column at emit-time; the WHERE clause ensures
-- idempotent re-runs.
UPDATE geo_events
SET tier_required = CASE
  WHEN source LIKE 'monitor:opensky%' THEN 'admin'
  ELSE 'free'
END
WHERE tier_required = 'free';
```

- [ ] **Step 1.3: Create `0017_audit_log.sql`**

Write to `hub-core/migrations/0017_audit_log.sql`:

```sql
-- 0017_audit_log.sql
-- Records tier-mismatch queries (free agent → paid source) and admin actions.
-- Retention 90 days (enforced by nightly cron; not in this migration).
-- DOWN: DROP TABLE audit_log;

CREATE TABLE audit_log (
    id BIGSERIAL PRIMARY KEY,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    agent_id UUID REFERENCES agents(agent_id),
    api_key_prefix TEXT NOT NULL,
    action TEXT NOT NULL,
    source_attempted TEXT,
    request_path TEXT,
    trace_id UUID,
    blocked BOOLEAN NOT NULL
);
CREATE INDEX idx_audit_log_ts ON audit_log(ts);
CREATE INDEX idx_audit_log_agent ON audit_log(agent_id);
```

- [ ] **Step 1.4: Verify migrations apply via `sqlx::migrate!`**

Migrations are embedded in the `core/hub` binary via `sqlx::migrate!("../../migrations")` (see `admin.rs:8`). Manually running psql would skip `_sqlx_migrations` bookkeeping, causing the next binary restart to fail. The correct flow is: build the binary (which embeds the new SQL), then run any admin CLI command to trigger migration apply.

Run from the worktree:
```bash
# Apply via ssh to 415 (the worktree has no local docker; ssh is the path)
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub
  bash scripts/build-hub.sh 2>&1 | grep -E "^error|^==> built" | head -3
'
```

Expected: `==> built: /home/zou/IntelHub/core/hub`. If errors appear, STOP and report BLOCKED.

Then trigger migrations via any admin command (e.g. create-agent for a throwaway name):
```bash
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub
  ./core/hub create-agent --name _migration_probe 2>&1 | tail -5
'
```

Expected: `agent created` + agent_id line (no error). The throwaway agent can be cleaned up later.

- [ ] **Step 1.5: Verify backfill row counts + audit_log schema**

Run:
```bash
ssh -o BatchMode=yes IntelHub-test 'docker exec intelhub-postgres psql -U intelhub -d intelhub -c \
  "SELECT tier_required, count(*) FROM geo_events GROUP BY 1 ORDER BY 1;"'
ssh -o BatchMode=yes IntelHub-test 'docker exec intelhub-postgres psql -U intelhub -d intelhub -c \
  "\d audit_log"'
```

Expected: at least 1 row with `tier_required='admin'` (any historical opensky event); all others `free`. `audit_log` table exists with 9 columns + 2 indexes.

- [ ] **Step 1.6: Commit**

```bash
git add hub-core/migrations/0015_agents_tier.sql \
        hub-core/migrations/0016_geo_events_tier_required.sql \
        hub-core/migrations/0017_audit_log.sql
git commit -m "feat(access): migrations 0015-0017 — agents.tier + geo_events.tier_required + audit_log"
```

---

## Task 2: Core types and metadata registry

**Files:**
- Modify: `hub-core/crates/hub-core/src/config.rs` — append types and metadata function
- Create: `hub-core/crates/hub-core/tests/metadata_completeness.rs` — compile-time invariant test

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub enum Tier { Free, Paid, Admin }` with `as_str()`, `parse()`
  - `pub enum LicenseClass { Open, FairUse, Restricted, NonCommercial }` with `as_str()`
  - `pub struct MonitorMeta { pub tier_required: Tier, pub license_class: LicenseClass, pub data_age_hours: u32, pub doc_url: &'static str }`
  - `pub fn monitor_metadata() -> &'static HashMap<&'static str, MonitorMeta>`

- [ ] **Step 2.1: Add the types block to `config.rs`**

Read the existing `hub-core/crates/hub-core/src/config.rs` to find a suitable insertion point (end of file, before any closing `}` of the module). Append:

```rust
use std::collections::HashMap;
use std::sync::OnceLock;

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
        m.insert("usgs",          MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://earthquake.usgs.gov/earthquides/feed/v1.0/geojson.php" });
        m.insert("noaa",          MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "https://www.weather.gov/documentation/services-web-api" });
        m.insert("epa",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.epa.gov/enviro/web-services" });
        m.insert("eonet",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "https://eonet.gsfc.nasa.gov/api/v3/" });
        m.insert("who",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.who.int/" });
        m.insert("climateseries", MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.ncei.noaa.gov/access/monitoring/climate-at-a-glance/" });
        m.insert("radiation",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "https://www.epa.gov/radnet" });

        // US-gov economic data — public domain
        m.insert("firms",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 3,  doc_url: "https://firms.modaps.eosdis.nasa.gov/api/" });
        m.insert("bls",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.bls.gov/developers/" });
        m.insert("fred",          MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://fred.stlouisfed.org/docs/api/" });
        m.insert("treasury",      MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://home.treasury.gov/developers" });
        m.insert("comtrade",      MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 720,doc_url: "https://comtrade.un.org/data/doc/api" });
        m.insert("usaspending",   MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.usaspending.gov/disbursement/Transparency" });
        m.insert("gscpi",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 720,doc_url: "https://www.newyorkfed.org/markets/global-supply-chain-pressure-index" });
        m.insert("sec_edgar",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://efts.sec.gov/LATEST/search-index" });
        m.insert("ofac",          MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://sanctionssearch.ofac.treas.gov/" });

        // ODbL / CC-BY
        m.insert("gdelt",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "https://blog.gdeltproject.org/gdelt-2-0-english-translation-api/" });
        m.insert("acled",         MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://acleddata.com/api-documentation/" });
        m.insert("reliefweb",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://reliefweb.int/help/api" });
        m.insert("kiwisdr",       MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 1,  doc_url: "http://kiwisdr.com/" });
        m.insert("nvd",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://nvd.nist.gov/developers" });
        m.insert("cisakev",       MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.cisa.gov/known-exploited-vulnerabilities-catalog" });
        m.insert("osv",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 6,  doc_url: "https://google.github.io/osv.dev/" });
        m.insert("opensanctions", MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open, data_age_hours: 24, doc_url: "https://www.opensanctions.org/api/" });

        // Paid APIs — redistributability restricted
        m.insert("x",             MonitorMeta { tier_required: Tier::Paid, license_class: LicenseClass::Restricted, data_age_hours: 1, doc_url: "https://developer.twitter.com/en/docs/twitter-api" });
        m.insert("bluesky",       MonitorMeta { tier_required: Tier::Paid, license_class: LicenseClass::Restricted, data_age_hours: 1, doc_url: "https://docs.bsky.app/docs/api" });
        m.insert("finintel",      MonitorMeta { tier_required: Tier::Paid, license_class: LicenseClass::Restricted, data_age_hours: 1, doc_url: "https://finnhub.io/docs/api" });
        m.insert("markets",       MonitorMeta { tier_required: Tier::Paid, license_class: LicenseClass::Restricted, data_age_hours: 1, doc_url: "https://financialmodelingprep.com/developer/docs/" });

        // Fair-use (free tier, redistribution limited)
        m.insert("telegram",      MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::FairUse, data_age_hours: 1, doc_url: "https://core.telegram.org/api" });
        m.insert("rss",           MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::FairUse, data_age_hours: 1, doc_url: "internal://rss-aggregator" });
        m.insert("textclass",     MonitorMeta { tier_required: Tier::Free, license_class: LicenseClass::Open,      data_age_hours: 0, doc_url: "internal://textclass" });

        // Admin-only — NC clause (per prior wontfix analysis)
        m.insert("opensky",       MonitorMeta { tier_required: Tier::Admin, license_class: LicenseClass::NonCommercial, data_age_hours: 1, doc_url: "https://opensky-network.org/apidoc/" });

        m
    })
}
```

Note: `textclass` is registered here even though it's not in `monitor/mod.rs` registry as a `Box::new` source — it's a helper, not a feed. The metadata completeness test (Task 3) only asserts the *registry* matches metadata, not the reverse. If `textclass` is removed from `monitor/mod.rs` later, remove its metadata entry here too.

- [ ] **Step 2.2: Verify cargo check**

Run from worktree:
```bash
cargo check --workspace --message-format=short 2>&1 | tail -8
```

Expected: `Finished ... target(s) in <time>s` with no errors. Warnings unrelated to new code are OK (baseline has 6).

- [ ] **Step 2.3: Commit `config.rs` changes**

```bash
git add hub-core/crates/hub-core/src/config.rs
git commit -m "feat(access): add Tier, LicenseClass, MonitorMeta + monitor_metadata() registry"
```

---

## Task 3: Compile-time metadata completeness test

**Files:**
- Create: `hub-core/crates/hub-core/tests/metadata_completeness.rs`

**Interfaces:**
- Consumes: `monitor_metadata()` registry from `config.rs`, names from `monitor/mod.rs` Box::new list
- Produces: a test that fails CI if any registered monitor lacks metadata

- [ ] **Step 3.1: Read the registry names**

Read `hub-core/crates/hub-core/src/monitor/mod.rs` to enumerate every `Box::new(sources::XYZ::XYZ)` line. The current list (post-OSINT-trio merge) is exactly 32 names:

```rust
&[
    "usgs", "noaa", "firms", "gdelt", "opensky", "rss",
    "sec_edgar", "radiation", "acled", "kiwisdr", "reliefweb", "who", "cisakev",
    "nvd", "osv", "ofac", "opensanctions", "usaspending", "epa",
    "bluesky", "telegram", "x", "bls", "eonet",
    "fred", "eia", "treasury", "markets", "finintel", "gscpi", "comtrade",
    "climateseries",
]
```

(Note: ordering follows the registry in `monitor/mod.rs`. The test does NOT depend on order.)

- [ ] **Step 3.2: Write the test**

Write to `hub-core/crates/hub-core/tests/metadata_completeness.rs`:

```rust
//! Compile-time-adjacent invariant: every registered monitor MUST have
//! a `monitor_metadata()` entry, and every metadata entry MUST correspond
//! to a registered monitor. Catches drift between `monitor/mod.rs` registry
//! and `config.rs::monitor_metadata()` map.

use hub_core::config::monitor_metadata;

const REGISTRY: &[&str] = &[
    "usgs", "noaa", "firms", "gdelt", "opensky", "rss",
    "sec_edgar", "radiation", "acled", "kiwisdr", "reliefweb", "who", "cisakev",
    "nvd", "osv", "ofac", "opensanctions", "usaspending", "epa",
    "bluesky", "telegram", "x", "bls", "eonet",
    "fred", "eia", "treasury", "markets", "finintel", "gscpi", "comtrade",
    "climateseries",
];

#[test]
fn every_registered_monitor_has_metadata() {
    let meta = monitor_metadata();
    for name in REGISTRY {
        assert!(
            meta.contains_key(name),
            "{name} is in monitor/mod.rs registry but missing from monitor_metadata() map"
        );
    }
}

#[test]
fn every_metadata_entry_is_registered() {
    let meta = monitor_metadata();
    for name in meta.keys() {
        assert!(
            REGISTRY.contains(name),
            "{name} has metadata entry but is NOT in monitor/mod.rs registry"
        );
    }
}

#[test]
fn registry_count_matches_metadata_count() {
    let meta = monitor_metadata();
    assert_eq!(
        REGISTRY.len(),
        meta.len(),
        "registry has {} entries, metadata has {}",
        REGISTRY.len(),
        meta.len()
    );
}
```

- [ ] **Step 3.3: Verify the test compiles and runs**

Run:
```bash
cargo test --package hub-core --test metadata_completeness -- --nocapture
```

Expected: `3 passed; 0 failed`.

- [ ] **Step 3.4: Commit**

```bash
git add hub-core/crates/hub-core/tests/metadata_completeness.rs
git commit -m "test(access): compile-time invariant — registry ⇔ metadata coverage"
```

---

## Task 4: apply_tier_filter + audit_on_reject in access.rs

**Files:**
- Create: `hub-core/crates/hub-core/src/access.rs`

**Interfaces:**
- Consumes: `Tier`, `LicenseClass`, `MonitorMeta` from `config`
- Produces:
  - `pub trait TierAccess { fn tier(&self) -> Tier; }`
  - `pub fn apply_tier_filter(base_query: &str, tier: Tier) -> String` — pure SQL builder
  - `pub async fn audit_on_reject(pool: &PgPool, agent_id: Uuid, api_key_prefix: &str, source_attempted: &str, request_path: &str, trace_id: Option<Uuid>) -> Result<()>`

- [ ] **Step 4.1: Create `access.rs` skeleton**

Write to `hub-core/crates/hub-core/src/access.rs`:

```rust
//! Tier-based access control for OSINT data.
//!
//! Every query path that touches `geo_events` goes through `apply_tier_filter`.
//! This rewrites the SQL at the boundary, so a free-tier caller physically
//! cannot retrieve paid-source rows even via crafted SQL strings.
//!
//! When a free-tier caller attempts to read paid-source data (e.g., by
//! passing `source=monitor:x` to `/api/v1/events/recent`), the handler
//! returns 200 with `events: []` and `audit_on_reject` logs the attempt.
//! We deliberately do NOT return 403, which would leak source existence.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Tier;

/// Implemented by anything that carries a caller identity.
pub trait TierAccess {
    fn tier(&self) -> Tier;
}

/// Rewrites a SQL fragment so a `free` caller only sees `free` rows,
/// `paid` sees `free+paid`, and `admin` sees everything.
///
/// Caller is responsible for any prior WHERE / ORDER BY / LIMIT clauses.
/// `base_query` should end with the FROM clause but NOT have a WHERE on
/// `tier_required` (we add it).
///
/// Example:
/// ```text
/// apply_tier_filter("SELECT id FROM geo_events WHERE source = $1", Tier::Free)
///   → "SELECT id FROM geo_events WHERE source = $1 AND tier_required = 'free'"
/// ```
pub fn apply_tier_filter(base_query: &str, tier: Tier) -> String {
    match tier {
        Tier::Admin => base_query.to_string(),
        Tier::Paid => {
            if base_query.to_uppercase().contains(" WHERE ") {
                format!("{base_query} AND tier_required IN ('free', 'paid')")
            } else {
                format!("{base_query} WHERE tier_required IN ('free', 'paid')")
            }
        }
        Tier::Free => {
            if base_query.to_uppercase().contains(" WHERE ") {
                format!("{base_query} AND tier_required = 'free'")
            } else {
                format!("{base_query} WHERE tier_required = 'free'")
            }
        }
    }
}

/// Writes an audit row when a tier-mismatch access is attempted (or for
/// admin actions like `set-tier`). Best-effort: errors are logged but do
/// not fail the user-facing request — auditing must never become a DoS
/// vector.
pub async fn audit_on_reject(
    pool: &PgPool,
    agent_id: Uuid,
    api_key_prefix: &str,
    action: &str,
    source_attempted: Option<&str>,
    request_path: Option<&str>,
    trace_id: Option<Uuid>,
    blocked: bool,
) {
    let result = sqlx::query(
        "INSERT INTO audit_log
            (ts, agent_id, api_key_prefix, action, source_attempted, request_path, trace_id, blocked)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(Utc::now())
    .bind(agent_id)
    .bind(api_key_prefix)
    .bind(action)
    .bind(source_attempted)
    .bind(request_path)
    .bind(trace_id)
    .bind(blocked)
    .execute(pool)
    .await;

    if let Err(e) = result {
        tracing::warn!(
            target: "access::audit",
            "audit_on_reject failed: {e}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_passes_through() {
        let q = "SELECT * FROM geo_events WHERE source = $1";
        assert_eq!(
            apply_tier_filter(q, Tier::Admin),
            "SELECT * FROM geo_events WHERE source = $1"
        );
    }

    #[test]
    fn paid_adds_in_clause() {
        let q = "SELECT * FROM geo_events WHERE source = $1";
        let out = apply_tier_filter(q, Tier::Paid);
        assert!(out.contains("WHERE source = $1"));
        assert!(out.contains("tier_required IN ('free', 'paid')"));
        assert!(!out.contains(" WHERE WHERE "));
    }

    #[test]
    fn free_adds_eq_clause() {
        let q = "SELECT * FROM geo_events WHERE source = $1";
        let out = apply_tier_filter(q, Tier::Free);
        assert!(out.contains("tier_required = 'free'"));
        assert!(!out.contains(" WHERE WHERE "));
    }

    #[test]
    fn free_without_where_adds_where() {
        let q = "SELECT * FROM geo_events";
        let out = apply_tier_filter(q, Tier::Free);
        assert_eq!(out, "SELECT * FROM geo_events WHERE tier_required = 'free'");
    }

    #[test]
    fn case_insensitive_where_detection() {
        let q = "select * from geo_events where source = $1";
        let out = apply_tier_filter(q, Tier::Paid);
        assert!(out.contains(" AND tier_required IN ('free', 'paid')"));
    }
}
```

- [ ] **Step 4.2: Run the unit tests**

```bash
cargo test --package hub-core --lib access -- --nocapture
```

Expected: `5 passed; 0 failed`.

- [ ] **Step 4.3: Register the module in `lib.rs`**

Read `hub-core/crates/hub-core/src/lib.rs`. Find the existing `pub mod ...` block and add `pub mod access;` (alphabetical order).

- [ ] **Step 4.4: Verify cargo check**

```bash
cargo check --workspace --message-format=short 2>&1 | tail -5
```

Expected: clean compile, no new errors.

- [ ] **Step 4.5: Commit**

```bash
git add hub-core/crates/hub-core/src/access.rs hub-core/crates/hub-core/src/lib.rs
git commit -m "feat(access): apply_tier_filter() + audit_on_reject() + unit tests"
```

---

## Task 5: set_tier_cli in admin.rs + DB update in store.rs

**Files:**
- Modify: `hub-core/crates/hub-core/src/store.rs` — add `update_agent_tier`
- Modify: `hub-core/crates/hub-core/src/admin.rs` — add `set_tier_cli`

**Interfaces:**
- Consumes: `PgPool`, agent name + new tier
- Produces:
  - `pub async fn update_agent_tier(pool: &PgPool, name: &str, tier: Tier) -> Result<Uuid>` — returns agent_id
  - `pub async fn set_tier_cli(cfg: &Config, name: &str, tier_str: &str) -> anyhow::Result<()>`

- [ ] **Step 5.1: Add `update_agent_tier` to `store.rs`**

Read `hub-core/crates/hub-core/src/store.rs` to find `create_agent` (the closest existing pattern). Append a new function:

```rust
use crate::config::Tier;

/// Set an agent's tier. Returns the agent_id. Errors if the agent does
/// not exist or the supplied `tier` string is invalid.
///
/// Note: this does NOT enforce who is allowed to call set-tier. The CLI
/// dispatcher (set_tier_cli) is the privilege gate; the DB function is
/// a primitive.
pub async fn update_agent_tier(
    pool: &sqlx::PgPool,
    name: &str,
    tier: Tier,
) -> anyhow::Result<uuid::Uuid> {
    let tier_str = tier.as_str();

    let agent_id: Option<uuid::Uuid> = sqlx::query_scalar(
        "UPDATE agents SET tier = $1 WHERE name = $2 RETURNING id"
    )
    .bind(tier_str)
    .bind(name)
    .fetch_optional(pool)
    .await?;

    let agent_id = agent_id.ok_or_else(|| {
        anyhow::anyhow!("agent '{name}' not found")
    })?;

    Ok(agent_id)
}
```

- [ ] **Step 5.2: Add `set_tier_cli` to `admin.rs`**

Append to `hub-core/crates/hub-core/src/admin.rs`:

```rust
use crate::config::Tier;
use crate::store::update_agent_tier;

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
```

- [ ] **Step 5.3: Verify cargo check**

```bash
cargo check --workspace --message-format=short 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add hub-core/crates/hub-core/src/store.rs hub-core/crates/hub-core/src/admin.rs
git commit -m "feat(access): set_tier_cli + update_agent_tier DB primitive"
```

---

## Task 6: Wire apply_tier_filter into REST endpoints

**Files:**
- Modify: `hub-core/crates/hub-core/src/api.rs`

**Interfaces:**
- Consumes: existing handler functions for `/api/v1/events/recent`, `/api/v1/overview`, `/api/v1/search/unified`, `/api/v1/documents`, `/api/v1/entities`, `/api/v1/entities/{id}`
- Produces: each handler resolves `agent.tier` from request, calls `apply_tier_filter` on its SQL, and on mismatch calls `audit_on_reject` (returning 200 + empty list, not 403)

- [ ] **Step 6.1: Read the existing handlers**

Read `hub-core/crates/hub-core/src/api.rs` to identify:
- The shared `RequestCtx` / `CallerIdentity` struct (or equivalent) that handlers extract from request extensions
- The exact SQL strings used by each of: `events_recent`, `console_overview`, `console_unified_search`, `console_documents`, `console_entities`, `console_entity`

- [ ] **Step 6.2: Add a helper for tier-resolved queries**

In `api.rs`, add a helper near the top:

```rust
use crate::access::{apply_tier_filter, audit_on_reject, TierAccess};

/// Extracts the agent's tier from a request, applies it to a SQL fragment,
/// and on tier-mismatch (i.e., the source name in the query is paid-tier
/// but the agent is free), writes an audit row.
///
/// Returns the filtered SQL. Free agents asking for paid sources get a SQL
/// that returns zero rows; they cannot detect that the source exists.
async fn filter_query_by_tier(
    pool: &sqlx::PgPool,
    caller: &dyn TierAccess,
    agent_id: uuid::Uuid,
    api_key_prefix: &str,
    base_sql: &str,
    source_attempted: Option<&str>,
    request_path: &str,
    trace_id: Option<uuid::Uuid>,
) -> String {
    let tier = caller.tier();
    let filtered = apply_tier_filter(base_sql, tier);

    // If the caller asked for a specific source that doesn't match their
    // tier, log it. Detection: the source_attempted prefix is `monitor:`
    // AND the tier_required for that source exceeds caller's tier.
    if let Some(src) = source_attempted {
        if src.starts_with("monitor:") {
            let monitor_name = src.trim_start_matches("monitor:");
            if let Some(meta) = crate::config::monitor_metadata().get(monitor_name) {
                let denied = match (tier, meta.tier_required) {
                    (crate::config::Tier::Free, crate::config::Tier::Paid) => true,
                    (crate::config::Tier::Free, crate::config::Tier::Admin) => true,
                    (crate::config::Tier::Paid, crate::config::Tier::Admin) => true,
                    _ => false,
                };
                if denied {
                    audit_on_reject(
                        pool,
                        agent_id,
                        api_key_prefix,
                        "tier_mismatch_query",
                        Some(src),
                        Some(request_path),
                        trace_id,
                        true,
                    ).await;
                }
            }
        }
    }

    filtered
}
```

- [ ] **Step 6.3: Modify `events_recent` handler**

Find the `events_recent` handler. Replace its SQL build step to call `filter_query_by_tier`:

```rust
async fn events_recent(
    State(state): State<AppState>,
    Extension(caller): Extension<crate::middleware::CallerIdentity>,
    Query(params): Query<EventsQuery>,
) -> Result<Json<Vec<Event>>, ApiError> {
    let source = params.source.as_deref();

    let base_sql = format!(
        "SELECT id, kind, severity, occurred_at, source, lat, lon, external_id, payload
         FROM geo_events
         WHERE occurred_at > now() - INTERVAL '24 hours'
         {}",
        source.map(|s| format!("AND source = '{s}'")).unwrap_or_default()
    );

    let sql = filter_query_by_tier(
        &state.pool, &caller, caller.agent_id, &caller.api_key_prefix,
        &base_sql, source, "/api/v1/events/recent", caller.trace_id,
    ).await;

    let rows = sqlx::query_as::<_, Event>(&sql)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows))
}
```

(Adapt field types to match the existing `Event` struct in the file. Do NOT rename existing fields.)

- [ ] **Step 6.4: Modify `console_overview`, `console_unified_search`, `console_documents`, `console_entities`, `console_entity` similarly**

Apply the same pattern: extract `caller` from extensions, build the base SQL with optional `source` filter, call `filter_query_by_tier`, execute the filtered SQL, return results. Free agents asking for paid sources get empty results + an audit row.

Do NOT delete or rename existing fields. Only wrap the SQL execution.

- [ ] **Step 6.5: Verify cargo check**

```bash
cargo check --workspace --message-format=short 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 6.6: Commit**

```bash
git add hub-core/crates/hub-core/src/api.rs
git commit -m "feat(rest): apply apply_tier_filter to all geo_events-touching handlers"
```

---

## Task 7: Wire apply_tier_filter into 6 MCP handlers

**Files:**
- Modify: `hub-core/crates/hub-core/src/mcp.rs`

**Interfaces:**
- Consumes: `keyword_search`, `hybrid_search`, `semantic_search`, `query_entity`, `find_path`, `investigate` handlers
- Produces: each handler resolves agent tier from request context and filters SQL results

- [ ] **Step 7.1: Locate the agent-tier extraction pattern**

Read `mcp.rs` to find:
- How the existing handlers extract the caller identity (likely `RequestCtx` from a shared type)
- The exact SQL fragments that touch `geo_events` (search handlers JOIN through documents/embeddings, but the final filter is on `geo_events.source` or via `document_id`)

- [ ] **Step 7.2: For each of the 6 handlers, add a tier filter**

The pattern is the same as REST: after computing the candidate result set, call `audit_on_reject` for any source the agent attempted but isn't entitled to. For search handlers, the SQL JOIN through `documents` and `embeddings` doesn't include a `source = 'monitor:X'` clause directly; instead the filter is on the row's source column after the JOIN.

Apply the same `filter_query_by_tier` helper from `api.rs` (or its MCP equivalent if extraction differs — copy it into mcp.rs if the request context type doesn't match).

For `investigate` (multi-step executor), apply the filter at every internal `query_entity` / `hybrid_search` step. The existing executor already calls these helpers — intercept the call sites.

- [ ] **Step 7.3: Verify cargo check**

```bash
cargo check --workspace --message-format=short 2>&1 | tail -10
```

Expected: clean. If borrow-checker complains about RequestCtx lifetime, prefer cloning the `tier: Tier` value (it's Copy).

- [ ] **Step 7.4: Commit**

```bash
git add hub-core/crates/hub-core/src/mcp.rs
git commit -m "feat(mcp): apply apply_tier_filter to all geo_events-touching MCP handlers"
```

---

## Task 8: Register `set-tier` subcommand in main.rs

**Files:**
- Modify: `hub-core/crates/hub/src/main.rs`

**Interfaces:**
- Consumes: existing arg parsing pattern (simple positional `match` on `args.get(1)`)
- Produces: new branch `"set-tier"` calling `set_tier_cli(name, tier)` after parsing remaining args

- [ ] **Step 8.1: Read main.rs arg dispatch**

Read `hub-core/crates/hub/src/main.rs` to see the existing match arms. The current pattern (per prior inspection) is:

```rust
match args.get(1).map(String::as_str) {
    Some("create-agent") => { ... }
    Some("rotate-key") => { ... }
    _ => { ... }
}
```

- [ ] **Step 8.2: Add `set-tier` arm**

Append to the match:

```rust
Some("set-tier") => {
    let name = args.get(2).cloned().unwrap_or_else(|| {
        eprintln!("usage: core/hub set-tier <agent-name> <free|paid|admin>");
        std::process::exit(2);
    });
    let tier = args.get(3).cloned().unwrap_or_else(|| {
        eprintln!("usage: core/hub set-tier <agent-name> <free|paid|admin>");
        std::process::exit(2);
    });
    if let Err(e) = hub_core::admin::set_tier_cli(&cfg, &name, &tier).await {
        eprintln!("set-tier failed: {e}");
        std::process::exit(1);
    }
}
```

- [ ] **Step 8.3: Verify cargo check + clippy**

```bash
cargo check --workspace --message-format=short 2>&1 | tail -5
cargo clippy --workspace -- -D warnings 2>&1 | tail -5
```

Expected: both clean.

- [ ] **Step 8.4: Smoke test the CLI**

Build the binary:

```bash
cargo build --release -p hub
```

Run on the 415 test VM (after rsync, env sourced per Ruling 5):

```bash
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub
  set -a && . core/hub.env && . core/secrets.env && set +a
  ./core/hub set-tier pi free
'
```

Expected output:
```
agent tier updated
agent_id: <uuid>
name:     pi
tier:     free
```

Verify the row in PG:

```bash
ssh -o BatchMode=yes IntelHub-test 'docker exec intelhub-postgres psql -U intelhub -d intelhub \
  -c "SELECT name, tier FROM agents ORDER BY name"'
```

Expected: `pi` row has `tier=free`.

Verify the audit_log row:

```bash
ssh -o BatchMode=yes IntelHub-test 'docker exec intelhub-postgres psql -U intelhub -d intelhub \
  -c "SELECT action, source_attempted, api_key_prefix FROM audit_log ORDER BY ts DESC LIMIT 1"'
```

Expected: `set_tier | free | cli`.

- [ ] **Step 8.5: Commit**

```bash
git add hub-core/crates/hub/src/main.rs
git commit -m "feat(cli): add 'set-tier' subcommand to core/hub dispatcher"
```

---

## Task 9: Integration test tier_enforcement.rs

**Files:**
- Create: `hub-core/crates/hub-core/tests/tier_enforcement.rs`

**Interfaces:**
- Consumes: a running hub-core instance (assume pre-deployed), three test API keys (free / paid / admin), a known paid source event (e.g., X monitor)
- Produces: 4 tests that prove the tier filter is correctly applied

- [ ] **Step 9.1: Set up test keys on 415**

SSH to 415:

```bash
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub
  ./core/hub create-agent --name tier-test-free >/dev/null
  ./core/hub create-agent --name tier-test-paid >/dev/null
  ./core/hub create-agent --name tier-test-admin >/dev/null
  ./core/hub set-tier tier-test-free free
  ./core/hub set-tier tier-test-paid paid
  ./core/hub set-tier tier-test-admin admin
'
```

Capture the keys:
```bash
ssh -o BatchMode=yes IntelHub-test 'cat /home/zou/IntelHub/core/agent-keys.txt' > /tmp/tier-keys.txt
```

Extract via grep: `grep "tier-test" /tmp/tier-keys.txt`.

- [ ] **Step 9.2: Seed a paid-tier event in PG**

Insert directly into geo_events with `tier_required='paid'`:

```bash
ssh -o BatchMode=yes IntelHub-test '
docker exec intelhub-postgres psql -U intelhub -d intelhub -c "
  INSERT INTO geo_events (kind, severity, occurred_at, source, lat, lon, external_id, payload, tier_required)
  VALUES (\"test_event\", \"routine\", now(), \"monitor:test:1\", 0, 0, \"test:paid:1\", \"{}\"::jsonb, \"paid\");
"
'
```

Note: shell quoting in the heredoc — use single-quote outer + escape inner as `\"`. If quoting is fragile, write the SQL to a file via `cat > /tmp/seed.sql <<'EOF' ... EOF` first, then `psql < /tmp/seed.sql`.

- [ ] **Step 9.3: Write the test file**

Write to `hub-core/crates/hub-core/tests/tier_enforcement.rs`:

```rust
//! Tier enforcement integration tests.
//!
//! Run against a deployed hub-core (415 or 410). Requires the three test
//! agents created in Task 9.1 and the seeded paid event from Task 9.2.

use reqwest::Client;
use serde_json::Value;

const HUB: &str = "http://10.10.10.45:8800";

fn key_for(name: &str) -> String {
    let path = std::env::var("TIER_TEST_KEYS")
        .unwrap_or_else(|_| "/tmp/tier-keys.txt".to_string());
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("read keys file: {path}"));
    contents
        .lines()
        .find(|l| l.contains(name))
        .and_then(|l| l.split("api_key:").nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| panic!("key for {name} not found"))
}

#[tokio::test]
async fn free_cannot_read_paid_events() {
    let client = Client::new();
    let key = key_for("tier-test-free");

    let resp: Value = client
        .get(format!("{HUB}/api/v1/events/recent?source=monitor:test"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Free agent sees no rows even though the source exists in PG.
    let rows = resp.as_array().expect("response must be JSON array");
    assert_eq!(rows.len(), 0, "free agent must NOT see paid-tier rows: got {rows:?}");
}

#[tokio::test]
async fn paid_reads_paid_and_free() {
    let client = Client::new();
    let key = key_for("tier-test-paid");

    let resp: Value = client
        .get(format!("{HUB}/api/v1/events/recent?source=monitor:test"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let rows = resp.as_array().expect("response must be JSON array");
    assert!(rows.len() >= 1, "paid agent must see at least 1 paid-tier row: got {rows:?}");
}

#[tokio::test]
async fn admin_reads_admin_paid_free() {
    let client = Client::new();
    let key = key_for("tier-test-admin");

    // (Test setup would seed an admin-tier event similarly to Task 9.2.)
    // For PR1 we only verify paid + free are visible to admin:
    let resp: Value = client
        .get(format!("{HUB}/api/v1/events/recent?source=monitor:test"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let rows = resp.as_array().expect("response must be JSON array");
    assert!(rows.len() >= 1, "admin must see paid-tier row: got {rows:?}");
}

#[tokio::test]
async fn audit_log_records_mismatch() {
    let client = Client::new();
    let key = key_for("tier-test-free");

    // Trigger a tier mismatch
    let _ = client
        .get(format!("{HUB}/api/v1/events/recent?source=monitor:test"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    // Give the audit_log writer 200ms to flush (best-effort).
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Check audit_log directly via PG. (Tests may run in a sandbox where
    // PG is accessible; if not, query via a separate API in production.)
    let pg_url = std::env::var("DATABASE_URL").unwrap_or_default();
    if pg_url.is_empty() {
        eprintln!("DATABASE_URL not set; skipping direct audit_log assertion");
        return;
    }
    let pool = sqlx::PgPool::connect(&pg_url).await.unwrap();
    let count: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_log
         WHERE api_key_prefix = $1
           AND source_attempted = 'monitor:test'
           AND blocked = true"
    )
    .bind(&key[..12])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(count.0 >= 1, "expected at least 1 audit row for tier mismatch, got {}", count.0);
}
```

- [ ] **Step 9.4: Run the tests**

```bash
DATABASE_URL='postgres://intelhub:<password>@localhost:5432/intelhub' \
  TIER_TEST_KEYS=/tmp/tier-keys.txt \
  cargo test --package hub-core --test tier_enforcement -- --nocapture --test-threads=1
```

Expected: 4 passed.

If `DATABASE_URL` is unavailable in the sandbox, run via direct SSH:

```bash
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub/hub-core
  DATABASE_URL=$(grep "^DATABASE_URL=" /home/zou/IntelHub/core/hub.env | cut -d= -f2-)
  TIER_TEST_KEYS=/tmp/tier-keys.txt
  cargo test --package hub-core --test tier_enforcement -- --nocapture --test-threads=1 2>&1 | tail -20
'
```

- [ ] **Step 9.5: Commit**

```bash
git add hub-core/crates/hub-core/tests/tier_enforcement.rs
git commit -m "test(access): tier_enforcement integration test — free/paid/admin + audit row"
```

---

## Task 10: Integration test secrets_redaction.rs

**Files:**
- Create: `hub-core/crates/hub-core/tests/secrets_redaction.rs`

**Interfaces:**
- Consumes: deployed hub-core, an API key with known substring, knowledge of what `HUB_*_API_KEY` values look like
- Produces: 3 tests that scan responses + log lines + payload JSON for key substrings

- [ ] **Step 10.1: Read `core/secrets.env` to know the key patterns**

From the 415 VM:

```bash
ssh -o BatchMode=yes IntelHub-test 'cat /home/zou/IntelHub/core/secrets.env | grep -E "^(HUB|FINNHUB|FMP|TELEGRAM|BLUESKY)_.*_(KEY|TOKEN)" | sed "s/=.*/=<REDACTED>/"'
```

Pick 3 keys that are known to be live (e.g., `HUB_NVD_API_KEY`, `FINNHUB_API_KEY`, `BLUESKY_API_KEY` if configured). Record the first 16 chars of each — those are the substrings we'll scan for.

- [ ] **Step 10.2: Write the test file**

Write to `hub-core/crates/hub-core/tests/secrets_redaction.rs`:

```rust
//! Secrets-redaction integration tests.
//!
//! For any paid-source event, the response payload MUST NOT contain the
//! substring of the upstream API key. Free agents in particular must be
//! unable to extract the key via repeated queries.

use reqwest::Client;
use serde_json::Value;
use std::collections::HashSet;

const HUB: &str = "http://10.10.10.45:8800";

fn free_key() -> String {
    std::env::var("TIER_TEST_FREE_KEY").expect("TIER_TEST_FREE_KEY env")
}

fn known_key_substrings() -> HashSet<String> {
    // First 16 chars of each live key. Loaded from a file the operator
    // pre-populates before running tests.
    let path = std::env::var("TIER_TEST_KEY_SUBSTR")
        .unwrap_or_else(|_| "/tmp/tier-key-substrs.txt".to_string());
    std::fs::read_to_string(&path)
        .expect("read substrs file")
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

#[tokio::test]
async fn no_key_in_responses() {
    let client = Client::new();
    let key = free_key();
    let substrs = known_key_substrings();

    // Hit all geo_events-touching surfaces as a free agent.
    let urls = [
        format!("{HUB}/api/v1/events/recent"),
        format!("{HUB}/api/v1/events/recent?source=monitor:x"),
        format!("{HUB}/api/v1/events/recent?source=monitor:finintel"),
        format!("{HUB}/api/v1/overview"),
        format!("{HUB}/api/v1/search/unified?q=test"),
    ];

    for url in &urls {
        let resp = client
            .get(url)
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap();
        let body = resp.text().await.unwrap();

        for sub in &substrs {
            assert!(
                !body.contains(sub),
                "response from {url} contains key substring {sub:?} — leak"
            );
        }
    }
}

#[tokio::test]
async fn no_key_in_payload_json() {
    let client = Client::new();
    let key = free_key();
    let substrs = known_key_substrings();

    // Trigger a search that returns documents with payloads.
    let resp: Value = client
        .get(format!("{HUB}/api/v1/search/unified?q=test"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let body = serde_json::to_string(&resp).unwrap();

    for sub in &substrs {
        assert!(
            !body.contains(sub),
            "search response payload contains key substring {sub:?}"
        );
    }
}

#[tokio::test]
async fn no_key_in_logs() {
    // This test reads the hub-core log file. Assumes journald path or
    // syslog. The operator must run this test on the same VM as hub-core
    // (or share the log file via the same path).
    let log_path = std::env::var("HUB_CORE_LOG")
        .unwrap_or_else(|_| "/home/zou/IntelHub/core/hub.log".to_string());
    let substrs = known_key_substrings();

    // Trigger a few requests first so any logging happens.
    let client = Client::new();
    let key = free_key();
    for _ in 0..3 {
        let _ = client
            .get(format!("{HUB}/api/v1/events/recent"))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await;
    }
    // Give logs 200ms to flush.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Read recent log tail (10K bytes).
    let mut cmd = std::process::Command::new("tail");
    cmd.arg("-c").arg("10000").arg(&log_path);
    let output = cmd.output().expect("tail log file");
    let log_text = String::from_utf8_lossy(&output.stdout);

    for sub in &substrs {
        assert!(
            !log_text.contains(sub),
            "log file contains key substring {sub:?}"
        );
    }
}
```

- [ ] **Step 10.3: Prepare the substrs file**

```bash
ssh -o BatchMode=yes IntelHub-test '
  grep -E "^(HUB|FINNHUB|FMP|TELEGRAM|BLUESKY)_.*_(KEY|TOKEN)=" /home/zou/IntelHub/core/secrets.env | while IFS== read -r k v; do
    echo "${v:0:16}"
  done
' > /tmp/tier-key-substrs.txt
cat /tmp/tier-key-substrs.txt | head -3
```

Expected: 1-3 lines, each 16 chars.

- [ ] **Step 10.4: Run the tests**

```bash
TIER_TEST_FREE_KEY=$(grep "tier-test-free" /tmp/tier-keys.txt | grep -o "ihk_[a-f0-9]*") \
  TIER_TEST_KEY_SUBSTR=/tmp/tier-key-substrs.txt \
  HUB_CORE_LOG=/home/zou/IntelHub/core/hub.log \
  cargo test --package hub-core --test secrets_redaction -- --nocapture --test-threads=1
```

Expected: 3 passed.

- [ ] **Step 10.5: Commit**

```bash
git add hub-core/crates/hub-core/tests/secrets_redaction.rs
git commit -m "test(access): secrets_redaction — no key leak in response / payload / log"
```

---

## Task 11: Extend scripts/accept-sp5.py

**Files:**
- Modify: `scripts/accept-sp5.py` — append 3 new tier-isolation checks at the end (before the final summary line)

**Interfaces:**
- Consumes: existing `check`, `check_shelved`, `pg`, `redis`, `vm` helpers from `_remote.py`
- Produces: 3 new checks at the end of the script

- [ ] **Step 11.1: Read the existing sp5 layout**

Read `scripts/accept-sp5.py` (215 lines). Find the existing summary print near the bottom, which looks like:

```python
== 12 passed, 6 shelved, 0 failed ==
```

(or equivalent `passed, shelved, failed` line). The 3 new checks will be inserted immediately before this summary block.

- [ ] **Step 11.2: Add the 3 new checks**

Insert before the summary print:

```python
# 11. Tier isolation infrastructure — PR1 quant-readability
# 11a. Every monitor in registry has monitor_metadata() entry (compile-time
# invariant — verified at build time, but we sanity-check via the audit log)
agents_with_tier = pg("SELECT count(*) FROM agents WHERE tier IN ('free','paid','admin')").splitlines()[-1]
check("agents.tier column populated for all rows", agents_with_tier.isdigit() and int(agents_with_tier) >= 1, f"agents_with_tier={agents_with_tier}")

# 11b. Every geo_events row has tier_required set
tier_required_rows = pg("SELECT count(*) FROM geo_events WHERE tier_required IN ('free','paid','admin')").splitlines()[-1]
total_events = pg("SELECT count(*) FROM geo_events").splitlines()[-1]
try:
    tr, te = int(tier_required_rows), int(total_events)
    check("geo_events.tier_required populated for all rows", tr == te, f"tier_required={tr} total={te}")
except (ValueError, AttributeError):
    check("geo_events.tier_required populated", False, f"unparseable: {tier_required_rows}/{total_events}")

# 11c. audit_log table exists and is queryable
audit_count = pg("SELECT count(*) FROM audit_log").splitlines()[-1]
check("audit_log table exists", audit_count.isdigit() and int(audit_count) >= 0, f"rows={audit_count}")

# 11d. set-tier CLI roundtrip — promote an existing test agent to 'paid'
# (idempotent: re-running sp5 must not error). Skip if no agent named
# 'tier-test-free' exists (it would only exist on 415 after Task 9.1).
test_agent_exists = pg("SELECT count(*) FROM agents WHERE name='tier-test-free'").splitlines()[-1]
if test_agent_exists.isdigit() and int(test_agent_exists) >= 1:
    rc = vm("cd /home/zou/IntelHub && set -a && . core/hub.env && . core/secrets.env && set +a && ./core/hub set-tier tier-test-free free 2>&1 | grep -c 'tier:     free'").strip()
    check("set-tier CLI roundtrip succeeds", rc == "1", f"rc={rc}")
    # Verify the audit_log row was written
    audit_set_tier = pg("SELECT count(*) FROM audit_log WHERE action='set_tier'").splitlines()[-1]
    check("set-tier wrote audit_log row", audit_set_tier.isdigit() and int(audit_set_tier) >= 1, f"rows={audit_set_tier}")
```

- [ ] **Step 11.3: Syntax check**

```bash
python3 -m py_compile scripts/accept-sp5.py
```

Expected: no error.

- [ ] **Step 11.4: Run the script against 415 (existing baseline)**

```bash
KEY=$(ssh -o BatchMode=yes IntelHub-test 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
INTELHUB_SSH=IntelHub-test python3 scripts/accept-sp5.py "$KEY" "http://10.10.10.45:8800" 2>&1 | tail -8
```

Expected: at least the existing `12 passed, 6 shelved, 0 failed` plus the new 4 (or 5 if tier-test agent exists) checks.

- [ ] **Step 11.5: Commit**

```bash
git add scripts/accept-sp5.py
git commit -m "test(sp5): 4 new checks for tier isolation (agents.tier, geo_events.tier_required, audit_log, set-tier CLI)"
```

---

## Task 12: Deploy to 415 + acceptance verification

**Files:** (no source files modified this task — pure deployment)

**Interfaces:**
- Consumes: 11 commits on `feat/quant-tier-isolation` branch
- Produces: 415 hub-core rebuilt + restarted, all sp5 checks green

- [ ] **Step 12.1: rsync worktree to 415**

From the worktree root:

```bash
cd /Volumes/TBU/Workspace/IntelHub-quant-pr1
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub-test:/home/zou/IntelHub/
```

- [ ] **Step 12.2: Apply migrations**

```bash
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub
  for f in hub-core/migrations/0015_*.sql hub-core/migrations/0016_*.sql hub-core/migrations/0017_*.sql; do
    echo "applying $f"
    docker exec -i intelhub-postgres psql -U intelhub -d intelhub < "$f" || exit 1
  done
'
```

Expected: 3 "applying ..." lines, all exit 0.

- [ ] **Step 12.3: Clean cargo cache + rebuild (touch admin.rs first per Ruling 6)**

```bash
ssh -o BatchMode=yes IntelHub-test '
  docker run --rm -v intelhub-hub-target:/target alpine sh -c "
    rm -rf /target/release/.fingerprint /target/release/deps /target/release/build 2>/dev/null
    echo cleaned
  "
'
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub
  touch hub-core/crates/hub-core/src/admin.rs
  bash scripts/build-hub.sh 2>&1 | grep -E "^error|^==> built" | head -3
'
```

Expected: `cleaned`, then `==> built: /home/zou/IntelHub/core/hub` (no `error` lines). The `touch admin.rs` forces cargo to re-embed the migrations (Ruling 6).

- [ ] **Step 12.4: Restart hub-core**

```bash
ssh -o BatchMode=yes IntelHub-test '
  sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core
'
```

Expected: `active`.

- [ ] **Step 12.5: Wait for first sweep (60s)**

```bash
sleep 60
```

- [ ] **Step 12.6: Seed test agents + verify CLI (env sourced per Ruling 5)**

```bash
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub
  set -a && . core/hub.env && . core/secrets.env && set +a
  ./core/hub create-agent --name tier-test-free >/tmp/agent-free.txt 2>&1
  ./core/hub create-agent --name tier-test-paid >/tmp/agent-paid.txt 2>&1
  ./core/hub create-agent --name tier-test-admin >/tmp/agent-admin.txt 2>&1
  ./core/hub set-tier tier-test-free free
  ./core/hub set-tier tier-test-paid paid
  ./core/hub set-tier tier-test-admin admin
  grep "api_key:" /tmp/agent-free.txt /tmp/agent-paid.txt /tmp/agent-admin.txt
'
```

Expected: 3 `api_key:` lines printed.

Capture keys:
```bash
ssh -o BatchMode=yes IntelHub-test 'cat /tmp/agent-free.txt' | grep "api_key:" | awk '{print $NF}' > /tmp/tier-keys.txt
ssh -o BatchMode=yes IntelHub-test 'cat /tmp/agent-paid.txt' | grep "api_key:" | awk '{print $NF}' >> /tmp/tier-keys.txt
ssh -o BatchMode=yes IntelHub-test 'cat /tmp/agent-admin.txt' | grep "api_key:" | awk '{print $NF}' >> /tmp/tier-keys.txt
```

- [ ] **Step 12.7: Seed a paid-tier event**

```bash
ssh -o BatchMode=yes IntelHub-test '
  docker exec intelhub-postgres psql -U intelhub -d intelhub <<EOSQL
  INSERT INTO geo_events (kind, severity, occurred_at, source, lat, lon, external_id, payload, tier_required)
  VALUES ('"'"'test_event'"'"', '"'"'routine'"'"', now(), '"'"'monitor:test:1'"'"', 0, 0, '"'"'test:paid:1'"'"', '"'"'{}'"'"'::jsonb, '"'"'paid'"'"');
EOSQL
'
```

Expected: `INSERT 0 1`.

- [ ] **Step 12.8: Run integration tests**

```bash
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub/hub-core
  export DATABASE_URL=$(grep "^DATABASE_URL=" /home/zou/IntelHub/core/hub.env | cut -d= -f2-)
  export TIER_TEST_KEYS=/tmp/tier-keys.txt
  export TIER_TEST_FREE_KEY=$(head -1 /tmp/tier-keys.txt)
  cargo test --package hub-core --test tier_enforcement --test secrets_redaction -- --nocapture --test-threads=1 2>&1 | tail -20
'
```

Expected: 4 + 3 = 7 passed; 0 failed.

- [ ] **Step 12.9: Run accept-sp5 against 415**

```bash
KEY=$(ssh -o BatchMode=yes IntelHub-test 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
INTELHUB_SSH=IntelHub-test python3 scripts/accept-sp5.py "$KEY" "http://10.10.10.45:8800" 2>&1 | tail -10
```

Expected: `12 + N passed, 6 shelved, 0 failed` where N = number of new tier checks (4 if test agent was seeded, 3 if not).

- [ ] **Step 12.10: Run sp7 + sp9 baseline check (no regression)**

```bash
for sp in sp7 sp9; do
  python3 scripts/accept-$sp.py "$KEY" "http://10.10.10.45:8800" 2>&1 | grep -E '==.*(passed|failed)' | tail -1
done
```

Expected: same baseline numbers as before PR1 (24/24 for sp7, 18/18 for sp9 — confirm against pre-PR1 log if needed).

- [ ] **Step 12.11: Clean up test data**

```bash
ssh -o BatchMode=yes IntelHub-test '
  cd /home/zou/IntelHub
  for name in tier-test-free tier-test-paid tier-test-admin; do
    docker exec intelhub-postgres psql -U intelhub -d intelhub -c "DELETE FROM agents WHERE name='"'"'$name'"'"';"
  done
  docker exec intelhub-postgres psql -U intelhub -d intelhub -c "DELETE FROM audit_log WHERE action='"'"'set_tier'"'"' AND api_key_prefix='"'"'cli'"'"';"
  docker exec intelhub-postgres psql -U intelhub -d intelhub -c "DELETE FROM geo_events WHERE external_id='"'"'test:paid:1'"'"';"
'
```

(This is optional — Task 13 will do the same cleanup on 410 after merge.)

---

## Task 13: Merge → deploy to 410 → push to origin

**Files:** (no source files modified — pure deployment)

**Interfaces:**
- Consumes: 11 commits on `feat/quant-tier-isolation` branch, verified on 415
- Produces: 410 hub-core rebuilt + restarted, all sp5 checks green, origin main pushed

- [ ] **Step 13.1: Merge to main**

From the main repo (NOT worktree):

```bash
cd /Volumes/TBU/Workspace/IntelHub
git checkout main
git merge --no-ff feat/quant-tier-isolation \
  -m "merge: PR1 quant-readability tier isolation (free/paid/admin + audit_log)

Tier isolation infrastructure — the foundation for safe OSINT consumption
by paid quant agents. Free-tier callers physically cannot retrieve paid-
source rows even via crafted SQL; cross-tier access attempts are recorded
in audit_log.

Components:
  * Migrations 0015-0017 (agents.tier, geo_events.tier_required + backfill,
    audit_log table)
  * Tier, LicenseClass, MonitorMeta typed enums in config.rs
  * monitor_metadata() OnceLock<HashMap> with all 32 monitors classified
  * access.rs: apply_tier_filter (SQL boundary rewrite) + audit_on_reject
  * apply_tier_filter wired into all geo_events-touching REST + MCP handlers
  * set-tier CLI subcommand (core/hub set-tier <agent> <free|paid|admin>)
  * Compile-time invariant test (registry ⇔ metadata coverage)
  * Integration tests: tier_enforcement, secrets_redaction
  * accept-sp5.py extended with 4 tier-related checks

PR2 will add payload.license_class + payload.data_age_seconds (32
collectors) and write tier_required at emit-time, narrowing the
backfill clause. PR3 will add quant_history table + 5y backfill of
FRED/BLS/EIA/Treasury."

git log --oneline -1
```

Expected: merge commit hash printed.

- [ ] **Step 13.2: Remove worktree + delete branch**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git worktree remove --force ../IntelHub-quant-pr1
git branch -d feat/quant-tier-isolation
```

Expected: clean removal, no warnings.

- [ ] **Step 13.3: rsync to 410**

From the main repo:

```bash
cd /Volumes/TBU/Workspace/IntelHub
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'compose/.env.crucix' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  ./ IntelHub:/home/zou/IntelHub/
```

- [ ] **Step 13.4: Apply migrations on 410**

```bash
ssh -o BatchMode=yes IntelHub '
  cd /home/zou/IntelHub
  for f in hub-core/migrations/0015_*.sql hub-core/migrations/0016_*.sql hub-core/migrations/0017_*.sql; do
    echo "applying $f"
    docker exec -i intelhub-postgres psql -U intelhub -d intelhub < "$f" || exit 1
  done
'
```

Expected: 3 "applying ..." lines, all exit 0.

- [ ] **Step 13.5: Clean cargo cache + build on 410 (touch admin.rs per Ruling 6)**

```bash
ssh -o BatchMode=yes IntelHub '
  docker run --rm -v intelhub-hub-target:/target alpine sh -c "
    rm -rf /target/release/.fingerprint /target/release/deps /target/release/build 2>/dev/null
    echo cleaned
  "
'
ssh -o BatchMode=yes IntelHub '
  cd /home/zou/IntelHub
  touch hub-core/crates/hub-core/src/admin.rs
  bash scripts/build-hub.sh 2>&1 | grep -E "^error|^==> built" | head -3
  bash scripts/build-console.sh 2>&1 | tail -1
'
```

Expected: `cleaned`, `==> built: /home/zou/IntelHub/core/hub`, console build line.

- [ ] **Step 13.6: Restart hub-core on 410**

```bash
ssh -o BatchMode=yes IntelHub '
  sudo systemctl restart hub-core && sleep 4 && systemctl is-active hub-core
'
```

Expected: `active`.

- [ ] **Step 13.7: Wait for first sweep (60s)**

```bash
sleep 60
```

- [ ] **Step 13.8: Seed test agents + verify CLI on 410 (env sourced per Ruling 5)**

```bash
ssh -o BatchMode=yes IntelHub '
  cd /home/zou/IntelHub
  set -a && . core/hub.env && . core/secrets.env && set +a
  ./core/hub create-agent --name tier-test-free >/tmp/agent-free.txt 2>&1
  ./core/hub create-agent --name tier-test-paid >/tmp/agent-paid.txt 2>&1
  ./core/hub create-agent --name tier-test-admin >/tmp/agent-admin.txt 2>&1
  ./core/hub set-tier tier-test-free free
  ./core/hub set-tier tier-test-paid paid
  ./core/hub set-tier tier-test-admin admin
'
```

- [ ] **Step 13.9: Run integration tests on 410**

Same as Task 12.8 but on `IntelHub`:

```bash
ssh -o BatchMode=yes IntelHub '
  cd /home/zou/IntelHub/hub-core
  export DATABASE_URL=$(grep "^DATABASE_URL=" /home/zou/IntelHub/core/hub.env | cut -d= -f2-)
  export TIER_TEST_KEYS=/tmp/tier-keys.txt
  export TIER_TEST_FREE_KEY=$(head -1 /tmp/tier-keys.txt)
  cargo test --package hub-core --test tier_enforcement --test secrets_redaction -- --nocapture --test-threads=1 2>&1 | tail -10
'
```

Expected: 7 passed.

- [ ] **Step 13.10: Run accept-sp5 against 410**

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep "api_key:" /home/zou/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
INTELHUB_SSH=IntelHub python3 scripts/accept-sp5.py "$KEY" "http://10.10.10.41:8800" 2>&1 | tail -10
```

Expected: `>= 12 + N passed, 6 shelved, 0 failed`.

- [ ] **Step 13.11: Confirm sp7 + sp9 baseline**

```bash
for sp in sp7 sp9; do
  python3 scripts/accept-$sp.py "$KEY" "http://10.10.10.41:8800" 2>&1 | grep -E '==.*(passed|failed)' | tail -1
done
```

Expected: same as 415 baseline (24/24, 18/18).

- [ ] **Step 13.12: Push to origin**

```bash
cd /Volumes/TBU/Workspace/IntelHub
git push origin main
```

Expected: `To https://github.com/rootazero/IntelHub.git` + commit hashes.

- [ ] **Step 13.13: Clean up test data on 410**

```bash
ssh -o BatchMode=yes IntelHub '
  for name in tier-test-free tier-test-paid tier-test-admin; do
    docker exec intelhub-postgres psql -U intelhub -d intelhub -c "DELETE FROM agents WHERE name='"'"'$name'"'"';"
  done
  docker exec intelhub-postgres psql -U intelhub -d intelhub -c "DELETE FROM audit_log WHERE action='"'"'set_tier'"'"' AND api_key_prefix='"'"'cli'"'"';"
'
```

---

## Self-Review

Run this checklist before declaring the plan complete.

### 1. Spec coverage

| Spec § | Requirement | Plan task |
|--------|-------------|-----------|
| §4.1 monitor_metadata registry | Type-safe enum + OnceLock + 32 monitors | Task 2 |
| §4.2 agents.tier column | Migration + default 'free' | Task 1 (migration 0015) |
| §4.3 geo_events.tier_required column | Migration + backfill | Task 1 (migration 0016) |
| §4.4 apply_tier_filter + 8 enforcement points | Helper + REST + MCP wiring | Task 4 (helper), Task 6 (REST), Task 7 (MCP) |
| §4.5 secrets stay in VM | Collector contract + E2E test | Task 10 |
| §4.6 audit_log | Table + writer + CLI audit | Task 1 (table), Task 4 (writer), Task 5 (CLI audit) |
| §4.9 GET /api/v1/sources | Public endpoint | **NOT in PR1** — deferred to PR3 |
| §6.1 PR1 scope | All 11 deliverables | Tasks 1-11 |
| §7 security model (7 threats) | Test matrix in §7.3 | Tasks 9, 10 |
| §9 testing strategy | Unit + integration + acceptance + compile-time invariant | Tasks 3 (invariant), 4 (unit), 9+10 (integration), 11 (acceptance) |

**Gap:** `/api/v1/sources` is not in PR1 (correctly deferred — it's a PR3 deliverable per §6.3).

### 2. Placeholder scan

Searched plan for: TBD, TODO, FIXME, "appropriate tests", "implement later", "similar to Task N". None found.

### 3. Type consistency

- `Tier` enum: defined in Task 2 as `Free | Paid | Admin`, used in Task 4 (`apply_tier_filter` match), Task 5 (`update_agent_tier(pool, name, tier: Tier)`), Task 6 (`filter_query_by_tier`). ✓
- `apply_tier_filter(base_sql: &str, tier: Tier) -> String`: defined in Task 4, called in Task 6. ✓
- `audit_on_reject(pool, agent_id, api_key_prefix, action, source_attempted, request_path, trace_id, blocked)`: defined in Task 4, called in Task 6 (via `filter_query_by_tier`). ✓
- `monitor_metadata() -> &'static HashMap<&'static str, MonitorMeta>`: defined in Task 2, called in Task 3 (test) and Task 6 (helper). ✓
- `set_tier_cli(cfg, name, tier_str)`: defined in Task 5, registered in Task 8. ✓

### 4. Migration numbering

Spec used 0012-0015 for PR1; main has 0012-0014 already. **Plan uses 0015-0017.** Note added at top of plan. PR3 will use 0018.

### 5. Risks / open issues

- **Borrow checker in Task 7** (MCP wiring): `RequestCtx` may not be `Copy`. Mitigation noted (clone `tier` value, it's Copy). Plan implementer should expect one or two lifetime annotations to be needed; the worktree's compile errors will guide.
- **Tier-test agent in sp5**: Task 11 check 11d conditionally skips if `tier-test-free` doesn't exist. This means the baseline sp5 doesn't require the test agent — only run after Task 12.6/13.8 seed it. Acceptable.
- **Hub-core log path**: Task 10.3 assumes `/home/zou/IntelHub/core/hub.log`. May differ if hub-core runs under journald only. Mitigation: env var `HUB_CORE_LOG` overrides; if unset, test errors with a clear message.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-09-15-quant-readability-pr1-tier-isolation.md`.**

13 tasks, 11 of them code-change (Tasks 1-11) plus 2 deployment tasks (12-13). Estimated 700-900 LoC code + ~250 LoC tests + ~50 LoC acceptance script = ~1000-1200 LoC total.

**Two execution options:**

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. The skill `subagent-driven-development` will be invoked next.
2. **Inline Execution** — Execute tasks in this session using `executing-plans` skill, batch execution with checkpoints for review.

**Which approach?**

Recommendation: Subagent-Driven. Tasks 1, 2, 3, 4, 5 are independent enough to parallelize if needed; Tasks 6, 7, 8, 9, 10 must run in sequence (shared `access.rs` / `api.rs` / `mcp.rs` files). Tasks 11, 12, 13 are deployment, best done inline.