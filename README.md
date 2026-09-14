# IntelHub — OSINT Intelligence Hub

Agent-agnostic, MCP-native, Docker-first OSINT infrastructure on a single host.
A Rust hub-core exposes the entire signal plane through one Model Context
Protocol gateway: an evidence store, event bus, policy & cost governance,
native signal collectors, a built-in web console with a global radar map,
and a Prometheus/Grafana observability stack.

---

## What is IntelHub?

IntelHub turns a fresh Debian/Ubuntu VM into a self-contained OSINT workbench
in one curl command. It ingests from open sources (web crawls, social
listings, OSINT monitors, market / climate / sanctions data), deduplicates and
ranks what it finds, stores it with full evidence chains, and serves it back
through MCP tools that any agent (pi, codex, custom) can call. The web console
gives a human the same view: a global radar map, an investigation workbench,
a graph canvas, and live activity streams.

Everything runs on one host. No SaaS dependencies. The whole stack is
containerized except `hub-core` itself, which is a native systemd unit (the
single non-container service).

## Key features

- **One-line install** — `curl … | bash` on a fresh Debian/Ubuntu VM gets a
  fully-working OSINT stack in ~10 minutes. Crash-safe: re-running the same
  command resumes where you left off.
- **MCP-native gateway** — every capability is exposed as an MCP tool. Any
  MCP-compatible agent can call `hybrid_search`, `investigate`, `crawl_url`,
  `create_claim`, etc. without bespoke integration. Every MCP call threads
  a UUID `trace_id` through `cost_records.trace_id`, `embedding_jobs.trace_id`,
  and the response's top-level `trace_id` field. Walk the full call graph
  (tool audit rows, embedding jobs, rerank token costs) via
  `GET /api/v1/traces/{trace_id}`.
- **Native signal collectors** — climate (EONET, NOAA), radiation (EPA
  RadNet), seismic (USGS), financial (FRED, Treasury, Finnhub, Comtrade, EIA),
  sanctions (OFAC, USASpending), threats (ACLED, GDELT, Bluesky, Telegram,
  X/Twitter, RSS), web (SearXNG, Crawl4AI). 25+ collectors, built into
  hub-core itself.
- **Evidence + audit trail** — every claim, finding, document, and source is
  traceable. Claims have audit rows, documents have reverse refs, findings
  have evidence chains. The audit log is append-only.
- **Cost & rate governance** — per-agent rate limits (Redis token bucket),
  per-agent budgets (token / tool-call / embed), policy levels (admin token
  required for L3 actions like component backups).
- **Built-in web console** — React 19 + Vite, zh/en i18n. Includes a global
  radar map with dark basemaps (Stadia primary → Esri fallback → CARTO last),
  an investigation workbench, a knowledge graph canvas (filters + side
  panels), live activity streams, audit/search/overview pages.
- **Observability** — Prometheus + Grafana + cAdvisor + node-exporter,
  scraping hub-core's `/metrics` and the docker stack. Pre-built Grafana
  dashboards for monitor sweep history, budget burn, graph mirror
  reconcile, cost records.
- **Knowledge graph (Neo4j)** — entities / claims / findings mirrored to
  Neo4j for graph queries (path finding, neighbors, subgraph extraction).
  Reconcile worker keeps PG and Neo4j in sync.
- **Hybrid search** — keyword (BM25 via Qdrant) + semantic (embeddings via
  T8star / OpenAI-compatible endpoint), fused via Reciprocal Rank Fusion,
  optional cross-encoder rerank (BAAI/bge-reranker-v2-m3, gated by
  `HUB_RERANK_ENABLED`). Multi-hop Q&A via the `investigate(question)` tool
  with rule-based + LLM-fallback planners. Repeat calls are served from a
  Redis result cache keyed by `(mode, query, limit, url_contains)` with a
  300 s TTL — 590× speedup on a hit (7.1 s → 12 ms). Every search response
  carries a top-level `cache: "hit" | "miss" | "disabled" | "error"` field;
  toggle with `HUB_QUERY_CACHE_ENABLED` (default `true`).
- **Crash-safe install + update** — every step is idempotent and recorded in
  `~/IntelHub/.install-state`. Re-running the install command fast-skips
  completed steps and resumes at the failure point. Secrets are write-once:
  no re-run can clobber generated keys.

