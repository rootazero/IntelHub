#!/usr/bin/env bash
# resolve-versions.sh — query registries for newest stable tags and write a
# pinned .env on the VM. Run ON the hub host:  bash resolve-versions.sh
# Idempotent: refuses to overwrite an existing .env unless FORCE=1.
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$HUB_DIR/compose/.env"

# --dry-run: print the resolved .env to stdout, skip overwrite guard +
# side effects (postgres init sql render, searxng secret inject). Used
# by update.sh to compute "what would change" without touching anything.
DRY_RUN=0
if [[ "${1:-}" == "--dry-run" ]]; then DRY_RUN=1; shift || true; fi

if [[ "$DRY_RUN" != "1" && -f "$ENV_FILE" && "${FORCE:-0}" != "1" ]]; then
  echo ".env already exists ($ENV_FILE); set FORCE=1 to regenerate" >&2
  exit 1
fi

hub_api() { curl -fsSL "https://hub.docker.com/v2/repositories/$1/tags?page_size=100" ; }

# newest tag matching an extended-regex from Docker Hub (library/* repos)
latest_tag() { # $1=repo  $2=ERE filter
  hub_api "$1" | jq -r '.results[].name' | grep -E "$2" | sort -V | tail -1
}

echo "==> resolving tags from Docker Hub..."
POSTGRES_VERSION=$(latest_tag library/postgres '^17\.[0-9]+(\.[0-9]+)?-trixie$')
REDIS_VERSION=$(latest_tag library/redis '^7\.[0-9]+\.[0-9]+-alpine$')
NEO4J_VERSION=$(latest_tag library/neo4j '^5\.26\.[0-9]+-community$')
QDRANT_VERSION=$(latest_tag qdrant/qdrant '^v1\.[0-9]+\.[0-9]+$')
CRAWL4AI_VERSION=$(latest_tag unclecode/crawl4ai '^[0-9]+\.[0-9]+\.[0-9]+$')
# SearXNG tags are date-based: 2025.9.18-<sha>
SEARXNG_VERSION=$(hub_api searxng/searxng | jq -r '.results[].name' | grep -E '^20[0-9]{2}\.[0-9]+\.[0-9]+-[0-9a-f]+$' | sort -V | tail -1)

for v in POSTGRES_VERSION REDIS_VERSION NEO4J_VERSION QDRANT_VERSION CRAWL4AI_VERSION SEARXNG_VERSION; do
  if [[ -z "${!v}" ]]; then echo "ERROR: failed to resolve $v" >&2; exit 1; fi
  echo "    $v=${!v}"
done

