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

# Pre-load INTELHUB_ENV_FILE (unattended mode). Runs BEFORE any other env
# vars are read so user-supplied INTELHUB_HOME / LAN_IP / INTELHUB_LAN take
# effect for the rest of the script. Missing keys remain empty → step_keys
# will prompt for them (or skip under INTELHUB_NONINTERACTIVE=1).
if [[ -n "${INTELHUB_ENV_FILE:-}" ]]; then
  [[ -r "${INTELHUB_ENV_FILE}" ]] || { printf '\033[31m!! INTELHUB_ENV_FILE=%s unreadable\033[0m\n' "${INTELHUB_ENV_FILE}" >&2; exit 1; }
  set -a; . "${INTELHUB_ENV_FILE}"; set +a
  echo "    preloaded $(grep -cE '^[A-Z_]+=' "${INTELHUB_ENV_FILE}") keys from ${INTELHUB_ENV_FILE}"
fi

# ---------------------------------------------------------------- config ----
# Public repo — `curl ... | bash` on a clean VM does a fresh `git clone` of
# the source tree. INTELHUB_TARBALL=<url> bypasses git for tarball installs
# (firewalled VMs that can't reach GitHub, or air-gapped deployments).
REPO_URL="https://github.com/rootazero/IntelHub.git"
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
mark_done() {
  [[ -f "$STATE" ]] || { mkdir -p "$(dirname "$STATE")"; touch "$STATE"; chmod 600 "$STATE"; }
  grep -qxF "$1" "$STATE" 2>/dev/null || echo "$1" >> "$STATE"
}
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
  sudo -n -v 2>/dev/null || warn "sudo requires password or tty; NOPASSWD+tty-free setup recommended (see step_bootstrap)"
  mkdir -p "$HOME_DIR"
  echo "    os=${PRETTY_NAME:-$ID} user=$RUN_USER home=$HOME_DIR"
}

