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
  redis(*args, timeout=30) — str, redis-cli output
  cypher(query, timeout=30) — str, cypher-shell output (auto-extracts NEO4J_AUTH)
  docker_exec(container, *args) — generic docker exec, returns stripped stdout

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
# Password lives in compose/.env as REDIS_PASSWORD. We extract it on
# demand via docker exec (works locally) or sh-eval over ssh (works
# remotely) — both fall through :func:`sh`.
_REDIS_PASSWORD_CACHE = None


def _redis_password() -> str:
    global _REDIS_PASSWORD_CACHE
    if _REDIS_PASSWORD_CACHE is not None:
        return _REDIS_PASSWORD_CACHE
    # Read via sh() so it routes through ssh on a remote Mac (where the
    # compose/.env file doesn't exist locally).
    env_path = os.path.join(HOME_DIR, "compose", ".env")
    out = sh(
        f"grep '^REDIS_PASSWORD=' '{env_path}' | head -1 | cut -d= -f2-",
        timeout=10,
    )
    _REDIS_PASSWORD_CACHE = out.strip()
    return _REDIS_PASSWORD_CACHE


def redis(*args, timeout=30) -> str:
    """Run a redis-cli command, return stripped stdout.

    Arguments are passed through as a shell string. Example::

        redis("HKEYS", "hub:monitor:health")
        redis("HGET", "hub:monitor:health", "firms")

    The redis password is auto-extracted from compose/.env on first
    call and cached.
    """
    pw = _redis_password()
    quoted_args = " ".join(f"'{a}'" for a in args)
    cmd = (
        f"docker exec intelhub-redis redis-cli "
        f"-a '{pw}' --no-auth-warning {quoted_args}"
    )
    return sh(cmd, timeout=timeout)


# ─── neo4j ─────────────────────────────────────────────────────────────
# NEO4J_AUTH lives in the neo4j container's env as ``neo4j/<password>``,
# NOT in compose/.env. We extract it via docker inspect once per process.
_NEO4J_PASSWORD_CACHE = None


def _neo4j_password() -> str:
    global _NEO4J_PASSWORD_CACHE
    if _NEO4J_PASSWORD_CACHE is not None:
        return _NEO4J_PASSWORD_CACHE
    # Read via sh() so it routes through ssh on a remote Mac (where the
    # neo4j container isn't accessible locally). Format: NEO4J_AUTH=neo4j/<password>
    out = sh(
        "docker inspect intelhub-neo4j "
        "--format '{{range .Config.Env}}{{println .}}{{end}}' "
        "| grep '^NEO4J_AUTH=neo4j/' | head -1 | sed 's|^NEO4J_AUTH=neo4j/||'",
        timeout=15,
    )
    _NEO4J_PASSWORD_CACHE = out.strip()
    return _NEO4J_PASSWORD_CACHE


def cypher(query: str, timeout: int = 30) -> str:
    """Run a cypher-shell query against intelhub-neo4j.

    Returns stripped stdout. Use ``cypher-shell --format plain`` semantics:
    the column header is the first line, data rows follow. Callers
    typically strip the header themselves.

    Single quotes inside the query are escaped for shell safety.
    """
    pw = _neo4j_password()
    escaped = query.replace("'", "'\\''")
    # ``bash -c`` lets us use single quotes around the cypher that
    # survive shell expansion on both the local /bin/bash and remote ssh
    # paths (ssh invokes the user's login shell which is also bash).
    cmd = (
        f"docker exec intelhub-neo4j bash -c "
        f'"cypher-shell -u neo4j -p \'{pw}\' --format plain \'{escaped}\'"'
    )
    return sh(cmd, timeout=timeout)