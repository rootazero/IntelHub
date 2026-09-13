# IntelHub — OSINT Intelligence Hub

Agent-agnostic, MCP-native, Docker-first intelligence infrastructure on a single
host: Rust hub-core (MCP gateway / evidence store / event bus / policy & cost
governance / **native monitor signal collectors**), containerized data + sensor
plane, Prometheus/Grafana observability, and a built-in web console with a
global radar map. (SP6: the signal layer moved from the Crucix container into
hub-core itself — 9 built-in collectors + static chokepoint layer.)

## One-line install (Debian/Ubuntu, e.g. a fresh Proxmox VM)

```bash
curl -fsSL https://raw.githubusercontent.com/rootazero/IntelHub/main/scripts/install.sh | bash
```

The installer walks 13 idempotent steps (preflight → fetch code → host
bootstrap → version pins → secrets → **interactive API-key prompts** → hub
build → stack up → crucix build → console build → hub start → agent
provisioning → health check). Every key prompt accepts Enter-to-skip with the
degraded capability stated inline. If anything crashes, **re-run the same
command**: completed steps fast-skip via `~/IntelHub/.install-state`, secrets
are write-once, and the final health check re-validates everything.

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

## Layout

- `hub-core/` — Rust hub (native systemd service, directive §20 exception)
- `compose/` — pinned multi-file compose stack (base/data/sensor/ui)
- `console/` — React 19 + Vite console (zh/en i18n), served by hub-core
- `scripts/` — install.sh plus build/deploy/health/acceptance tooling
- `docs/superpowers/specs/` — design specs per sub-project
- `manifests/` — component registry (versions, upgrade policy)
