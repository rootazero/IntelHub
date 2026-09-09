# IntelHub Substrate — Design Spec (Sub-project 1)

Date: 2026-09-09
Status: Approved by user (2026-09-09)
Supersedes: none. Derived from the "OSINT Intelligence Hub 工程实施指令" (sections 0–19, 45–53 infrastructure parts).

## 0. Scope

Sub-project 1「Substrate」of a 4-sub-project decomposition:

1. **Substrate (this spec)** — PVE VM, Debian 13 hardened host, Docker-first data + sensor infrastructure, lifecycle/backup/upgrade tooling.
2. Hub Core — Rust single-binary: MCP Gateway, Evidence API, ingest/normalization, Event Bus, policy/audit, cost governor.
3. Unified Console — unified intelligence workbench UI.
4. Crucix / Grafana integration — macro-intelligence layer + observability aggregation.

Explicitly out of scope here: Rust Hub Core, MCP Gateway, unified UI, Crucix, Grafana, embedding pipelines.

## 1. Target Environment (verified 2026-09-09)

- PVE40: PVE 9.2.11, kernel 7.0.14-12-pve, 4 physical cores, 31 GB RAM (~20 GB free)
- Storage: `local-lvm` (lvmthin, ~767 GB free); `local` (dir, 73 GB free)
- Network: single bridge `vmbr0`, 10.10.10.0/24, gateway+DNS 10.10.10.1 (OPNsense)
- Existing VMs: 400 SynologyNVR, 401 HomeAssistant. Next free VMID used: 410; template pool starts at 9000.
- Occupied IPs: .1 .2 .8 .10 .11 .20 .30 .40 .100 .113

Resource reality: host cannot meet the directive's recommended 6 vCPU / 24 GB. User approved **option C: 4 vCPU + ballooning (min 8G / max 16G)**, minimum-mode sensor set (SpiderFoot/Huginn deployed but stopped by default; Grafana deferred to SP4).

## 2. VM Provisioning

| Item | Value |
|---|---|
| Template | VMID **9000** `debian-13-cloud`, built from official `debian-13-genericcloud-amd64.qcow2`, SHA256 verified against official CHECKSUM |
| VM | VMID **410**, name `IntelHub`, full clone from 9000 |
| CPU | 4 vCPU, type `host` |
| RAM | 16384 MB, balloon min 8192 MB |
| Disk | 100 GB scsi0 on local-lvm, `discard=on,ssd=1`, virtio-scsi-single |
| NIC | virtio @ vmbr0 |
| Guest agent | qemu-guest-agent enabled (template + guest) |
| Cloud-init | user `zou` (sudo), ssh public key = `debian_ed25519` public part; static `ip=10.10.10.41/24,gw=10.10.10.1`; nameserver `10.10.10.1`; hostname `IntelHub` |

Mac-side ssh config entry `Host IntelHub` (10.10.10.41, user zou, `~/.ssh/debian_ed25519`) per user spec.

## 3. Host Hardening (bootstrap-host.sh)

