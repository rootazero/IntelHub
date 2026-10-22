#!/usr/bin/env python3
"""scripts/_remote.py — unified ssh-or-local VM helper for accept-sp*.py.

Auto-detects whether this script is running ON the hub VM (no ssh
needed) or REMOTELY (such as a developer Mac). When local, all helpers
execute via docker exec / open() directly. When remote, helpers shell
out via ssh to the VM named by INTELHUB_SSH (default: IntelHub).

Auto-detection sentinel: ``$INTELHUB_HOME/core/hub`` — the compiled
hub-core binary. This file is gitignored, so it ONLY exists on actual
installs of IntelHub. A developer Mac without a local install will
naturally fall through to the ssh path. No ``INTELHUB_LOCAL`` env needed.

Manual overrides:
  INTELHUB_LOCAL=1     — force local execution regardless of sentinel
  INTELHUB_SSH=<alias> — ssh destination alias (default: IntelHub)
  INTELHUB_HOME=<path> — hub checkout root (default: /home/zou/IntelHub)

Helpers:
  is_local_vm()         — bool, true if running on the hub VM
  run(cmd, timeout=90)  — full subprocess.CompletedProcess (rc, stdout, stderr)
  sh(cmd, timeout=90)   — str, stripped stdout (empty on error)
  pg(query, timeout=60) — str, psql query result (single statement)
  pg_params(sql, *params) — like pg() but substitutes $1/$2 placeholders
  pg_stdin(sql)         — multi-statement psql via base64 stdin
  redis(*args, timeout=30) — str, redis-cli output
  cypher(query, timeout=30) — str, cypher-shell output (auth via the
                              container's own NEO4J_AUTH env var)
  docker_exec(container, *args) — generic docker exec, returns stripped stdout
  grafana_creds()       — (user, password) tuple, password from compose/.env
  grafana_request(path) — GET {GRAFANA_URL}{path} with admin auth, returns JSON

Usage from a sibling script::

    import sys, os
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from _remote import sh, pg, redis, cypher, is_local_vm

    rows = pg("SELECT count(*) FROM documents")
    cells = redis("HKEYS", "hub:monitor:health")
    match = cypher("MATCH (n:Entity) RETURN count(n)")
"""

import os
import subprocess

# ─── configuration ─────────────────────────────────────────────────────
SSH_HOST = os.environ.get("INTELHUB_SSH", "IntelHub")
HOME_DIR = os.environ.get("INTELHUB_HOME", "/home/zou/IntelHub")

# Grafana URL — mgmt-net static IP pinned in compose.ui.yml. Reachable
# from the hub VM only (not from the LAN), so callers must run on the
# VM (or ssh in via :func:`sh`). If the deployment ever moves the
# Grafana container, update this constant OR set INTELHUB_GRAFANA_URL.
GRAFANA_URL = os.environ.get("INTELHUB_GRAFANA_URL", "http://172.30.3.22:3000")

# Sentinel: compiled hub-core binary is gitignored, only exists on installs.
_LOCAL_SENTINEL = os.path.join(HOME_DIR, "core", "hub")


# ─── detection ─────────────────────────────────────────────────────────
def is_local_vm() -> bool:
    """True if this script is running on the hub VM (no ssh needed).

    Auto-detected via sentinel file ``$INTELHUB_HOME/core/hub``. The
    binary is gitignored — only present on actual installs. Override
    with ``INTELHUB_LOCAL=1`` to force local mode (e.g. when running
    scripts locally on a Mac VM that has a copy of hub-core).
    """
    if os.environ.get("INTELHUB_LOCAL") == "1":
        return True
    return os.path.exists(_LOCAL_SENTINEL)


# ─── core runner ──────────────────────────────────────────────────────
def _run_remote(cmd, timeout=90, **kwargs):
    """Execute ``cmd`` (str) via ssh on the configured SSH_HOST."""
    return subprocess.run(
        ["ssh", "-o", "BatchMode=yes", SSH_HOST, cmd],
        capture_output=True, text=True, timeout=timeout, **kwargs,
    )


