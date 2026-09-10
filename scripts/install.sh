#!/usr/bin/env bash
# install.sh — IntelHub one-line installer.
#
#   curl -fsSL <repo-raw-url>/scripts/install.sh | bash
#
# Runs ON the target Debian/Ubuntu machine (a Proxmox VM is the reference
# shape). Interactive API-key prompts read from /dev/tty; press Enter to skip
# any key (the corresponding feature degrades, install continues).
#
# Crash-safe: every step is idempotent AND recorded in $HOME/.install-state
# equivalent ($INTELHUB_HOME/.install-state). Re-running the same command
# fast-skips completed steps and resumes at the failure point. No re-run can
# clobber generated secrets (they are write-once).
#
# Overrides (env):
#   INTELHUB_HOME=<dir>        install root          (default ~/IntelHub)
#   INTELHUB_TARBALL=<url>     fetch code as tarball instead of git clone
#   INTELHUB_LAN=<cidr>        LAN CIDR for nftables (default 10.10.10.0/24)
#   LAN_IP=<ip>                skip auto-detection
#   INTELHUB_NONINTERACTIVE=1  skip all key prompts
#   REDO=<step[,step]>         force re-run of named step(s)
#   FORCE=1                    wipe state, re-run every step (secrets still
#                              write-once — never regenerated)
set -euo pipefail

# ---------------------------------------------------------------- config ----
# Placeholder until the GitHub repo is created; INTELHUB_TARBALL bypasses it.
REPO_URL="__INTELHUB_REPO_URL__"
REPO_BRANCH="main"

HOME_DIR="${INTELHUB_HOME:-$HOME/IntelHub}"
STATE="$HOME_DIR/.install-state"
NONINTERACTIVE="${INTELHUB_NONINTERACTIVE:-0}"
REDO="${REDO:-}"
FORCE="${FORCE:-}"
RUN_USER="$(id -un)"
CHANGED_HUB_ENV=0

say()  { printf '\033[1m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[33m    !! %s\033[0m\n' "$*"; }
die()  { printf '\033[31m!! %s\033[0m\n' "$*" >&2; exit 1; }

# ------------------------------------------------------- state machinery ----
done_step()   { grep -qxF "$1" "$STATE" 2>/dev/null; }
mark_done()   { grep -qxF "$1" "$STATE" 2>/dev/null || echo "$1" >> "$STATE"; }
mark_undone() { [[ -f "$STATE" ]] && sed -i "/^$1\$/d" "$STATE" || true; }
want_step() { # REDO=a,b or FORCE=1 → true even if recorded done
  [[ "$FORCE" == "1" ]] && return 0
  [[ ",$REDO," == *",$1,"* ]] && return 0
  ! done_step "$1"
}
run_step() { # run_step <name> <function>
  local name="$1" fn="$2"
  if ! want_step "$name"; then
    echo "── [$name] done, skipping (REDO=$name to force)"
    return 0
  fi
  say "[$name]"
  if "$fn"; then
    mark_done "$name"
  else
    local rc=$?
    mark_undone "$name"
    die "step [$name] failed (rc=$rc). Fix the cause, then re-run the SAME command — completed steps are skipped automatically."
  fi
}

# upsert KEY=VALUE in an env file (append if absent, replace in place if not)
upsert_env() { # upsert_env <file> <KEY> <value>
  local file="$1" key="$2" val="$3"
  if grep -q "^${key}=" "$file" 2>/dev/null; then
    sed -i "s|^${key}=.*|${key}=${val}|" "$file"
  else
    echo "${key}=${val}" >> "$file"
  fi
}
get_env() { grep "^$2=" "$1" 2>/dev/null | head -1 | cut -d= -f2-; }

