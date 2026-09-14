#!/usr/bin/env bash
# reset-key.sh — IntelHub API key rotation
#
# Usage (run ON the installed VM):
#   bash scripts/reset-key.sh agent      # rotate the agent API key
#   bash scripts/reset-key.sh console    # rotate the console API key
#   bash scripts/reset-key.sh all        # rotate both
#
# What it does:
#   - Calls `hub rotate-agent-key --name <name>` which soft-revokes the
#     agent's previous api_keys (kept for audit) and mints a fresh one.
#     The agent_id is preserved so MCP clients keep their identity.
#   - Replaces the matching `=== name ===` block in core/agent-keys.txt
#     (mode 0600) so the on-disk record reflects the new key.
#   - Does NOT touch install state, secrets, or restart any service.
#     New key works immediately — hub-core hashes & looks up on every
#     request, so the old key starts failing as soon as the UPDATE commits.
#
# Env overrides:
#   INTELHUB_HOME=<dir>           install root (default ~/IntelHub)
#   INTELHUB_NONINTERACTIVE=1     skip the "are you sure?" confirmation

set -euo pipefail

HUB_DIR="${INTELHUB_HOME:-$HOME/IntelHub}"
KEYS_FILE="$HUB_DIR/core/agent-keys.txt"
HUB_BIN="$HUB_DIR/core/hub"
NONINTERACTIVE="${INTELHUB_NONINTERACTIVE:-0}"

NAME="${1:-}"
[[ -n "$NAME" ]] || {
  cat >&2 <<USAGE
usage: bash scripts/reset-key.sh {agent|console|all}

  agent    rotate only the agent API key (MCP clients)
  console  rotate only the console API key (web UI)
  all      rotate both, in sequence
USAGE
  exit 2
}

say()  { printf '\033[1m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[33m    !! %s\033[0m\n' "$*"; }
die()  { printf '\033[31m!! %s\033[0m\n' "$*" >&2; exit 1; }

# -------------------------------------------------------------- preflight ---
[[ -d "$HUB_DIR" ]]   || die "IntelHub not installed at $HUB_DIR (set INTELHUB_HOME)"
[[ -f "$KEYS_FILE" ]] || die "core/agent-keys.txt missing — install first"
[[ -x "$HUB_BIN" ]]   || die "$HUB_BIN not found or not executable (build hub-core?)"

[[ -f "$HUB_DIR/core/hub.env" ]]     || die "core/hub.env missing"
[[ -f "$HUB_DIR/core/secrets.env" ]] || die "core/secrets.env missing"

# Sanity: hub-core must be able to reach Postgres (rotate-agent-key inserts
# a new api_keys row in a transaction). We don't ping the binary itself —
# rotate-agent-key will fail loudly with a PG error if the DB is down.
say "Rotating API key(s): $NAME"
warn "  The previous key will be invalidated immediately."
if [[ "$NONINTERACTIVE" != "1" ]]; then
  if [[ -t 0 ]]; then
    read -r -p "  Continue? [y/N] " ans
    [[ "$ans" =~ ^[Yy]$ ]] || { echo "  aborted"; exit 0; }
  else
    warn "  no TTY and INTELHUB_NONINTERACTIVE!=1 — set INTELHUB_NONINTERACTIVE=1 to skip confirmation"
    die "aborting (refusing to rotate without confirmation)"
  fi
fi

# Pull the current key for `name` from agent-keys.txt so we can echo the
# invalidated value back to the user. Returns empty if the agent isn't yet
# provisioned (rotation still works — it just inserts a first key).
existing_key() {
  local name="$1"
  grep -A3 "^=== ${name} ===" "$KEYS_FILE" 2>/dev/null \
    | grep -o 'ihk_[a-f0-9]*' | head -1 || true
}

# Atomic (best-effort) block replacement in agent-keys.txt. Writes to a
# sibling tempfile then renames over the original so a crash mid-write
# can't leave the file half-edited. Keeps the file mode at 0600.
replace_block() {
  local name="$1" agent_id="$2" api_key="$3"
  python3 - "$KEYS_FILE" "$name" "$agent_id" "$api_key" <<'PYEOF'
import sys, re, os
kf, name, agent_id, api_key = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
with open(kf, 'r') as f:
    text = f.read()
block_pat = re.compile(
    rf'^=== {re.escape(name)} ===\n(?:agent_id:.*?\n|name:.*?\n|api_key:.*?\n)+\n?',
    re.MULTILINE,
)
new_block = f"=== {name} ===\nagent_id: {agent_id}\nname: {name}\napi_key: {api_key}\n\n"
if block_pat.search(text):
    text = block_pat.sub(new_block, text, count=1)
else:
    if text and not text.endswith('\n'):
        text += '\n'
    text += new_block
tmp = kf + '.tmp'
with open(tmp, 'w') as f:
    f.write(text)
os.chmod(tmp, 0o600)
os.replace(tmp, kf)
PYEOF
}

rotate_one() {
  local name="$1"
  local old_key=""
  old_key=$(existing_key "$name")

  set -a
  . "$HUB_DIR/core/hub.env"
  . "$HUB_DIR/core/secrets.env"
  set +a

  local out new_key new_agent_id
  out=$("$HUB_BIN" rotate-agent-key --name "$name" 2>&1) \
    || die "rotate-agent-key '$name' failed: $out"
  new_key=$(printf '%s\n' "$out" | grep -E '^api_key:'   | head -1 | awk '{print $2}')
  new_agent_id=$(printf '%s\n' "$out" | grep -E '^agent_id:' | head -1 | awk '{print $2}')
  [[ -n "$new_key" ]]      || die "rotate-agent-key did not return api_key; got: $out"
  [[ -n "$new_agent_id" ]] || die "rotate-agent-key did not return agent_id; got: $out"

  replace_block "$name" "$new_agent_id" "$new_key"

  echo
  echo "── ${name} ──"
  if [[ -n "$old_key" ]]; then
    echo "  old key: $old_key  (revoked)"
  else
    echo "  old key: <none — fresh provisioning>"
  fi
  echo "  new key: $new_key"
  echo "  agent_id (unchanged): $new_agent_id"
  echo
}

case "$NAME" in
  agent)   rotate_one agent ;;
  console) rotate_one console ;;
  all)     rotate_one agent; rotate_one console ;;
  *)       die "unknown name '$NAME' (use agent, console, or all)" ;;
esac

cat <<EOF

┌──────────────────────────────────────────────────────────────┐
│  ✓ API key 重置完成                                          │
├──────────────────────────────────────────────────────────────┤
│  新 key 已立即生效（旧 key 已失效）。无需重启任何服务。       │
│  备份在 core/agent-keys.txt（0600）。                        │
│  把新 key 更新到你 MCP 客户端 / 控制台配置里。                │
└──────────────────────────────────────────────────────────────┘
EOF