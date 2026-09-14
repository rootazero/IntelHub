#!/usr/bin/env bash
# update.sh — IntelHub in-place updater.
#
# Entry points:
#   curl -fsSL <repo-url>/scripts/install.sh | bash -s -- update
#   INTELHUB_ENV_FILE=./intelhub.env curl -fsSL <repo-url>/scripts/install.sh \
#     | bash -s -- update
#
# Two tracks:
#   Track A (always): git pull + rebuild hub-core + restart systemd unit +
#                     poll /api/v1/health until 200.
#   Track B (smart):  resolve-versions.sh --dry-run, compare version pins
#                     against compose/.env, upgrade only components whose pin
#                     actually moved. Each bumped component goes through
#                     upgrade.sh (backup → disposable test → promote → health
#                     probe → rollback on failure). SPIDERFOOT / HUGINN
#                     changes are reported with manual rebuild commands.
#
# Overrides (env):
#   INTELHUB_ENV_FILE=<file>  preload new secrets/console-build keys (added
#                              to core/secrets.env + core/console-build.env
#                              if not already present; never overwrites)
#   INTELHUB_HOME=<dir>        install root          (default ~/IntelHub)
#   INTELHUB_NONINTERACTIVE=1  same semantics as install.sh
#   SKIP_BACKUP=1              skip the pre-upgrade backup (NOT recommended)
set -euo pipefail

# ---------------------------------------------------------------- config ----
HUB_DIR="${INTELHUB_HOME:-$HOME/IntelHub}"
SCRIPTS="$HUB_DIR/scripts"
COMPOSE_ENV="$HUB_DIR/compose/.env"
SECRETS="$HUB_DIR/core/secrets.env"
CONSOLE_BUILD_ENV="$HUB_DIR/core/console-build.env"
ENV_FILE="${INTELHUB_ENV_FILE:-}"
NONINTERACTIVE="${INTELHUB_NONINTERACTIVE:-0}"

say()  { printf '\033[1m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[33m    !! %s\033[0m\n' "$*"; }
die()  { printf '\033[31m!! %s\033[0m\n' "$*" >&2; exit 1; }

# -------------------------------------------------------------- preflight ---
[[ -d "$HUB_DIR" ]]     || die "IntelHub not installed at $HUB_DIR (set INTELHUB_HOME)"
[[ -f "$COMPOSE_ENV" ]] || die "compose/.env missing — was IntelHub installed here?"
cd "$HUB_DIR"

# Pre-load env file (same semantics as install.sh). Never overwrites existing
# values; only fills empty slots in secrets.env / console-build.env later.
if [[ -n "$ENV_FILE" ]]; then
  [[ -r "$ENV_FILE" ]] || die "INTELHUB_ENV_FILE=$ENV_FILE unreadable"
  set -a; . "$ENV_FILE"; set +a
  echo "    preloaded $(grep -cE '^[A-Z_]+=' "$ENV_FILE") keys from $ENV_FILE"
fi

# Component → version-var mapping (must mirror upgrade.sh's case statement).
declare -A COMP_VAR=(
  [postgres]=POSTGRES_VERSION [redis]=REDIS_VERSION
  [neo4j]=NEO4J_VERSION [qdrant]=QDRANT_VERSION
  [searxng]=SEARXNG_VERSION [crawl4ai]=CRAWL4AI_VERSION
  [prometheus]=PROMETHEUS_VERSION [node-exporter]=NODE_EXPORTER_VERSION
  [cadvisor]=CADVISOR_VERSION [grafana]=GRAFANA_VERSION
)
DEP_ORDER=(postgres redis neo4j qdrant searxng crawl4ai prometheus node-exporter cadvisor grafana)

# ============================================================ Track A ====
say "Track A: hub-core code refresh"
TRACK_A_STATUS=skipped
if [[ ! -d "$HUB_DIR/.git" ]]; then
  warn "no .git directory at $HUB_DIR (installed from tarball?) — skipping git pull"
  warn "to enable future updates: re-clone via 'git clone https://github.com/rootazero/IntelHub $HUB_DIR'"
else
  if ! git -C "$HUB_DIR" diff --quiet HEAD 2>/dev/null; then
    die "local working tree has uncommitted changes; commit/stash before updating"
  fi
  echo "==> git pull --rebase --autostash"
  git -C "$HUB_DIR" pull --rebase --autostash
fi

echo "==> rebuild hub-core"
bash "$SCRIPTS/build-hub.sh"
sudo chown "$(id -un):$(id -gn)" "$HUB_DIR/core/hub" 2>/dev/null || true