# interactive prompt via /dev/tty (stdin is the curl pipe); empty = skip
prompt_key() { # prompt_key <VAR> <description> <skip-consequence> → echoes value or ""
  local var="$1" desc="$2" consequence="$3" val=""
  if [[ "$NONINTERACTIVE" == "1" ]]; then return 0; fi
  echo
  echo "    ── $var: $desc"
  echo "       跳过后果: $consequence"
  printf '       输入值（回车跳过）: ' >/dev/tty
  read -r val </dev/tty || true
  printf '%s' "$val"
}

# ------------------------------------------------------------------ steps ---
step_preflight() {
  [[ -r /etc/os-release ]] || die "cannot identify OS"
  . /etc/os-release
  case "${ID:-}" in
    debian|ubuntu) ;;
    *) die "unsupported OS '${ID:-?}' — IntelHub targets Debian/Ubuntu (set INTELHUB_FORCE_OS=1 to override)" ;;
  esac
  command -v curl  >/dev/null || die "curl missing: apt-get install -y curl"
  command -v git   >/dev/null || warn "git not installed yet — bootstrap will install it"
  command -v openssl >/dev/null || die "openssl missing"
  sudo -v  # cache credentials; bootstrap needs passwordless or cached sudo
  mkdir -p "$HOME_DIR"
  touch "$STATE" && chmod 600 "$STATE"
  echo "    os=${PRETTY_NAME:-$ID} user=$RUN_USER home=$HOME_DIR"
}

step_fetch_code() {
  if [[ -f "$HOME_DIR/scripts/hub-compose.sh" && -d "$HOME_DIR/compose" ]]; then
    echo "    code already present at $HOME_DIR"
    return 0
  fi
  if [[ -n "${INTELHUB_TARBALL:-}" ]]; then
    say "fetching tarball: $INTELHUB_TARBALL"
    mkdir -p "$HOME_DIR"
    curl -fsSL "$INTELHUB_TARBALL" | tar xz --strip-components=1 -C "$HOME_DIR"
  elif [[ "$REPO_URL" == "__INTELHUB_REPO_URL__" ]]; then
    die "no code source: repo URL is a placeholder. Either pre-populate $HOME_DIR (git clone/rsync) or set INTELHUB_TARBALL=<url>."
  else
    git clone --depth 1 -b "$REPO_BRANCH" "$REPO_URL" "$HOME_DIR"
  fi
  # re-exec the on-disk copy so the rest of the run uses the repo's own scripts
  if [[ "${BASH_SOURCE[0]}" != "$HOME_DIR/scripts/install.sh" ]]; then
    echo "    re-executing $HOME_DIR/scripts/install.sh"
    exec bash "$HOME_DIR/scripts/install.sh"
  fi
}

step_bootstrap() {
  INTELHUB_HOME="$HOME_DIR" INTELHUB_USER="$RUN_USER" \
    bash "$HOME_DIR/scripts/bootstrap-host.sh"
}

step_versions() {
  if [[ -f "$HOME_DIR/compose/.env" && ",$REDO," != *",versions,"* ]]; then
    echo "    compose/.env exists — keeping (REDO=versions to regenerate pinned tags)"
    return 0
  fi
  FORCE=1 bash "$HOME_DIR/scripts/resolve-versions.sh"
}