## Architecture (one-line mental model)

```
Internet ──► native collectors ──► hub-core (Rust, systemd) ──► MCP tools
                                       │                       │
                                       ▼                       ▼
                            Postgres + Redis + Neo4j       pi / codex
                            + Qdrant (vectors)             web console
                                       │
                                       ▼
                          Prometheus ─► Grafana dashboards
```

| Layer | Component | Role |
|---|---|---|
| Gateway | `hub-core` (Rust, native) | MCP server, REST API, auth, rate limits, policy, audit, cost tracking, native signal collectors |
| Storage | Postgres + Redis + Neo4j + Qdrant | evidence store, rate-limit buckets, knowledge graph, vector search |
| Sensors | SearXNG, Crawl4AI, SpiderFoot, Huginn | web search / crawl / OSINT bridges |
| Observability | Grafana + Prometheus + cAdvisor + node-exporter | metrics, dashboards, host/container stats |
| UI | `console/` (React 19 + Vite) | web console, served as static files by hub-core |

The detailed spec — every component's role, schema, SP milestone, and
deployment topology — lives in [`OSINTIntelligenceHub.md`](OSINTIntelligenceHub.md).

---

## One-line install (Debian or RHEL family, e.g. a fresh Proxmox VM)

```bash
curl -fsSL https://raw.githubusercontent.com/rootazero/IntelHub/main/scripts/install.sh | bash
```

The installer walks 12 idempotent steps:

1. **preflight** — check OS / docker / disk / RAM
2. **fetch-code** — `git clone` (or tarball via `INTELHUB_TARBALL=…`)
3. **bootstrap** — nftables LAN rules, ntp, sysctl
4. **versions** — resolve pinned versions per component
5. **secrets** — write `core/secrets.env`, `core/hub.env`, `compose/.env`
6. **keys** — **interactive API-key prompts** via `/dev/tty` (each prompts
   its feature + a stated "Enter-to-skip" degraded capability)
7. **build-hub** — compile Rust binary in a pinned `rust:trixie` container
8. **stack-up** — `docker compose up -d` for data + sensor + ui layers
9. **build-console** — `npm run build` in `node:22-trixie`, embeds map keys
10. **start-hub** — enable + start `hub-core.service`
11. **provision-agents** — mint agent + console API keys into
    `core/agent-keys.txt`
12. **verify** — health check (marks step undone on failure so a re-run
    resumes there)

Every step is recorded in `~/IntelHub/.install-state`. **Re-run the same
command to resume**: completed steps fast-skip, secrets stay write-once, the
final health check re-validates everything.

### Unattended install (CI / Terraform / pre-provisioned)

Copy [`examples/intelhub.env.example`](examples/intelhub.env.example), uncomment
the keys you have, fill in real values, then:

```bash
curl -fsSL https://raw.githubusercontent.com/rootazero/IntelHub/main/scripts/install.sh \
  | INTELHUB_ENV_FILE=$PWD/intelhub.env bash
```

Missing keys in the env file still prompt via `/dev/tty`. Set
`INTELHUB_NONINTERACTIVE=1` alongside to skip every prompt silently
(corresponding features degrade; nothing aborts).

## One-line update (in-place upgrade of hub-core + docker stack)

```bash
curl -fsSL https://raw.githubusercontent.com/rootazero/IntelHub/main/scripts/install.sh \
  | bash -s -- update
```

Two tracks run in sequence:

- **Track A — hub-core code:** `git pull` → rebuild Rust binary → restart
  `hub-core.service` → poll `/api/v1/health` until 200.
- **Track B — docker stack (smart diff):** run `resolve-versions.sh --dry-run`,
  compare version pins against `compose/.env`. If nothing changed: cheap path
  (`compose pull && up -d`). If a pin moved: run `backup.sh` once, then loop
  `upgrade.sh` per changed component with backup → disposable test → promote
  → health probe → rollback on failure.

Newly-introduced optional secrets (added to `install.sh` between updates) are
auto-merged into existing `core/secrets.env` as empty entries — populate via
`INTELHUB_ENV_FILE` on the next update. SpiderFoot commit / Huginn digest
changes are reported with manual rebuild commands.