step_fetch_code() {
  if [[ -f "$HOME_DIR/scripts/hub-compose.sh" && -d "$HOME_DIR/compose" ]]; then
    echo "    code already present at $HOME_DIR"
    return 0
  fi
  # Clean up stale .install-state from a prior aborted preflight; STATE gets
  # lazily recreated by mark_done() so removing it here is always safe.
  if [[ -d "$HOME_DIR" ]]; then
    stale=$(find "$HOME_DIR" -mindepth 1 ! -name .install-state 2>/dev/null | wc -l)
    if [[ "$stale" -eq 0 ]]; then
      rm -f "$HOME_DIR/.install-state"
    fi
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
HUB_EMBED_MIN_WORDS=50
HUB_EMBED_WORKER_ENABLED=true
HUB_SEED_ENABLED=true
# Cross-encoder rerank stage (e2e audit 2026-09-13). Off by default — flip
# on after you've verified T8star /v1/rerank is reachable from your VM.
HUB_RERANK_ENABLED=false
HUB_RERANK_MODEL=BAAI/bge-reranker-v2-m3
HUB_RERANK_TOP_K=50
HUB_RERANK_TIMEOUT_SECS=10
# A: Redis-backed query result cache (2026-09-13). Repeated identical
# hybrid/semantic/keyword search within TTL skips PG + Qdrant + T8star.
HUB_QUERY_CACHE_ENABLED=true
HUB_QUERY_CACHE_TTL_SECS=300
# B: LLM-driven planner for investigate() (2026-09-13). Falls back to
# rule-based silently on timeout/parse/empty. Off by default — flip on
# after verifying T8star /v1/chat/completions is reachable.
HUB_LLM_ENABLED=false
HUB_LLM_MODEL=gpt-4.1-mini
HUB_LLM_TIMEOUT_MS=5000
HUB_LLM_MAX_TOKENS=800
EOF
    chmod 600 "$HOME_DIR/core/hub.env"
    echo "$ADMIN" > "$HOME_DIR/core/admin-token.txt" && chmod 600 "$HOME_DIR/core/admin-token.txt"
    echo "    wrote core/hub.env + core/admin-token.txt (0600)"
  fi

  if [[ ! -f "$HOME_DIR/core/secrets.env" ]]; then
    # >>>INTELHUB_SECRETS_TEMPLATE_V1>>>
    # Marker block consumed by scripts/update.sh to detect newly-introduced
    # optional keys. To add a new optional secret: append `KEY=` inside the
    # heredoc below — update.sh will auto-add it to existing deployments on
    # the next update run.
    cat > "$HOME_DIR/core/secrets.env" <<'EOF'
# IntelHub hub-core secrets (0600, never commit). Referenced by hub.env names only.
EMBEDDING_API_KEY=
HUB_ALERT_TELEGRAM_BOT_TOKEN=
HUB_ALERT_TELEGRAM_CHAT_ID=
# SP6 native monitor source keys (empty = that source degrades by design)
FIRMS_MAP_KEY=
ACLED_EMAIL=
ACLED_PASSWORD=
# SP8: ReliefWeb 需注册免费 appname（apidoc.reliefweb.int/parameters#appname）
RELIEFWEB_APPNAME=
# SP6B finance collector keys (empty = that collector degrades by design)
FRED_API_KEY=
COMTRADE_API_KEY=
# BLS：注册页被 Akamai 封（需美国住宅网络一次）；空值 = 采集器可见降级
BLS_API_KEY=
# X(Twitter)：无免费读通道，Basic $200/月；空值 = 可见降级（Bluesky+Telegram 覆盖免费面）
X_BEARER_TOKEN=
FMP_API_KEY=
FINNHUB_API_KEY=
FINANCIALDATASETS_API_KEY=
EIA_API_KEY=
EOF
    chmod 600 "$HOME_DIR/core/secrets.env"
    echo "    wrote core/secrets.env (empty key slots)"
    # <<<INTELHUB_SECRETS_TEMPLATE_V1<<<
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
      p=$(prompt_key "ACLED_PASSWORD" "ACLED 账号密码（OAuth password grant，官方现行方式）" "冲突图层仍缺")
      [[ -n "$p" ]] && upsert_env "$senv" ACLED_PASSWORD "$p"
      CHANGED_HUB_ENV=1
    fi
  fi
  # SP6B finance collectors (each independently optional)
  local -a fin_keys=(
    "FRED_API_KEY|FRED 宏观序列 key (fred.stlouisfed.org)|宏观时序序列缺失"
    "COMTRADE_API_KEY|UN Comtrade 贸易序列 key (comtradedeveloper.un.org)|战略贸易流序列缺失"
    "FMP_API_KEY|FMP 报价 key (financialmodelingprep.com)|市场报价序列缺失"
    "FINNHUB_API_KEY|Finnhub key (finnhub.io)|新闻/内部人/财报日历情报缺失"
    "FINANCIALDATASETS_API_KEY|financialdatasets.ai key|financials_fetch 工具不可用"
    "EIA_API_KEY|EIA 能源 key (eia.gov)|油气现货序列缺失"
  )
  local entry k desc degrade
  for entry in "${fin_keys[@]}"; do
    k="${entry%%|*}"; local rest="${entry#*|}"; desc="${rest%%|*}"; degrade="${rest##*|}"
    if [[ -z "$(get_env "$senv" "$k")" ]]; then
      v=$(prompt_key "$k" "$desc" "$degrade")
      [[ -n "$v" ]] && { upsert_env "$senv" "$k" "$v"; CHANGED_HUB_ENV=1; }
    fi
  done
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

  # Dark basemap keys (optional) → console build-time inputs.
  # Order: Stadia (preferred, designed for OSINT data overlay, true
  # black) first; CARTO legacy primary second. Either or both may be
  # provided — the rendered chain picks Stadia if keyed, else CARTO if
  # keyed, else Esri fallback (grey not black).
  local benv="$HOME_DIR/core/console-build.env"
  [[ -f "$benv" ]] || { echo "# console build-time inputs (0600)" > "$benv"; chmod 600 "$benv"; }
  if [[ -z "$(get_env "$benv" VITE_STADIA_KEY)" ]]; then
    v=$(prompt_key "STADIA_API_KEY" "Stadia Maps dark basemap key（stadiamaps.com 免费注册，非商业可用）" "雷达自动回退 Esri 灰色底图")
    [[ -n "$v" ]] && upsert_env "$benv" VITE_STADIA_KEY "$v"
  fi
  if [[ -z "$(get_env "$benv" VITE_CARTO_KEY)" ]]; then
    v=$(prompt_key "CARTO_BASEMAP_KEY" "CARTO 底图 key（carto.com/basemaps/apikey 免费注册）" "雷达自动回退 Esri 灰色底图")
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
  # Build local-only images first. intelhub/spiderfoot is pinned to a
  # smicallef/spiderfoot git commit (build-spiderfoot.sh) — no upstream
  # prebuilt image exists, so `docker compose pull` will fail for it on
  # a fresh VM. build-spiderfoot.sh is idempotent (skips if image exists).
  if grep -q '^SPIDERFOOT_IMAGE=' "$HOME_DIR/compose/.env" 2>/dev/null; then
    bash "$HOME_DIR/scripts/build-spiderfoot.sh" || warn "build-spiderfoot.sh failed (spiderfoot is profile:optional, install continues)"
  fi
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
  # One generic agent key (auth/budget axis) + the console key. Agent
  # identity beyond that is self-declared via MCP clientInfo — no per-agent
  # or per-provider preseeding (SP8 dual-axis model).
  for name in agent console; do
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
# Dispatch: `bash -s -- update` hands off to update.sh after the shared
# preflight + fetch-code prologue. The install state machine already fast-
# skips completed steps, so re-running the same update command is cheap.
if [[ "${1:-}" == "update" ]]; then
    exec bash "$HOME_DIR/scripts/update.sh"
fi
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

# Best-effort historical-data backfills on fresh install. The two
# scripts are idempotent and exit 0 with a no-op message when there's
# nothing to do; a fresh DB just prints "nothing to backfill".
# Skip with SKIP_BACKFILLS=1 (default on — they're cheap and harmless).
if [[ "${SKIP_BACKFILLS:-0}" != "1" ]]; then
  for bf in backfill-claim-audit.py backfill-finding-entities.py; do
    [[ -f "$HOME_DIR/scripts/$bf" ]] && python3 "$HOME_DIR/scripts/$bf" 2>&1 | tail -3 || true
  done
fi

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