def _run_local(cmd, timeout=90, **kwargs):
    """Execute ``cmd`` (str) locally via /bin/bash (shell=True)."""
    return subprocess.run(
        cmd, shell=True, executable="/bin/bash",
        capture_output=True, text=True, timeout=timeout, **kwargs,
    )


def run(cmd, timeout=90, **kwargs):
    """Run a shell command on the hub VM (locally or via ssh).

    Returns the full :class:`subprocess.CompletedProcess` so callers can
    inspect ``.stderr`` / ``.returncode`` on failure. Most callers should
    prefer :func:`sh` for the simple stdout-only case.
    """
    runner = _run_local if is_local_vm() else _run_remote
    return runner(cmd, timeout=timeout, **kwargs)


def sh(cmd, timeout=90) -> str:
    """Run a shell command, return stripped stdout (empty on error)."""
    return run(cmd, timeout=timeout).stdout.strip()


# ─── generic docker exec (mirrors sp10.docker_exec) ───────────────────
def docker_exec(container, *args, timeout=30):
    """Run a command inside a container. Returns stripped stdout."""
    return subprocess.run(
        ["docker", "exec", container, *args],
        capture_output=True, text=True, timeout=timeout,
    ).stdout.strip()


# ─── postgres ──────────────────────────────────────────────────────────
# The postgres container does not publish host ports (it's on the
# internal intelhub-data docker network). All access goes through
# ``docker exec``. We use the standard deployment user ``intelhub`` and
# database ``intelhub`` — non-default deployments can override via
# POSTGRES_USER / POSTGRES_DB env vars in the helper.
def pg(query: str, timeout: int = 60) -> str:
    """Run a psql query (single statement), return stripped stdout.

    The query is wrapped in single quotes for shell safety. Inner
    single quotes are escaped (``'`` → ``'\\''``). Output mode is
    ``-tA`` (tuples-only, unaligned) so callers get one row per line
    with no padding.
    """
    escaped = query.replace("'", "'\\''")
    return sh(
        f"docker exec intelhub-postgres psql -U intelhub -d intelhub -tAc '{escaped}'",
        timeout=timeout,
    )


def pg_stdin(sql: str, timeout: int = 60) -> str:
    """Run a multi-statement psql block via stdin, return stripped stdout.

    Use this when you have a SQL block that won't fit on a single -c
    line or that needs multiple statements (e.g. INSERT followed by
    UPDATE in a transaction, or DDL). The caller passes the raw SQL
    bytes — we encode as base64 to survive both ssh and local shell
    expansion without any escaping headaches.
    """
    import base64
    b64 = base64.b64encode(sql.encode()).decode()
    return sh(
        f"echo {b64} | base64 -d | docker exec -i intelhub-postgres "
        f"psql -U intelhub -d intelhub -tAq",
        timeout=timeout,
    )


# ─── redis ─────────────────────────────────────────────────────────────
# The redis container already carries REDIS_PASSWORD in its own env
# (compose.data.yml). We therefore authenticate *inside* the container
# via REDISCLI_AUTH instead of passing `-a <password>` on the command
# line: the old form put the plaintext password into the host's process
# table (visible in `ps` and in `docker events`).
def redis(*args, timeout=30) -> str:
    """Run a redis-cli command, return stripped stdout.

    Arguments are passed through as a shell string. Example::

        redis("HKEYS", "hub:monitor:health")
        redis("HGET", "hub:monitor:health", "firms")

    Authentication happens inside the container (REDISCLI_AUTH sourced
    from the container's own REDIS_PASSWORD env var), so the password
    never appears in the host's argv.
    """
    quoted_args = " ".join(f"'{a}'" for a in args)
    cmd = (
        "docker exec intelhub-redis sh -c "
        "'REDISCLI_AUTH=\"$REDIS_PASSWORD\" redis-cli --no-auth-warning \"$@\"' sh "
        f"{quoted_args}"
    )
    return sh(cmd, timeout=timeout)