step_secrets() {
  local cenv="$HOME_DIR/compose/.env"
  [[ -f "$cenv" ]] || die "compose/.env missing — versions step must run first"
  # write-once guard: never regenerate an existing deployment's secrets
  for f in "$HOME_DIR/core/hub.env" "$HOME_DIR/core/secrets.env"; do
    if [[ -f "$f" ]]; then echo "    exists, keeping: $f"; fi
  done

  if [[ ! -f "$HOME_DIR/core/hub.env" ]]; then
    mkdir -p "$HOME_DIR/core" "$HOME_DIR/data/raw" "$HOME_DIR/backups"
    local PG REDIS NEO CRAWL LANIP
    PG=$(get_env "$cenv" POSTGRES_PASSWORD);   REDIS=$(get_env "$cenv" REDIS_PASSWORD)
    NEO=$(get_env "$cenv" NEO4J_PASSWORD);     CRAWL=$(get_env "$cenv" CRAWL4AI_API_TOKEN)
    LANIP=$(get_env "$cenv" LAN_IP)
    [[ -n "$PG" && -n "$REDIS" && -n "$NEO" && -n "$LANIP" ]] || die "compose/.env incomplete"
    local ADMIN; ADMIN=$(openssl rand -hex 24)
    cat > "$HOME_DIR/core/hub.env" <<EOF
HUB_LISTEN_ADDR=${LANIP}:8800
DATABASE_URL=postgres://intelhub:${PG}@172.30.2.10:5432/intelhub
REDIS_URL=redis://:${REDIS}@172.30.2.11:6379
NEO4J_URI=bolt://172.30.2.12:7687
NEO4J_USER=neo4j
NEO4J_PASSWORD=${NEO}
QDRANT_URL=http://172.30.2.13:6333
SEARXNG_URL=http://${LANIP}:8080
CRAWL4AI_URL=http://${LANIP}:11235
CRAWL4AI_API_TOKEN=${CRAWL}
EMBEDDING_BASE_URL=https://ai.t8star.org/v1
EMBEDDING_MODEL=text-embedding-3-small
EMBEDDING_DIMS=1536
HUB_RAW_DIR=${HOME_DIR}/data/raw
HUB_MANIFESTS_DIR=${HOME_DIR}/manifests
HUB_SCRIPTS_DIR=${HOME_DIR}/scripts
HUB_CONSOLE_DIR=${HOME_DIR}/console/dist
HUB_ADMIN_TOKEN=${ADMIN}
HUB_RATE_LIMIT_RPM=120
HUB_BUDGET_AGENT_TOOL_CALLS=2000
HUB_BUDGET_AGENT_CRAWL_PAGES=300
HUB_BUDGET_AGENT_EMBED_TOKENS=1000000
HUB_BUDGET_GLOBAL_CRAWL_PAGES=1000
HUB_BUDGET_GLOBAL_EMBED_TOKENS=5000000
HUB_ALERT_WEBHOOK_MIN_SEVERITY=warning
HUB_EMBED_MIN_WORDS=300
HUB_EMBED_WORKER_ENABLED=true
EOF
    chmod 600 "$HOME_DIR/core/hub.env"
    echo "$ADMIN" > "$HOME_DIR/core/admin-token.txt" && chmod 600 "$HOME_DIR/core/admin-token.txt"
    echo "    wrote core/hub.env + core/admin-token.txt (0600)"
  fi

  if [[ ! -f "$HOME_DIR/core/secrets.env" ]]; then
    cat > "$HOME_DIR/core/secrets.env" <<'EOF'
# IntelHub hub-core secrets (0600, never commit). Referenced by hub.env names only.
EMBEDDING_API_KEY=
HUB_ALERT_TELEGRAM_BOT_TOKEN=
HUB_ALERT_TELEGRAM_CHAT_ID=
# SP6 native monitor source keys (empty = that source degrades by design)
FIRMS_MAP_KEY=
ACLED_EMAIL=
ACLED_PASSWORD=
EOF
    chmod 600 "$HOME_DIR/core/secrets.env"
    echo "    wrote core/secrets.env (empty key slots)"
  fi
}