# 2026-09-14: always rebuild the console too, so any out-of-band dist
# (e.g. Mac-built dist rsynced to the VM) gets overwritten with a
# verify-passing build before the user reloads. build-console.sh has
# its own post-build verify (refuses to publish a bundle that's missing
# the keys present in core/console-build.env).
echo "==> rebuild console (matches dist to source + console-build.env keys)"
bash "$SCRIPTS/build-console.sh" || warn "build-console.sh failed — console dist may be stale"

echo "==> restart hub-core.service"
sudo systemctl restart hub-core

LISTEN=$(grep '^HUB_LISTEN_ADDR=' "$HUB_DIR/core/hub.env" 2>/dev/null | cut -d= -f2- | cut -d: -f1)
LISTEN="${LISTEN:-127.0.0.1}"
echo "==> wait for /api/v1/health on $LISTEN:8800"
HEALTHY=
for i in $(seq 1 24); do
  code=$(curl -s -o /dev/null -w '%{http_code}' -m 3 "http://$LISTEN:8800/api/v1/health" 2>/dev/null || echo 000)
  if [[ "$code" =~ ^2 ]]; then
    HEALTHY=1; TRACK_A_STATUS=ok
    echo "    hub-core answering (http $code) after $((i*5))s"
    break
  fi
  sleep 5
done
if [[ -z "$HEALTHY" ]]; then
  sudo journalctl -u hub-core --since "-2min" --no-pager | tail -10 >&2 || true
  die "hub-core did not come up"
fi

# ============================================================ backfills ====
# Post-upgrade one-shot healers (idempotent). Each script exits 0 with
# a no-op message when there's nothing to do. They target specific
# historical data shapes that predate the corresponding migration —
# re-running them on a fully migrated VM is a cheap no-op.
#
# SKIP_BACKFILLS=1 to skip (useful for narrow component-only updates
# that don't touch hub-core schema).

if [[ "${SKIP_BACKFILLS:-0}" != "1" ]]; then
  echo "==> run post-upgrade backfills (idempotent healers)"
  for bf in backfill-claim-audit.py backfill-finding-entities.py; do
    bf_path="$SCRIPTS/$bf"
    if [[ -f "$bf_path" ]]; then
      echo "    $bf"
      if ! python3 "$bf_path" 2>&1 | tail -5; then
        warn "$bf failed — continuing update (backfills are best-effort)"
      fi
    fi
  done
fi

# ============================================================ Track B ====
say "Track B: docker components — smart diff"
TRACK_B_STATUS=skipped
RESOLVED=$(mktemp); trap 'rm -f "$RESOLVED"' EXIT

echo "==> resolve-versions.sh --dry-run"
if ! bash "$SCRIPTS/resolve-versions.sh" --dry-run > "$RESOLVED" 2>/dev/null; then
  warn "resolve-versions.sh --dry-run exited non-zero (registry unreachable?)"
  warn "Track B skipped this run; re-run when registry is back"
