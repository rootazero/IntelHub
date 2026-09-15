# Quant Readability Layer — Design Spec

Date: 2026-09-15
Status: Approved by user (2026-09-15)
Supersedes: none. Derived from the "IntelHub for 量化交易 AI agent" feasibility analysis (2026-09-15 chat).

## 0. Scope

Make IntelHub's OSINT data plane **legally safe + structurally usable** as an auxiliary data feed for a downstream quant trading AI agent, without compromising IntelHub's existing 32-monitor OSINT breadth or license posture.

Out of scope:
- Real-time market data (Bloomberg/Refinitiv/Polygon integration)
- Order execution / routing (regulatory + liability separation)
- HFT / sub-minute data feeds (out of OSINT cadence by definition)
- Backtest framework / factor research tooling (consumer's responsibility)
- Surprise-factor computation (v1 simplified — see §6.4)

In scope:
- Tier-based access control at the SQL layer (free / paid / admin)
- Per-monitor license metadata, compiled-in and immutable
- Audit logging for cross-tier access attempts
- data_age_seconds on every emitted Signal
- Historical backfill of 4 US-federal data sources to a dedicated quant_history table
- Single endpoint `GET /api/v1/sources` for transparency

## 1. Goals (success criteria)

| # | Criterion | Measurable as |
|---|-----------|---------------|
| G1 | A `free`-tier agent cannot physically retrieve any event from a `tier_required='paid'` source, regardless of client filter attempts | SQL EXPLAIN shows paid rows absent from query plan |
| G2 | Paid-API keys (finnhub, stocktwits, X) never appear in any API response body or log line | E2E test asserts key substring absence across all endpoints |
| G3 | Cross-tier access attempts leave an audit trail | `audit_log` row written within 100ms of the rejected query |
| G4 | Every Signal carries data_age_seconds ≤ actual upstream latency | Field present on 100% of new Signals; numerically ≤ documented upstream latency |
| G5 | quant_history contains ≥ 5 years of daily/weekly data for FRED, BLS, EIA, Treasury | SQL `SELECT count(DISTINCT series_id), MIN(ts), MAX(ts)` per source |
| G6 | Three independent PRs deliver the scope, each with full SP5/SP7/SP9 acceptance green before merge | Git log shows 3 merge commits; acceptance output saved |
| G7 | License / tier semantics are auditable from code alone (no hidden DB-only state) | `monitor_metadata` is a single HashMap in `config.rs`, no other source of truth |

## 2. Non-Goals (what we explicitly will not do)

| Non-goal | Reason |
|----------|--------|
| Token issuance / OAuth for tier upgrade | User manages tier via `core/hub set-tier` CLI; agents never self-upgrade |
| Surprise-factor computation in v1 | Investing.com scraping TOS risk; FRED consensus coverage too sparse. v2 with paid consensus feed |
| Per-user rate limiting in v1 | Existing global rate limit suffices. v2 may add per-agent quotas |
| Encryption at rest for paid events | Existing PG encryption covers all rows. Per-event crypto would prevent index use |
| Stock-level K-anonymity | Out of OSINT scope; user agent owns its own privacy model |
| Polygon.io / Alpaca / Unusual Whales integration | Breaks the "free + open" moat; deferred to v2 user decision |
| WebSocket real-time push | Existing REST + MCP poll cadence (6h–24h) is sufficient for OSINT-driven decisions |

## 3. Architecture Overview

```
                        ┌─────────────────────────┐
   Agent (free/paid/    │  REST/MCP Handler       │
   admin) ──────────────►  + apply_tier_filter()  │ ◄── SQL-layer WHERE
                        │  + audit_on_reject()    │     enforcement
                        └──────┬──────────────────┘
                               │
                               ▼
                  ┌──────────────────────────────┐
                  │  PostgreSQL: geo_events      │
                  │  + tier_required column      │
                  │  + payload.license_class     │
                  │  + payload.data_age_seconds  │
                  └──────────────────────────────┘
                               ▲
                               │ (write at emit)
                  ┌──────────────────────────────┐
                  │  32 Monitor Collectors       │
                  │  read: secrets.env           │
                  │  consult: monitor_metadata   │
                  └──────────────────────────────┘

                  ┌──────────────────────────────┐
                  │  PostgreSQL: quant_history   │ ◄── PR3 backfill
                  │  (FRED/BLS/EIA/Treasury)     │
                  └──────────────────────────────┘

                  ┌──────────────────────────────┐
                  │  PostgreSQL: agents          │ ◄── tier column
                  │  Redis: hub:audit:log        │ ◄── recent rejects
                  └──────────────────────────────┘

                  ┌──────────────────────────────┐
                  │  Rust source: config.rs      │
                  │  monitor_metadata: HashMap   │ ◄── single source
                  │  enum Tier {Free,Paid,Admin} │     of truth, compiled
                  └──────────────────────────────┘
```

Key invariants:
1. **One source of truth**: `monitor_metadata` HashMap in `config.rs`. New monitor added ⇒ metadata registered in same PR.
3. **Type-safe tier enum**: Rust `enum Tier { Free, Paid, Admin }` — impossible to construct an arbitrary string tier.
4. **Server-side enforcement only**: client never gets a filter to opt out of tier enforcement. Free agents cannot enumerate paid sources via a permissive query.

## 4. Components

### 4.1 monitor_metadata registry

Defined in `hub-core/crates/hub-core/src/config.rs` as:

```rust
use std::collections::HashMap;
use std::sync::OnceLock;

pub struct MonitorMeta {
    pub tier_required: Tier,
    pub license_class: LicenseClass,
    pub data_age_hours: u32,         // estimated max upstream staleness
    pub doc_url: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier { Free, Paid, Admin }

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

pub fn monitor_metadata() -> &'static HashMap<&'static str, MonitorMeta> {
    static CACHE: OnceLock<HashMap<&'static str, MonitorMeta>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut m = HashMap::with_capacity(32);
        m.insert("usgs",  MonitorMeta { tier: Tier::Free,  lic: LicenseClass::Open, age: 6, url: "https://earthquake.usgs.gov/earthquakes/feed/v1.0/geojson.php" });
        m.insert("noaa",  MonitorMeta { tier: Tier::Free,  lic: LicenseClass::Open, age: 1, url: "https://www.weather.gov/documentation/services-web-api" });
        m.insert("firms", MonitorMeta { tier: Tier::Free,  lic: LicenseClass::Open, age: 3, url: "https://firms.modaps.eosdis.nasa.gov/api/" });
        m.insert("gdelt", MonitorMeta { tier: Tier::Free,  lic: LicenseClass::Open, age: 1, url: "https://blog.gdeltproject.org/gdelt-2-0-english-translation-api/" });
        // ... (all 32 monitors enumerated in PR1; see §4.1.1 for full table)
        m
    })
}
```

Choice rationale: `OnceLock<HashMap>` is stdlib-only (no extra dependency), lock-free after first init, idiomatic for static config that depends on a non-`const` value. `phf` would be marginally faster but requires a new dep — not justified for 32 entries. The compiler still enforces correctness via the metadata-completeness test (§9.4).

**Critical invariant (compile-time test)**: every name in `monitor/mod.rs` registry MUST have a `MONITOR_METADATA` entry. CI test asserts `registry_names ⊆ metadata_names ⊆ registry_names`.

#### 4.1.1 Per-monitor license classification (provisional)

| Monitor | tier | license | Reasoning |
|---------|------|---------|-----------|
| `usgs`, `noaa`, `epa`, `eonet`, `who`, `climateseries`, `radiation` | Free | Open | US-gov / UN / WHO data — public domain |
| `firms` | Free | Open | NASA FIRMS — public domain |
| `bls`, `fred`, `treasury`, `comtrade`, `usaspending`, `gscpi` | Free | Open | US-gov / UN Comtrade / NY Fed — public domain |
| `sec_edgar`, `ofac` | Free | Open | SEC / OFAC — public domain |
| `gdelt`, `acled`, `reliefweb` | Free | Open | ODbL / CC-BY |
| `kiwisdr` | Free | Open | SDR receiver data — user's own |
| `nvd`, `cisakev`, `osv` | Free | Open | NIST / Google OSV — public domain / CC-BY |
| `opensanctions` | Free | Open | ODbL |
| `x`, `bluesky` | Paid | Restricted | X API + Bluesky API — paid key, redistribution prohibited |
| `finintel` (finnhub + stocktwits) | Paid | Restricted | Both APIs are paid + restrict redistribution |
| `markets` | Paid | Restricted | Paid market data feed |
| `telegram` | Free | FairUse | Telegram channel content — fair-use but not redistributable wholesale |
| `rss` | Free | FairUse | RSS feeds — fair-use; redistribute snippet only |
| `textclass` | Free | Open | Internal helper, not a feed |
| `opensky` | Admin | NonCommercial | NC clause — gated to admin tier only, see prior wontfix analysis |

**Migration**: existing `geo_events` rows pre-migration have no `tier_required`. SQL UPDATE FROM join sets them based on source name (one-shot migration).

### 4.2 agents.tier column

New migration `0012_agents_tier.sql`:

```sql
ALTER TABLE agents ADD COLUMN tier TEXT NOT NULL DEFAULT 'free';
CREATE INDEX idx_agents_tier ON agents(tier);
```

Existing agents in `core/agent-keys.txt` default to `free`. Admin promotes via:

```
core/hub set-tier --api-key ihk_<hex> --tier paid
core/hub set-tier --api-key ihk_<hex> --tier admin
```

CLI subcommand `set_tier` (new) lives in `hub-core/src/cli/set_tier.rs`, calls into `state.db.update_agent_tier()`.

**Tier hierarchy**:
- `admin` sees all events (including `tier_required='admin'` like opensky)
- `paid` sees `tier_required IN ('free', 'paid')`
- `free` sees only `tier_required='free'`

### 4.3 geo_events.tier_required column

New migration `0013_geo_events_tier_required.sql`:

```sql
ALTER TABLE geo_events ADD COLUMN tier_required TEXT NOT NULL DEFAULT 'free';
CREATE INDEX idx_geo_events_tier_required ON geo_events(tier_required);
```

**Backfill** (one-shot migration step, not a separate migration file):

```sql
-- Backfill tier_required from source name parsing
UPDATE geo_events
SET tier_required = CASE
  WHEN source LIKE 'monitor:x%' OR source = 'monitor:x' THEN 'paid'
  WHEN source LIKE 'monitor:bluesky%' OR source = 'monitor:bluesky' THEN 'paid'
  WHEN source LIKE 'monitor:finintel%' OR source = 'monitor:finintel' THEN 'paid'
  WHEN source LIKE 'monitor:markets%' OR source = 'monitor:markets' THEN 'paid'
  WHEN source LIKE 'monitor:opensky%' OR source = 'monitor:opensky' THEN 'admin'
  ELSE 'free'
END
WHERE tier_required = 'free'; -- only backfill rows that haven't been explicitly set
```

After PR2 (where every new event writes its tier at emit), the WHERE clause narrows; eventually zero rows match.

### 4.4 apply_tier_filter helper

Lives in `hub-core/src/access.rs`:

```rust
pub enum Tier { Free, Paid, Admin }
pub trait TierAccess {
    fn tier(&self) -> Tier;
}

pub fn apply_tier_filter<T: TierAccess>(
    base_query: &str,
    agent: &T,
) -> String {
    match agent.tier() {
        Tier::Admin => base_query.to_string(),
        Tier::Paid => format!(
            "{base_query} AND tier_required IN ('free', 'paid')"
        ),
        Tier::Free => format!("{base_query} AND tier_required = 'free'"),
    }
}
```

**Enforcement points** (every query path that touches `geo_events`):
- `GET /api/v1/events` (REST)
- `GET /api/v1/overview` (REST)
- `hybrid_search` MCP handler
- `semantic_search` MCP handler
- `keyword_search` MCP handler
- `query_entity` MCP handler (path that returns events attached to an entity)
- `find_path` MCP handler
- `investigate` MCP handler (multi-step executor)

Test: integration test `tests/tier_enforcement.rs` posts as a `free` agent, requests `/api/v1/events?source=monitor:x`, asserts `200 OK` body has `events: []` AND `audit_log` has a `tier_mismatch` row. The 200 (not 403) is deliberate: returning 403 reveals that the source exists; empty-array + silent logging is information-leak-free.

### 4.5 Secrets stay in VM (defense in depth)

E2E test `tests/secrets_redaction.rs`:

- Spawns 100 free-tier requests across `/api/v1/events`, `/api/v1/overview`, `/mcp` (hybrid_search)
- Asserts no response body contains the substring of any `HUB_*_API_KEY` value
- Asserts no log line (stdout, stderr, hub-core log file) contains the substring

**Collector emit contract**: `payload` MUST NEVER include:
- The collector's API key
- The upstream request URL (which may embed the key as query param)
- The raw `Authorization` header value

Code-review checklist item in PR template.

### 4.6 audit_log table

New migration `0014_audit_log.sql`:

```sql
CREATE TABLE audit_log (
    id BIGSERIAL PRIMARY KEY,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    agent_id UUID REFERENCES agents(agent_id),
    api_key_prefix TEXT NOT NULL,    -- first 12 chars of the key, not the secret
    action TEXT NOT NULL,            -- e.g. 'tier_mismatch_query'
    source_attempted TEXT,
    request_path TEXT,
    trace_id UUID,
    blocked BOOLEAN NOT NULL
);
CREATE INDEX idx_audit_log_ts ON audit_log(ts);
CREATE INDEX idx_audit_log_agent ON audit_log(agent_id);
```

Logged events:
- Tier-mismatch queries (free agent queries paid-source events)
- Invalid API key (already logged; ensure consistent schema)
- Set-tier CLI invocations (admin action audit)

Retention: 90 days. A nightly cron deletes `audit_log WHERE ts < now() - INTERVAL '90 days'`.

Operator surface: `GET /api/v1/audit?agent_id=...&from=...` for admin tier only.

### 4.7 payload.license_class + payload.data_age_seconds

Every `Signal::new(...).emit(...)` call writes these fields. Existing 32 collectors all modified.

**Collector emit template** (pseudocode for the change in each collector):

```rust
let meta = MONITOR_METADATA.get(self.name())
    .expect("metadata registered"); // compile-time-enforced invariant
let data_published_at = parse_published_at(&response_body)?;
let data_age_seconds = (now() - data_published_at).num_seconds() as u64;

Signal::new(kind, title, lat, lon, external_id)
    .severity(sev)
    .payload(serde_json::json!({
        "license_class": meta.license_class.as_str(),
        "data_age_seconds": data_age_seconds,
        // ... existing fields
    }))
    .occurred(data_published_at)
```

**published_at parsing**: each collector knows its source's response shape. EIA has `period` field, BLS has `year+period` derived into a date, FRED has `date` in the observations array. NVD has `published` field. The collector declares its `fn parse_published_at(&Value) -> Option<DateTime>` in a small helper trait; default implementation falls back to `now()` and emits `data_age_seconds=0` (a known-weak signal — see §6.2).

### 4.8 quant_history table + backfill

New migration `0015_quant_history.sql`:

```sql
CREATE TABLE quant_history (
    id BIGSERIAL PRIMARY KEY,
    source TEXT NOT NULL,            -- 'fred' | 'bls' | 'eia' | 'treasury'
    series_id TEXT NOT NULL,         -- e.g. 'DGS10', 'CPIAUCSL', 'STEO.WPROUS'
    ts DATE NOT NULL,
    value NUMERIC,
    tier_required TEXT NOT NULL DEFAULT 'free',
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source, series_id, ts)
);
CREATE INDEX idx_quant_history_source_series ON quant_history(source, series_id, ts);
```

**Backfill on first run** (one-shot per source, idempotent via UNIQUE constraint):

| Source | API endpoint | Series (default) | Lookback |
|--------|--------------|------------------|----------|
| FRED | `https://api.stlouisfed.org/fred/series/observations` | `DGS10`, `DGS2`, `CPIAUCSL`, `M2SL`, `UNRATE`, `INDPRO`, `RSAFS`, `HOUST`, `DEXUSEU`, `DCOILWTICO` | 5 years (1825 days) |
| BLS | `https://api.bls.gov/publicAPI/v2/timeseries/data/` | `CUUR0000SA0` (CPI-U), `LNS14000000` (unemployment), `CES0000000001` (nonfarm payrolls), `WPUFD49207` (PPI) | 5 years |
| EIA | `https://api.eia.gov/v2/series/?frequency=weekly` | `STEO.WPROUS` (crude production), `STEO.WPUSIRI` (imports), `PET.WCRSTUS1.W` (commercial crude inventories), `NG.NW2_EPG0_SWO_R48_BCF.W` (natgas storage) | 5 years |
| Treasury | `https://home.treasury.gov/resource-center/data-chart-center/interest-rates/daily-treasury-rates.csv/{year}/all` | All par yield curve points (1mo, 2mo, 3mo, 6mo, 1y, 2y, 3y, 5y, 7y, 10y, 20y, 30y) | 5 years |

Requires free keys for FRED and EIA (BLS is open up to 500 queries/day per registered key, Treasury CSV is public). Backfill runs in a separate binary `core/hub backfill-quant-history` invoked via systemd `hub-core-backfill.service` (one-shot, not a timer).

**Cadence after backfill**: normal scheduler appends new daily values. quant_history grows monotonically.

**API surface**: `GET /api/v1/history?source=fred&series=DGS10&from=2020-01-01&to=2025-12-31` returns the series. Tier-gated identically to geo_events.

### 4.9 REST endpoint /api/v1/sources

Returns the full registry:

```json
{
  "sources": [
    {
      "name": "fred",
      "tier_required": "free",
      "license_class": "open",
      "data_age_hours": 24,
      "doc_url": "https://fred.stlouisfed.org/docs/api/",
      "endpoint_count": 1
    },
    ...
  ]
}
```

Public endpoint (no auth required) — operators want to inspect capabilities before issuing an API key.

## 5. Data Model Summary

### 5.1 New columns

| Table | Column | Type | Default | Purpose |
|-------|--------|------|---------|---------|
| `agents` | `tier` | TEXT | `'free'` | Caller identity tier |
| `geo_events` | `tier_required` | TEXT | `'free'` | Server-side filter gate |

### 5.2 New tables

| Table | Rows estimate (5y) | Purpose |
|-------|---------------------|---------|
| `quant_history` | ~50K (4 sources × 10 series × 5y × 250d) | Backfilled time-series panel |
| `audit_log` | grows ~1K/day | Cross-tier access attempts |

### 5.3 New indexes

| Index | Reason |
|-------|--------|
| `idx_agents_tier` | Set-tier CLI lookup |
| `idx_geo_events_tier_required` | apply_tier_filter WHERE clause |
| `idx_quant_history_source_series` | /api/v1/history query path |
| `idx_audit_log_ts`, `idx_audit_log_agent` | Operator queries |

## 6. Phased Rollout (3 PRs)

### 6.1 PR1 — Tier isolation infrastructure

**Scope:**
- `0012_agents_tier.sql` migration
- `0013_geo_events_tier_required.sql` migration + backfill SQL
- `0014_audit_log.sql` migration
- `MonitorMeta` struct + `MONITOR_METADATA` phf map in `config.rs`
- `Tier`, `LicenseClass` enums
- `apply_tier_filter()` in `hub-core/src/access.rs`
- Wire into all 8 enforcement points (REST + 5 MCP handlers)
- `core/hub set-tier` CLI subcommand
- Compile-time invariant test (registry ⊆ metadata)
- Integration test `tests/tier_enforcement.rs`
- Integration test `tests/secrets_redaction.rs`

**Acceptance:**
- sp5, sp7, sp9 baseline still green (no behavior change for `free` agents on `free` sources)
- `tests/tier_enforcement.rs` passes
- `tests/secrets_redaction.rs` passes
- `cargo check` + `cargo clippy` clean
- 415 deploy + verify, then 410 deploy + verify
- One merge commit to main; push origin

**Estimated LoC:** 600–800

### 6.2 PR2 — payload.license_class + payload.data_age_seconds

**Scope:**
- Modify all 32 collectors to consult `MONITOR_METADATA` at emit time
- Add `fn parse_published_at(&Value) -> Option<DateTime>` per collector (or use a default if source doesn't expose one — log a warning)
- Embed `payload.license_class`, `payload.data_age_seconds`, plus `tier_required` (top-level column populated from emit path)
- Add `tests/payload_metadata.rs` integration test asserting all 32 monitors emit these fields

**Caveat:** some sources don't expose a clean `published_at`. Examples: ACLED has `event_date` only (no time component, so `data_age_seconds` reflects time-since-midnight-UTC of event_date, which is always ≤ 24h and sometimes wildly inflated). Document each source's `parse_published_at` accuracy in a comment block above the function.

**Acceptance:**
- sp5 baseline green; +3 new sp5 checks: license_class field present, data_age_seconds ≤ upstream-latency doc, tier_required matches metadata
- All 32 collectors emit the new fields
- 415 → 410 deploy sequence

**Estimated LoC:** 32 × ~20 = ~640

### 6.3 PR3 — quant_history + backfill

**Scope:**
- `0015_quant_history.sql` migration
- New binary `core/hub backfill-quant-history` (one-shot CLI subcommand)
- Backfill logic for FRED, BLS, EIA, Treasury (5 years each)
- New systemd unit `hub-core-backfill.service` (one-shot, depends on `hub-core.service`)
- New REST endpoint `GET /api/v1/history` (tier-gated)
- Integration test `tests/quant_history.rs` asserting ≥ 5 years of data per source

**Acceptance:**
- sp5 baseline green; +3 new checks: history table populated, ≥ 5y lookback per source, tier-gated correctly
- 415 backfill dry-run (small sample); 410 production backfill
- Add new collect_url `crawl_url`-style endpoint to MCP? Out of scope; user can call REST

**Estimated LoC:** 400–500

### 6.4 Surprise factor — deferred to v2

The Investing.com scraping TOS risk + FRED consensus coverage sparsity make v1 surprise computation risky. PR2 emits `payload.actual_value` + `payload.expected_date` so consumers can compute surprise if they have their own consensus feed. PR3 quant_history stores values only (no surprise). A future v2 PR can add surprise if user provisions a paid consensus source (LSEG, Bloomberg, or Hedgeye).

## 7. Security Model

### 7.1 Threat model

| Threat | Mitigation |
|--------|------------|
| Free-tier agent enumerates paid sources | apply_tier_filter always applied; cannot be bypassed via query |
| Free-tier agent accesses paid source directly via SQL | Same: PG column `tier_required` enforces the WHERE at the DB level (no client filter bypass) |
| Paid API key leaks via response body or log | Collector emit contract (§4.5) + E2E test that scans all responses |
| Tier downgrade attack (admin loses tier) | admin actions via `core/hub set-tier` write audit_log; downgrade path requires admin auth |
| Free agent constructs query that reveals paid source exists | Empty 200 with `events: []` (not 403); no information leak |
| Backfill binary accidentally exposes API key | Backfill reads from secrets.env, never embeds in DB rows |
| audit_log tampering | Append-only via PG role permissions (separate `hub_audit_writer` PG role with INSERT-only privilege) |

### 7.2 Out-of-band protections (already in place)

- `core/secrets.env` is mode 0600; `core/hub` runs as user `zou`, never `root`
- API key in `Authorization: Bearer ihk_<hex>`; hash-checked (existing)
- TLS for all external API calls (existing)

### 7.3 Test matrix (security)

| Test | Asserts |
|------|---------|
| `tests/tier_enforcement.rs::free_cannot_read_paid_events` | Free agent `/api/v1/events?source=monitor:x` returns `events: []`; no paid rows in PG result set |
| `tests/tier_enforcement.rs::paid_reads_paid_and_free` | Paid agent sees paid + free events |
| `tests/tier_enforcement.rs::admin_reads_admin_paid_free` | Admin sees opensky + X + finintel + everything |
| `tests/tier_enforcement.rs::audit_log_records_mismatch` | After free agent fails, `audit_log` has row referencing agent_id + source='monitor:x' |
| `tests/secrets_redaction.rs::no_key_in_responses` | 100 free-tier requests, no key substring appears in any response |
| `tests/secrets_redaction.rs::no_key_in_logs` | Tail of hub-core stdout/stderr after test run contains no key substring |
| `tests/secrets_redaction.rs::no_key_in_payload_json` | `payload::text` for any event does not contain a key substring |

## 8. Migration Strategy

### 8.1 Migrations order (numerical sequence)

```
0012_agents_tier.sql              (PR1)
0013_geo_events_tier_required.sql (PR1, includes backfill UPDATE)
0014_audit_log.sql                (PR1)
0015_quant_history.sql            (PR3)
```

Embedded migration version constant bumped per PR.

### 8.2 Rollback

Each migration is reversible:
- `0012` rollback: `ALTER TABLE agents DROP COLUMN tier`
- `0013` rollback: `ALTER TABLE geo_events DROP COLUMN tier_required`
- `0014` rollback: `DROP TABLE audit_log`
- `0015` rollback: `DROP TABLE quant_history`

Backfill data in `quant_history` is preserved (just DROP the table). Restoring means re-running backfill.

### 8.3 Existing data invariant

After PR1 deploy on 410:
- Existing `agents` rows default to `tier='free'`
- Existing `geo_events` rows backfilled with `tier_required` from `source` parsing
- No `audit_log` rows (table is empty initially)
- No behavior change for current `agent-keys.txt` callers (all default to free)

## 9. Testing Strategy

### 9.1 Unit

- `Tier`, `LicenseClass` Display + parsing
- `apply_tier_filter` SQL builder (pure function)
- `MonitorMeta` lookup + missing-key panics with helpful message

### 9.2 Integration (Rust)

- `tests/tier_enforcement.rs` (PR1)
- `tests/secrets_redaction.rs` (PR1)
- `tests/payload_metadata.rs` (PR2)
- `tests/quant_history.rs` (PR3)

### 9.3 Acceptance (Python)

- `scripts/accept-sp5.py` extended (3 new checks per PR):
  - PR1: `audit_log has tier_mismatch row`, `free-tier /api/v1/events never returns paid rows`, `agents.tier column populated`
  - PR2: `all geo_events rows have payload.license_class`, `all geo_events rows have payload.data_age_seconds`, `data_age_seconds ≤ documented upstream latency`
  - PR3: `quant_history ≥ 5y per source`, `GET /api/v1/history returns time-series`, `tier-gated correctly`

### 9.4 Compile-time invariant

`tests/metadata_completeness.rs`:
```rust
#[test]
fn every_monitor_has_metadata() {
    let registry: Vec<&str> = collect_registry_names();
    let metadata: Vec<&str> = MONITOR_METADATA.keys().collect();
    assert_eq!(registry.len(), metadata.len());
    for name in &registry {
        assert!(metadata.contains(name), "{name} missing from MONITOR_METADATA");
    }
}
```

## 10. Documentation

- `docs/superpowers/specs/2026-09-15-quant-readability-design.md` (this file)
- `ROADMAP.md` §4 added: OSINT Framework / Quant readout layer (3 PRs, links to spec)
- `AGENTS.md` updated with new lessons:
  - "free-tier agents see only `tier_required='free'` events" — applied at SQL layer
  - "Secrets never in payload" — code-review checklist item
  - "quant_history vs geo_events: events are real-time deltas; history is panel data for backtest" — clarification for downstream consumers

## 11. Open Questions (deferred to v2)

- Tier upgrade flow: self-serve billing page vs manual CLI?
- Per-agent rate limits: needed when paid agents scale up?
- Encrypted-at-rest for paid tier events: do we need it for SaaS rollout?
- Surrogate access pattern: free agent gets a count of paid events (privacy-preserving existence) but no content?
- Cross-tier aggregation: paid agent asks "free + paid count by country" — does that leak paid existence?

## 12. Implementation Plan

After spec approval, invoke `writing-plans` skill to break this into per-PR task lists with LoC estimates, test specs, and acceptance gates. Plans will reference this spec.

## 13. Success Criteria for Sign-Off

PR3 (final PR) is signed off when:
1. All 3 PRs merged to main, pushed to origin
2. 415 + 410 sp5/sp7/sp9 baseline + 9 new checks (3 per PR) all green
3. `cargo test --workspace` clean
4. AGENTS.md + ROADMAP.md updated
5. Demo: free agent queries `/api/v1/events?source=monitor:x` → empty 200, audit_log row created; paid agent queries same → 200 with events