# IntelHub — OSINT Intelligence Hub

Agent-agnostic, MCP-native, Docker-first intelligence infrastructure on a single
host: Rust hub-core (MCP gateway / evidence store / event bus / policy & cost
governance / **native monitor signal collectors**), containerized data + sensor
plane, Prometheus/Grafana observability, and a built-in web console with a
global radar map. (SP6: the signal layer moved from the Crucix container into
hub-core itself — 9 built-in collectors + static chokepoint layer.)

## One-line install (Debian/Ubuntu, e.g. a fresh Proxmox VM)

```bash
curl -fsSL __INTELHUB_REPO_URL__/scripts/install.sh | bash
```

> `__INTELHUB_REPO_URL__` is a placeholder until the GitHub repo is published.
> Until then, pre-populate `~/IntelHub` with a checkout of this repo (git
> clone / rsync) and run `bash ~/IntelHub/scripts/install.sh`, or set
> `INTELHUB_TARBALL=<url>` to a tarball URL.

The installer walks 13 idempotent steps (preflight → fetch code → host
bootstrap → version pins → secrets → **interactive API-key prompts** → hub
build → stack up → crucix build → console build → hub start → agent
provisioning → health check). Every key prompt accepts Enter-to-skip with the
degraded capability stated inline. If anything crashes, **re-run the same
command**: completed steps fast-skip via `~/IntelHub/.install-state`, secrets
are write-once, and the final health check re-validates everything.

Useful overrides: `REDO=keys` (re-answer key prompts), `REDO=versions`,
`FORCE=1` (wipe step state), `INTELHUB_NONINTERACTIVE=1`, `INTELHUB_HOME`,
`LAN_IP`, `INTELHUB_CRUCIX_SHA`, `INTELHUB_TARBALL`.

## Layout

- `hub-core/` — Rust hub (native systemd service, directive §20 exception)
- `compose/` — pinned multi-file compose stack (base/data/sensor/ui)
- `console/` — React 19 + Vite console (zh/en i18n), served by hub-core
- `scripts/` — install.sh plus build/deploy/health/acceptance tooling
- `docs/superpowers/specs/` — design specs per sub-project
- `manifests/` — component registry (versions, upgrade policy)