step_keys() {
  [[ "$NONINTERACTIVE" == "1" ]] && { echo "    non-interactive: all key prompts skipped"; return 0; }
  local senv="$HOME_DIR/core/secrets.env"
  local v

  if [[ -z "$(get_env "$senv" EMBEDDING_API_KEY)" ]]; then
    v=$(prompt_key "T8STAR KEY" "嵌入 relay key (sk-…)" "语义搜索降级为纯关键词")
    [[ -n "$v" ]] && { upsert_env "$senv" EMBEDDING_API_KEY "$v"; CHANGED_HUB_ENV=1; }
  fi
  if [[ -z "$(get_env "$senv" FIRMS_MAP_KEY)" ]]; then
    v=$(prompt_key "FIRMS_MAP_KEY" "NASA FIRMS 火点图层 key" "雷达缺火点图层")
    [[ -n "$v" ]] && { upsert_env "$senv" FIRMS_MAP_KEY "$v"; CHANGED_HUB_ENV=1; }
  fi
  if [[ -z "$(get_env "$senv" ACLED_EMAIL)" ]]; then
    v=$(prompt_key "ACLED_EMAIL" "ACLED 账号邮箱（冲突事件图层）" "雷达缺冲突图层")
    if [[ -n "$v" ]]; then
      upsert_env "$senv" ACLED_EMAIL "$v"
      local p
      p=$(prompt_key "ACLED_PASSWORD" "ACLED 账号密码" "冲突图层仍缺")
      [[ -n "$p" ]] && upsert_env "$senv" ACLED_PASSWORD "$p"
      CHANGED_HUB_ENV=1
    fi
  fi
  if [[ -z "$(get_env "$senv" HUB_ALERT_TELEGRAM_BOT_TOKEN)" ]]; then
    v=$(prompt_key "TELEGRAM_BOT_TOKEN" "Telegram 告警 bot token（找 @BotFather 创建）" "告警只在控制台可见，无推送")
    if [[ -n "$v" ]]; then
      upsert_env "$senv" HUB_ALERT_TELEGRAM_BOT_TOKEN "$v"
      local c
      c=$(prompt_key "TELEGRAM_CHAT_ID" "你的 Telegram chat_id（先给 bot 发条消息，再访问 api.telegram.org/bot<token>/getUpdates 查看）" "同上")
      [[ -n "$c" ]] && upsert_env "$senv" HUB_ALERT_TELEGRAM_CHAT_ID "$c"
      CHANGED_HUB_ENV=1
    fi
  fi

  # CARTO basemap key → console build-time input (never into the repo tree)
  local benv="$HOME_DIR/core/console-build.env"
  [[ -f "$benv" ]] || { echo "# console build-time inputs (0600)" > "$benv"; chmod 600 "$benv"; }
  if [[ -z "$(get_env "$benv" VITE_CARTO_KEY)" ]]; then
    v=$(prompt_key "CARTO_BASEMAP_KEY" "CARTO 底图 key（carto.com/basemaps/apikey 免费注册）" "雷达自动使用 Esri 兜底底图（美观度略降）")
    [[ -n "$v" ]] && upsert_env "$benv" VITE_CARTO_KEY "$v"
  fi
}

step_build_hub() {
  bash "$HOME_DIR/scripts/build-hub.sh"
  sudo chown "$RUN_USER:$RUN_USER" "$HOME_DIR/core/hub"
  # render the systemd unit for this user/home, install + enable (start later)
  sudo sed -e "s|/home/zou/IntelHub|$HOME_DIR|g" \
           -e "s|^User=zou|User=$RUN_USER|" \
           -e "s|^Group=zou|Group=$RUN_USER|" \
      "$HOME_DIR/config/hub-core.service" | sudo tee /etc/systemd/system/hub-core.service >/dev/null
  sudo systemctl daemon-reload
  sudo systemctl enable hub-core
  echo "    hub-core.service installed + enabled (starts after the stack is up)"
}

step_stack_up() {
  (cd "$HOME_DIR" && bash scripts/hub-compose.sh --profile optional up -d)
  # wait for postgres healthy (hub migrations need it)
  local i
  for i in $(seq 1 24); do
    [[ "$(docker inspect -f '{{.State.Health.Status}}' intelhub-postgres 2>/dev/null)" == "healthy" ]] && { echo "    postgres healthy"; return 0; }
    sleep 5
  done
  die "postgres did not become healthy in 120s"
}