Override knobs: `SKIP_BACKUP=1` (skip the pre-upgrade backup, not recommended),
`INTELHUB_ENV_FILE` (same semantics as install), `INTELHUB_HOME`,
`INTELHUB_NONINTERACTIVE`.

---

## API key management

IntelHub mints two API keys on first install:

- **agent key** — used by MCP clients (pi, codex, custom agents) to call
  tools. Show this in the install banner.
- **console key** — used by the web console UI to talk to hub-core. Show
  this in the install banner.

Both keys are written to `core/agent-keys.txt` (mode `0600`), format
`ihk_<64 hex chars>`. They are printed once in the install banner — **copy
them now**, the banner is not re-shown.

### Forgot a key? Rotate it.

```bash
bash scripts/reset-key.sh agent      # rotate the agent key
bash scripts/reset-key.sh console    # rotate the console key
bash scripts/reset-key.sh all        # rotate both
```

What this does:

- Soft-revokes the old key in PG `api_keys` (kept for audit, queryable via
  the DB). The previous key stops working **immediately** — hub-core hashes
  and looks up keys on every request, so no restart is needed.
- Mints a fresh key with the same `agent_id` (so MCP client identity is
  preserved).
- Updates `core/agent-keys.txt` atomically (write to sibling tempfile +
  `os.replace`).
- Prints the new key + a marked "revoked" old key.

The script refuses to run without confirmation (or `INTELHUB_NONINTERACTIVE=1`).
You can also invoke via the installer: `bash scripts/install.sh reset-key agent`.

### Updates don't touch keys

`step_provision_agents` is idempotent: if a key with that name already
exists in `core/agent-keys.txt`, the step skips. Re-running `install.sh` or
`bash scripts/install.sh update` will **never** rotate keys on its own.

---

## Install + update overrides (cheat sheet)

| Override | Effect |
|---|---|
| `REDO=keys` | re-run the interactive key prompts (install) |
| `REDO=versions` | regenerate `compose/.env` pinned tags |
| `FORCE=1` | wipe `.install-state`; re-run every step (secrets stay write-once) |
| `INTELHUB_NONINTERACTIVE=1` | skip every key prompt silently |
| `INTELHUB_HOME=<dir>` | install root (default `~/IntelHub`) |
| `INTELHUB_LAN=<cidr>` | LAN CIDR for nftables (default `10.10.10.0/24`) |
| `INTELHUB_TARBALL=<url>` | fetch code as tarball instead of `git clone` |
| `INTELHUB_ENV_FILE=<file>` | preload keys before prompts (install or update) |
| `SKIP_BACKUP=1` | update only — skip pre-upgrade backup (NOT recommended) |
| `bash scripts/reset-key.sh <agent\|console\|all>` | rotate API keys |

---

## System requirements

- **OS**: any Linux on x86_64 — both major families:
  - **Debian family**: Debian 12+ (bookworm/trixie), Ubuntu 22.04+, Linux Mint,
    Pop!_OS, Elementary, Kali, Raspbian, and any other distro whose `ID_LIKE`
    contains `debian`.
  - **RHEL family**: RHEL 8+/9, CentOS Stream 8+/9, Rocky Linux 8+/9, AlmaLinux
    8+/9, Fedora 36+, Amazon Linux 2023+, Oracle Linux, and any other distro
    whose `ID_LIKE` contains `rhel` or `fedora`.

  The install script auto-detects via `/etc/os-release` and adapts the
  package manager (`apt-get` vs `dnf`/`yum`), Docker repo paths, and
  unattended-upgrade mechanism (`unattended-upgrades` vs `dnf-automatic`).
  Both families produce the same end state — a working `hub-core` + Docker
  stack on a fixed LAN IP. Use `INTELHUB_FORCE_OS=1` to bypass detection
  (unsupported distros / future families).
- **Hardware**: 4 vCPU / 8 GB RAM minimum (a Proxmox VM is the reference
  shape). Plan for 16 GB if you'll be running heavy embedding / crawl jobs.
- **Disk**: ~20 GB for the OS + docker stack + raw data + manifests.
  Crawled HTML and snapshots grow fast — provision a separate data volume
  if you intend to crawl heavily.
- **Network**: outbound HTTPS to GitHub, Docker Hub, SearXNG upstream
  engines, T8star / OpenAI-compatible embedding endpoint, optional
  third-party data APIs (FRED, EIA, BLS, etc.).