- apt full-upgrade; install: qemu-guest-agent, ca-certificates, curl, gnupg, jq, htop, unattended-upgrades, nftables, rsync
- Docker from **official Docker repository**: docker-ce, docker-ce-cli, containerd.io, docker-buildx-plugin, docker-compose-plugin (not Debian's docker.io)
- `zou` added to `docker` group
- sshd: `PasswordAuthentication no`, `PermitRootLogin no`, `KbdInteractiveAuthentication no`; service reload (not restart, to protect session)
- journald: `Storage=persistent`
- systemd-timesyncd enabled; unattended-upgrades security updates enabled
- nftables: inet filter table
  - input: default drop; accept lo, established/related, ICMP, tcp/22 from 10.10.10.0/24, tcp published-service ports (7474, 8080, 11235; 5001/3000 when optional profiles active) from 10.10.10.0/24
  - `DOCKER-USER` chain jump: accept established/related; accept 10.10.10.0/24; drop all other forwarded traffic to published container ports
  - rules persisted via `/etc/nftables.conf`, enabled at boot
- Directory init: `/home/zou/IntelHub/{compose,manifests,config,scripts,backups}` (user-mandated path, overrides directive's /opt/intelligence-hub)

## 4. Docker Architecture

Networks (compose.base.yml):

| Network | Type | Members |
|---|---|---|
| mgmt-net | bridge | (reserved for Hub Core / mgmt tooling, SP2+) |
| data-net | bridge, `internal: true` | postgres, redis, neo4j, qdrant |
| sensor-net | bridge | searxng, crawl4ai, spiderfoot*, huginn* |
| egress-net | bridge | searxng, crawl4ai, spiderfoot*, huginn* (only members that need internet) |

\* profile `optional`, stopped by default (minimum mode).

Compose layout at `/home/zou/IntelHub/compose/`:

```
compose.base.yml    # networks, x-common anchors (restart policy, logging)
compose.data.yml    # postgres, redis, neo4j, qdrant
compose.sensor.yml  # searxng, crawl4ai, spiderfoot (profile), huginn (profile)
compose.ui.yml      # reserved (SP3/SP4: grafana etc.)
.env                # pinned versions + credentials (0600, gitignored)
```

Version policy: exact `major.minor.patch` tags in `.env`, image digests recorded in `manifests/` after pull. `latest` forbidden. Each component tracks `production_version` and `last_known_good`.

### Tier 1 — data (compose.data.yml)

| Service | Image (pin strategy) | Memory | Ports |
|---|---|---|---|
| postgres | postgres:17.x (latest stable 17 patch) | 1.5G | none (data-net only) |
| redis | redis:7.x-alpine | 384M | none |
| neo4j | neo4j:5.26.x-community (LTS) | 2G heap-capped | 7474 published, LAN-only (temporary; SP3 closes) |
| qdrant | qdrant/qdrant:v1.x | 2G | none |

All: named volumes on data-net, healthchecks, `restart: unless-stopped`, `no-new-privileges`, log rotation (json-file, 10m x3).

### Tier 2 — sensors (compose.sensor.yml)

| Service | Image | Memory/CPU | Ports |
|---|---|---|---|
| searxng | searxng/searxng:<date-tag> | 768M / 1.0 | 8080 LAN |
| crawl4ai | unclecode/crawl4ai:0.x | 3G / 2.0, concurrency 2 | 11235 LAN |
| spiderfoot | profile `optional` | 512M | 5001 LAN when up |
| huginn | profile `optional` | 768M | 3000 LAN when up |

SearXNG: redis used as its limiter backend is out of scope; standalone with local settings.yml (safe_search off by default, JSON format enabled for API use). Crawl4AI: `MAX_CONCURRENT_PAGES=2`-equivalent config, playwright browsers baked into image.

**Temporary port-exposure deviation (documented):** the directive's end-state is "one entry point: the Hub". Until Hub Core exists (SP2/SP3), sensor native UIs are published on the VM IP restricted to LAN by nftables. SP3 must close these behind the Hub.

## 5. Lifecycle & Ops Tooling

Per-component `manifests/<name>.yml` (directive §6 schema): name, type, deployment, image, version, digest, channel, volumes, networks, resources, healthcheck, backup, upgrade, rollback, native_ui, production_version, last_known_good.

Scripts (`/home/zou/IntelHub/scripts/`):

- `bootstrap-host.sh` — §3, idempotent, run once over ssh from Mac
- `backup.sh` — pg_dumpall (custom format), `neo4j-admin database dump`, qdrant snapshot API, tar of compose/+config/+manifests+/.env; output to `backups/<UTC-ts>/`, retain 7
- `upgrade.sh <component>` — mandated flow: detect → changelog note → compat check → **backup** → pull new image → disposable test instance → health check → integration test → promote (swap .env pin) → monitor. On failure: `rollback.sh`
- `rollback.sh <component>` — restore `.env` to last_known_good, `compose up -d`, verify health
- `health-check.sh` — one-page status: container state, health, versions, resource use, disk

Forbidden: bare `docker compose pull && up -d` as an "upgrade".

## 6. Source Control & Deployment

- Local repo (Mac): `/Volumes/TBU/Workspace/OSINT_Intelligence_Hub`, git; spec under `docs/superpowers/specs/`
- Deploy: rsync repo → `zou@IntelHub:/home/zou/IntelHub` (exclude .git, backups)
- Secrets only in VM-side `.env` (0600); repo carries `.env.example`

## 7. Acceptance Criteria

1. `ssh IntelHub` (zou@10.10.10.41, debian_ed25519) works; password auth refused
2. All compose services report `healthy`
3. Containers attached only to data-net cannot reach the internet (egress curl fails); egress-net members can
4. SearXNG returns JSON results for a test query
5. `backup.sh` completes and produces pg/neo4j/qdrant/config artifacts
6. nftables ruleset loaded and survives reboot (`systemctl is-enabled nftables`)
7. Versions actually deployed recorded into manifests + this spec amendment

## 8. Failure Model Notes (SP1-relevant)

- Any single container failure → `unless-stopped` restart; alert visibility via `health-check.sh` (SP2 adds Event Bus)
- Backup before every upgrade is enforced by `upgrade.sh`, not by convention
- Qdrant/Neo4j loss tolerated by SP1: no irreplaceable data yet; backups exist from first boot