# ─── neo4j ─────────────────────────────────────────────────────────────
# NEO4J_AUTH lives in the neo4j container's env as ``neo4j/<password>``,
# NOT in compose/.env. We authenticate *inside* the container: the
# password is derived from the container's own NEO4J_AUTH env var and
# handed to cypher-shell through NEO4J_PASSWORD, so it never appears in
# the host's argv (the old `cypher-shell -p <password>` form exposed it
# in `ps` and in `docker events`).
def cypher(query: str, timeout: int = 30) -> str:
    """Run a cypher-shell query against intelhub-neo4j.

    Returns stripped stdout. Use ``cypher-shell --format plain`` semantics:
    the column header is the first line, data rows follow. Callers
    typically strip the header themselves.

    Single quotes inside the query are escaped for shell safety.
    """
    escaped = query.replace("'", "'\\''")
    # The query is passed as a positional parameter ($1) so no amount of
    # quoting inside it can break out of the container's shell.
    cmd = (
        "docker exec intelhub-neo4j sh -c "
        "'NEO4J_PASSWORD=\"${NEO4J_AUTH#neo4j/}\" "
        "cypher-shell -u neo4j --format plain \"$1\"' sh "
        f"'{escaped}'"
    )
    return sh(cmd, timeout=timeout)


# ─── grafana ───────────────────────────────────────────────────────────
# The admin password is generated by scripts/resolve-versions.sh
# (``GRAFANA_ADMIN_PASSWORD=$(rand)``) and stored in compose/.env.
# Previously each callsite re-derived it via a subshell grep — now we
# cache it and expose a high-level request helper.
import json as _json
_GRAFANA_PASSWORD_CACHE = None


def grafana_creds() -> tuple:
    """Return ``(user, password)`` tuple for the Grafana admin account.

    Password is auto-extracted from compose/.env on first call and
    cached. User is hardcoded to ``admin`` (the only user Grafana
    creates by default in our compose).
    """
    global _GRAFANA_PASSWORD_CACHE
    if _GRAFANA_PASSWORD_CACHE is None:
        env_path = os.path.join(HOME_DIR, "compose", ".env")
        out = sh(
            f"grep '^GRAFANA_ADMIN_PASSWORD=' '{env_path}' | head -1 | cut -d= -f2-",
            timeout=10,
        )
        _GRAFANA_PASSWORD_CACHE = out.strip()
    return ("admin", _GRAFANA_PASSWORD_CACHE)


def grafana_request(path: str, timeout: int = 5, return_json: bool = True):
    """GET ``{GRAFANA_URL}{path}`` with auto-extracted admin creds.

    Path is the API path only (e.g. ``/api/datasources``), not the host.
    With ``return_json=True`` (default) the response body is parsed as
    JSON and returned; with ``False`` the raw stripped stdout is
    returned (useful for non-JSON endpoints like ``/api/health``).

    The HTTP call is routed through :func:`sh` (curl is universally
    available; urllib lacks the ``-u user:pass`` ergonomics here).
    """
    user, pw = grafana_creds()
    raw = sh(
        f"curl -s -m {timeout} -u '{user}:{pw}' '{GRAFANA_URL}{path}'",
        timeout=timeout + 2,
    )
    if not return_json:
        return raw
    try:
        return _json.loads(raw) if raw else {}
    except _json.JSONDecodeError:
        return {"_raw": raw[:400]}


# ─── psql parameterized (test-fixture only) ────────────────────────────
# sp10's _pg_scalar substitutes $1, $2 placeholders inline as quoted
# literals — a deliberate trade-off for clarity over safety in test
# fixtures (the values are always UUIDs or controlled marker strings).
# For user-controlled SQL use :func:`pg_stdin` instead.
def pg_params(sql: str, *params, timeout: int = 60) -> str:
    """Like :func:`pg` but supports ``$1``/``$2`` placeholder substitution.

    Each param is inserted as a single-quoted SQL literal. Empty
    params raise ValueError (caller bug). Do NOT use with untrusted
    input — use :func:`pg_stdin` (which goes via base64-encoded
    stdin) for any user-controlled value.
    """
    for i, p in enumerate(params, 1):
        if p is None:
            raise ValueError(f"pg_params: param {i} is None (use IS NULL or omit)")
        # Escape single quotes inside the literal; SQL standard.
        escaped = str(p).replace("'", "''")
        sql = sql.replace(f"${i}", f"'{escaped}'")
    return pg(sql, timeout=timeout)