else
  declare -A CHANGED=()
  for comp in "${DEP_ORDER[@]}"; do
    var="${COMP_VAR[$comp]}"
    new=$(grep "^${var}=" "$RESOLVED" | head -1 | cut -d= -f2-)
    old=$(grep "^${var}=" "$COMPOSE_ENV" | head -1 | cut -d= -f2-)
    if [[ -n "$new" ]] && [[ "$new" != "$old" ]]; then
      CHANGED[$comp]="$new"
    fi
  done

  if (( ${#CHANGED[@]} == 0 )); then
    echo "==> no version pin changes — Track B cheap path (pull + up)"
    bash "$SCRIPTS/hub-compose.sh" --profile optional pull
    bash "$SCRIPTS/hub-compose.sh" --profile optional up -d
    TRACK_B_STATUS=cheap
  else
    echo "==> version pins changed:"
    for comp in "${!CHANGED[@]}"; do
      old=$(grep "^${COMP_VAR[$comp]}=" "$COMPOSE_ENV" | head -1 | cut -d= -f2-)
      printf "      %-14s %s → %s\n" "$comp" "$old" "${CHANGED[$comp]}"
    done

    # Non-upgrade.sh-managed keys (spiderfoot commit / huginn digest).
    SPECIAL=()
    for k in SPIDERFOOT_IMAGE SPIDERFOOT_COMMIT HUGINN_REF; do
      new=$(grep "^${k}=" "$RESOLVED" | head -1 | cut -d= -f2-)
      old=$(grep "^${k}=" "$COMPOSE_ENV" | head -1 | cut -d= -f2-)
      [[ -n "$new" ]] && [[ "$new" != "$old" ]] && SPECIAL+=("$k")
    done

    if [[ "${SKIP_BACKUP:-0}" != "1" ]]; then
      echo "==> pre-upgrade backup"
      bash "$SCRIPTS/backup.sh"
    else
      warn "SKIP_BACKUP=1 — skipping pre-upgrade backup (NOT recommended)"
    fi

    failed=()
    for comp in "${DEP_ORDER[@]}"; do
      [[ -z "${CHANGED[$comp]:-}" ]] && continue
      new="${CHANGED[$comp]}"
      echo "==> upgrade $comp → $new"
      if ! bash "$SCRIPTS/upgrade.sh" "$comp" "$new"; then
        failed+=("$comp")
        warn "$comp upgrade failed; continuing with remaining components"
        warn "  retry:    bash $SCRIPTS/upgrade.sh $comp $new"
        warn "  rollback: bash $SCRIPTS/rollback.sh $comp"
      fi
    done

    if (( ${#failed[@]} > 0 )); then
      die "Track B failed for: ${failed[*]}"
    fi
    TRACK_B_STATUS="upgraded ${#CHANGED[@]}: ${!CHANGED[*]}"

    if (( ${#SPECIAL[@]} > 0 )); then
      echo
      warn "non-upgrade.sh-managed changes detected: ${SPECIAL[*]}"
      echo "    manual actions:"
      for k in "${SPECIAL[@]}"; do
        case "$k" in
          SPIDERFOOT_*)
            echo "      SPIDERFOOT: bash $SCRIPTS/build-spiderfoot.sh \\"
            echo "                   bash $SCRIPTS/hub-compose.sh --profile optional up -d spiderfoot" ;;
          HUGINN_REF)
            echo "      HUGINN: bash $SCRIPTS/hub-compose.sh --profile optional pull \\"
            echo "             bash $SCRIPTS/hub-compose.sh --profile optional up -d huginn" ;;
        esac
      done
    fi
  fi
fi

# ============================================================ post ====
say "post: reconciling secrets.env"
# Extract the canonical secrets.env heredoc body from install.sh and append
# any KEY= entries that aren't yet in local secrets.env. Idempotent: never
# overwrites existing values.
extract_template() {
  awk '
    /cat > "\$HOME_DIR\/core\/secrets.env" <<.EOF./ { capturing=1; next }
    capturing && /^EOF$/ { exit }
    capturing { print }
  ' "$SCRIPTS/install.sh"
}
NEW_KEYS=()
template=$(extract_template)
if [[ -n "$template" ]]; then
  [[ -f "$SECRETS" ]] || { mkdir -p "$(dirname "$SECRETS")"; touch "$SECRETS"; chmod 600 "$SECRETS"; }
  while IFS= read -r key; do
    [[ -z "$key" ]] && continue
    if ! grep -qE "^${key}=" "$SECRETS"; then
      echo "${key}=" >> "$SECRETS"
      NEW_KEYS+=("$key")
    fi
  done < <(echo "$template" | grep -oE '^[A-Z_][A-Z0-9_]*=' | sed 's/=$//' | sort -u)
  if (( ${#NEW_KEYS[@]} > 0 )); then
    chmod 600 "$SECRETS"
    warn "appended ${#NEW_KEYS[@]} new optional keys to $SECRETS: ${NEW_KEYS[*]}"
    echo "    populate via INTELHUB_ENV_FILE or re-run install interactively"
  else
    echo "    secrets.env template up-to-date"
  fi
fi

# VITE_CARTO_KEY is the one console-build env worth tracking across updates.
[[ -f "$CONSOLE_BUILD_ENV" ]] || {
  mkdir -p "$(dirname "$CONSOLE_BUILD_ENV")"
  printf '# console build-time inputs (0600)\n' > "$CONSOLE_BUILD_ENV"
  chmod 600 "$CONSOLE_BUILD_ENV"
}
if [[ -n "$ENV_FILE" ]] && [[ -n "${VITE_CARTO_KEY:-}" ]] && ! grep -qE '^VITE_CARTO_KEY=' "$CONSOLE_BUILD_ENV"; then
  echo "VITE_CARTO_KEY=${VITE_CARTO_KEY}" >> "$CONSOLE_BUILD_ENV"
  warn "appended VITE_CARTO_KEY to $CONSOLE_BUILD_ENV"
fi

# Final health check
echo "==> health-check.sh"
bash "$SCRIPTS/health-check.sh" || warn "health-check reported warnings; review above"

cat <<EOF

┌──────────────────────────────────────────────────────────────┐
│  IntelHub update complete                                     │
├──────────────────────────────────────────────────────────────┤
│  Track A (hub-core):       ${TRACK_A_STATUS}                 │
│  Track B (docker stack):   ${TRACK_B_STATUS}                 │
└──────────────────────────────────────────────────────────────┘
EOF