step_build_console() {
  local benv="$HOME_DIR/core/console-build.env"
  if [[ -f "$benv" ]]; then set -a; . "$benv"; set +a; fi
  bash "$HOME_DIR/scripts/build-console.sh"
  sudo chown -R "$RUN_USER:$RUN_USER" "$HOME_DIR/console/dist" "$HOME_DIR/console/node_modules" 2>/dev/null || true
}

step_start_hub() {
  sudo systemctl restart hub-core
  local i code
  for i in $(seq 1 24); do
    code=$(curl -s -o /dev/null -w '%{http_code}' -m 3 "http://$(get_env "$HOME_DIR/core/hub.env" HUB_LISTEN_ADDR | cut -d: -f1):8800/api/v1/health" 2>/dev/null || echo 000)
    [[ "$code" != "000" ]] && { echo "    hub-core answering (http $code)"; return 0; }
    sleep 5
  done
  sudo journalctl -u hub-core --since "-2min" --no-pager | tail -10 >&2 || true
  die "hub-core did not come up"
}

step_provision_agents() {
  local kf="$HOME_DIR/core/agent-keys.txt"
  [[ -f "$kf" ]] || { touch "$kf"; chmod 600 "$kf"; }
  set -a; . "$HOME_DIR/core/hub.env"; . "$HOME_DIR/core/secrets.env"; set +a
  local name
  for name in codex console; do
    if grep -qE "^name:\s+${name}\$" "$kf"; then
      echo "    agent '$name' already provisioned"
      continue
    fi
    echo "=== $name ===" >> "$kf"
    "$HOME_DIR/core/hub" create-agent --name "$name" | grep -E '^(agent_id|name|api_key):' >> "$kf"
    echo "    agent '$name' created → $kf"
  done
}

step_verify() {
  # apply deferred hub restart when keys were added to an already-running system
  [[ "$CHANGED_HUB_ENV" == "1" ]] && sudo systemctl restart hub-core || true
  if ! bash "$HOME_DIR/scripts/health-check.sh"; then
    mark_undone verify
    die "health check failed — re-run the same command after fixing the cause"
  fi
}

# ------------------------------------------------------------------- main ---
say "IntelHub installer — crash-safe, resumable. State: $STATE"
[[ "$FORCE" == "1" ]] && [[ -f "$STATE" ]] && { warn "FORCE=1: wiping step state (secrets remain write-once)"; : > "$STATE"; }

run_step preflight        step_preflight
run_step fetch-code       step_fetch_code
run_step bootstrap        step_bootstrap
run_step versions         step_versions
run_step secrets          step_secrets
run_step keys             step_keys
run_step build-hub        step_build_hub
run_step stack-up         step_stack_up
run_step build-console    step_build_console
run_step start-hub        step_start_hub
run_step provision-agents step_provision_agents
run_step verify           step_verify

LANIP=$(get_env "$HOME_DIR/compose/.env" LAN_IP)
CONSOLE_KEY=$(grep -A3 '^=== console ===' "$HOME_DIR/core/agent-keys.txt" 2>/dev/null | grep -o 'ihk_[a-f0-9]*' | head -1)
cat <<EOF

┌──────────────────────────────────────────────────────────────┐
│  IntelHub 安装完成                                            │
├──────────────────────────────────────────────────────────────┤
│  控制台    http://${LANIP}:8800/                              │
│  雷达      控制台内 Global Radar 页                           │
│  Grafana   http://${LANIP}:3001  (admin / 见 compose/.env)   │
│  console API key: ${CONSOLE_KEY:-见 core/agent-keys.txt}
│  admin token:     core/admin-token.txt (0600)                │
│  密钥文件:  core/secrets.env · compose/.env* (0600, 勿外传)  │
└──────────────────────────────────────────────────────────────┘
重跑同一命令可续装/补 key（REDO=keys 重答密钥问题）。
EOF