- **LAN**: a fixed IPv4 address on the LAN CIDR (the install script sets
  up nftables rules allowing only this CIDR to reach hub-core's 8800 +
  Grafana's 3001). Override with `INTELHUB_LAN`.

## Layout

- `hub-core/` — Rust hub (native systemd service, directive §20 exception)
- `compose/` — pinned multi-file compose stack (base/data/sensor/ui)
- `console/` — React 19 + Vite console (zh/en i18n), served by hub-core
- `scripts/` — install.sh, update.sh, reset-key.sh, plus build/deploy/
  health/acceptance tooling
- `manifests/` — component registry (versions, upgrade policy)
- `OSINTIntelligenceHub.md` — full project spec (1700+ lines: every
  component, schema, SP milestone, deployment topology)
- `examples/` — `intelhub.env.example` for unattended install
- `docs/superpowers/specs/` — per-sub-project design docs

## Documentation

- Full project spec: [`OSINTIntelligenceHub.md`](OSINTIntelligenceHub.md)
- Per-feature design docs: [`docs/superpowers/specs/`](docs/superpowers/specs/)
- 中文版 README: [`README.zh.md`](README.zh.md)

## Operating

Operational tooling that ships with the repo. All live in `scripts/`. Run
them from any host with the agent key — the dev Mac (over ssh) or the VM
itself (locally).

### Run the acceptance suite

```bash
KEY=$(ssh -o BatchMode=yes IntelHub 'grep "api_key:" ~/IntelHub/core/agent-keys.txt | head -1 | grep -o "ihk_[a-f0-9]*"')
for a in sp3 sp6 sp7 sp8 sp9 sp10; do
  echo "== $a =="; python3 scripts/accept-$a.py "$KEY" 2>&1 | tail -1
done
```

Each script prints `== N passed, K shelved, M failed ==`. `shelved`
(sp5/6/7) covers missing-API-key scenarios — the exit code only reflects
`failed`, so shelved checks do not break CI. Override the SSH target with
`INTELHUB_SSH=<alias>`. The suite auto-detects `$INTELHUB_HOME/core/hub`
and falls back to local execution, so the same scripts run on the deploy
host and your laptop.

### Custom scripts — use `scripts/_remote.py`

Don't hand-roll `subprocess.run(["ssh", ...])`. Import the helper:

```python
from _remote import (
    sh, pg, pg_stdin, pg_params,   # shell + Postgres
    redis, cypher,                 # Redis + Neo4j
    grafana_creds, grafana_request, # Grafana
)
```

It auto-routes ssh-or-local via the `$INTELHUB_HOME/core/hub` sentinel —
no `INTELHUB_LOCAL` switch needed. Auth for Redis / Neo4j / Grafana is
extracted from `compose/.env` automatically. Use `pg_params` only for test
fixtures with controlled input; untrusted input goes through `pg_stdin`.

### Backfill the Neo4j graph mirror after schema changes

Idempotent — safe to re-run any time a migration touches entities,
findings, claims, or documents:

```bash
python3 scripts/backfill-neo4j-graph-mirror.py    # entities / relationships / documents / findings / claim_evidence / finding_evidence
python3 scripts/backfill-neo4j-entity-ids.py      # orphan entity_id=NULL fix
python3 scripts/backfill-finding-entities.py
python3 scripts/backfill-claim-audit.py
```

### Set or rotate the dark-map basemap key

```bash
bash scripts/set-dark-map-key.sh
```

Auto-detects Stadia (UUID-shaped) vs CARTO (`cb1_`-prefixed) keys and
writes both `compose/.env` and `console-build.env`. Stadia is preferred
when both are present. A red "DARK MAP KEY MISSING" badge in the console
indicates neither is set. Rebuild the console
(`bash scripts/build-console.sh`) for the change to take effect in the UI.

### Update `compose/.env` without re-running the full installer

If a manual edit left unexpanded `$(...)` templates in `compose/.env`
(docker-compose does not expand `$(...)` — a common foot-gun), `update.sh`
will auto-detect and regenerate them via `verify_templates()`. The
underlying script is `resolve-versions.sh` (read `FORCE=1 bash
scripts/resolve-versions.sh` to regenerate the whole file from current
pinned tags).