echo "==> resolving optional-profile images..."
# SpiderFoot: NO official prebuilt image exists (hub 404 / ghcr 403; upstream
# ships a Dockerfile for local build). Pin upstream commit; the image is built
# locally on the hub by scripts/build-spiderfoot.sh — keeps the component
# Dockerized, versioned, and reproducible without trusting third-party images.
SPIDERFOOT_COMMIT=$(git ls-remote https://github.com/smicallef/spiderfoot.git refs/heads/master | awk '{print $1}')
[[ -z "$SPIDERFOOT_COMMIT" ]] && { echo "ERROR: could not resolve spiderfoot commit" >&2; exit 1; }
SPIDERFOOT_IMAGE="intelhub/spiderfoot:${SPIDERFOOT_COMMIT:0:12}"
echo "    SPIDERFOOT_IMAGE=$SPIDERFOOT_IMAGE (local build from upstream commit)"

# Huginn: pin current :latest by digest (upstream does not publish semver tags)
HUB_TOKEN=$(curl -fsSL "https://auth.docker.io/token?service=registry.docker.io&scope=repository:huginn/huginn:pull" | jq -r .token)
HUGINN_DIGEST=$(curl -fsSI -H "Authorization: Bearer $HUB_TOKEN" -H "Accept: application/vnd.docker.distribution.manifest.list.v2+json" \
  https://registry-1.docker.io/v2/huginn/huginn/manifests/latest | grep -i docker-content-digest | awk '{print $2}' | tr -d '\r')
[[ -z "$HUGINN_DIGEST" ]] && { echo "ERROR: could not resolve huginn digest" >&2; exit 1; }
echo "    HUGINN_REF=@${HUGINN_DIGEST}"

rand() { openssl rand -hex 24; }

# 2026-09-14: detect stale unexpanded `$(...)` command-substitution templates
# in compose/.env. Earlier bug: a hand-edited file left
# `GRAFANA_ADMIN_PASSWORD=$(cat /proc/sys/kernel/random/uuid)` as a literal
# string (docker-compose doesn't expand `$(...)` in env files), so the running
# grafana container used a different password than compose/.env claimed. The
# fix here auto-regenerates any *PASSWORD / *TOKEN / *KEY field whose RHS is
# an unexpanded `$(...)` template. Idempotent — re-runs are no-ops on a clean
# file. Returns 1 if any unfixable templates remain (operator must intervene).
verify_templates() {
  local file="$ENV_FILE"
  [[ -f "$file" ]] || return 0

  # Find lines matching KEY=$(...) where the body is anything but empty / rand.
  # Excludes comment lines and the heredoc-friendly `$(date ...)` etc.
  # Regex notes:
  #   - `\$\(` matches the literal `$(` (open paren of command substitution).
  #   - The KEY name allows digits: `CRAWL4AI_API_TOKEN` is a valid match,
  #     so the prefix class is `[A-Z0-9_]+` not just `[A-Z_]+`.
  #   - In ERE (`grep -E` / `sed -E`) parens do not need escaping in patterns,
  #     BUT `\$` is still required to match a literal `$`. Using `\(` `\)`
  #     in ERE will silently fail to match — see verify_templates test.
  #   - Body matches anything up to end of line.
  local hits
  hits=$(grep -nE '^[[:space:]]*[A-Z0-9_]+_(PASSWORD|TOKEN|KEY)=.*\$\(' "$file" 2>/dev/null \
         | grep -vE '=\$\(rand\)$' || true)
  [[ -z "$hits" ]] && return 0

  echo "WARN: stale command-substitution templates in $file (docker-compose will not expand \$()):" >&2
  echo "$hits" >&2
  echo "    auto-regenerating with openssl rand..." >&2

  # Rewrite each stale RHS to $(rand) so the next heredoc-pass will expand it.
  # NB: this sed is intentionally conservative — only KEY/PASSWORD/TOKEN
  # fields whose RHS contains a $( pattern. Avoids touching data-bearing
  # fields like `REDIS_URL=redis://:...@...` (which have no $). Uses '#'
  # as the delimiter because the pattern contains slashes (e.g. $(date +%s)).
  # ERE caveats (see comment above): allow digits in name; use `\(` not
  # `\(`, and bare `)` not `\)`.
  sed -i -E 's#^([A-Z0-9_]+_(PASSWORD|TOKEN|KEY))=\$\(.*\)#\1=$(rand)#' "$file"

  # Re-check: anything still stale = an unknown pattern we can't fix.
  local remain
  remain=$(grep -nE '^[[:space:]]*[A-Z0-9_]+_(PASSWORD|TOKEN|KEY)=.*\$\(' "$file" 2>/dev/null \
           | grep -vE '=\$\(rand\)$' || true)
  if [[ -n "$remain" ]]; then
    echo "ERROR: unknown stale templates remain in $file after auto-fix:" >&2
    echo "$remain" >&2
    echo "       edit manually (replace \$() with a real value) and re-run." >&2
    return 1
  fi
  echo "    fix applied (templates regenerated)." >&2
  return 0
}

# LAN IP: env override, else auto-detect the primary IPv4 source address.
LAN_IP="${LAN_IP:-$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src"){print $(i+1); exit}}')}"
[[ -z "$LAN_IP" ]] && { echo "ERROR: could not detect LAN IP; set LAN_IP=x.x.x.x" >&2; exit 1; }
echo "    LAN_IP=$LAN_IP"

# SP4 observability pins: validated versions, env-overridable.
# (Not live-resolved: cadvisor lives on gcr.io, grafana tag scheme differs —
# deterministic pins match the directive's version-pinning requirement.)
PROMETHEUS_VERSION="${PROMETHEUS_VERSION:-v3.14.0}"
NODE_EXPORTER_VERSION="${NODE_EXPORTER_VERSION:-v1.12.1}"
CADVISOR_VERSION="${CADVISOR_VERSION:-v0.55.1}"
GRAFANA_VERSION="${GRAFANA_VERSION:-13.2.1}"

if [[ "$DRY_RUN" == "1" ]]; then
  # Read-only check in dry-run mode: warn but never sed.
  if [[ -f "$ENV_FILE" ]]; then
    dry_hits=$(grep -nE '^[[:space:]]*[A-Z0-9_]+_(PASSWORD|TOKEN|KEY)=.*\$\(' "$ENV_FILE" 2>/dev/null \
               | grep -vE '=\$\(rand\)$' || true)
    if [[ -n "$dry_hits" ]]; then
      echo "WARN (dry-run): stale templates detected in $ENV_FILE (would auto-fix in write mode):" >&2
      echo "$dry_hits" >&2
    fi
  fi
  echo "# (dry-run; would write $ENV_FILE)" >&2
  cat <<EOF
# Generated by resolve-versions.sh on $(date -u +%Y-%m-%dT%H:%M:%SZ)
LAN_IP=${LAN_IP}

POSTGRES_VERSION=${POSTGRES_VERSION}
POSTGRES_USER=intelhub
POSTGRES_PASSWORD=$(rand)
POSTGRES_DB=intelhub

REDIS_VERSION=${REDIS_VERSION}
REDIS_PASSWORD=$(rand)

NEO4J_VERSION=${NEO4J_VERSION}
NEO4J_PASSWORD=$(rand)

QDRANT_VERSION=${QDRANT_VERSION}

SEARXNG_VERSION=${SEARXNG_VERSION}
CRAWL4AI_VERSION=${CRAWL4AI_VERSION}
CRAWL4AI_API_TOKEN=$(rand)

SPIDERFOOT_IMAGE=${SPIDERFOOT_IMAGE}
SPIDERFOOT_COMMIT=${SPIDERFOOT_COMMIT}
HUGINN_REF=@${HUGINN_DIGEST}
HUGINN_DB_PASSWORD=$(rand)

# SP4 observability stack (deterministic pins, env-overridable)
PROMETHEUS_VERSION=${PROMETHEUS_VERSION}
NODE_EXPORTER_VERSION=${NODE_EXPORTER_VERSION}
CADVISOR_VERSION=${CADVISOR_VERSION}
GRAFANA_VERSION=${GRAFANA_VERSION}
GRAFANA_ADMIN_PASSWORD=$(rand)
EOF
else
  verify_templates || { echo "aborting; fix compose/.env manually or delete it." >&2; exit 1; }
  echo "==> writing $ENV_FILE"
  cat > "$ENV_FILE" <<EOF
# Generated by resolve-versions.sh on $(date -u +%Y-%m-%dT%H:%M:%SZ)
LAN_IP=${LAN_IP}

POSTGRES_VERSION=${POSTGRES_VERSION}
POSTGRES_USER=intelhub
POSTGRES_PASSWORD=$(rand)
POSTGRES_DB=intelhub

REDIS_VERSION=${REDIS_VERSION}
REDIS_PASSWORD=$(rand)

NEO4J_VERSION=${NEO4J_VERSION}
NEO4J_PASSWORD=$(rand)

QDRANT_VERSION=${QDRANT_VERSION}

SEARXNG_VERSION=${SEARXNG_VERSION}
CRAWL4AI_VERSION=${CRAWL4AI_VERSION}
CRAWL4AI_API_TOKEN=$(rand)

SPIDERFOOT_IMAGE=${SPIDERFOOT_IMAGE}
SPIDERFOOT_COMMIT=${SPIDERFOOT_COMMIT}
HUGINN_REF=@${HUGINN_DIGEST}
HUGINN_DB_PASSWORD=$(rand)

# SP4 observability stack (deterministic pins, env-overridable)
PROMETHEUS_VERSION=${PROMETHEUS_VERSION}
NODE_EXPORTER_VERSION=${NODE_EXPORTER_VERSION}
CADVISOR_VERSION=${CADVISOR_VERSION}
GRAFANA_VERSION=${GRAFANA_VERSION}
GRAFANA_ADMIN_PASSWORD=$(rand)
EOF
  chmod 600 "$ENV_FILE"
fi

if [[ "$DRY_RUN" == "1" ]]; then
  echo "==> dry-run complete (no file written, no side effects applied)"
else
  # Render postgres init sql with the huginn password
  sed "s/@HUGINN_DB_PASSWORD@/$(grep '^HUGINN_DB_PASSWORD=' "$ENV_FILE" | cut -d= -f2)/" \
    "$HUB_DIR/config/postgres/init/01-huginn.sql.tmpl" > "$HUB_DIR/config/postgres/init/01-huginn.sql"

  # SearXNG refuses the default "ultrasecretkey" — inject a real random secret
  sed -i "s/secret_key: \"ultrasecretkey\"/secret_key: \"$(rand)\"/" \
    "$HUB_DIR/config/searxng/settings.yml"

  echo "==> done. .env written (0600), huginn init sql rendered, searxng secret injected."
fi
