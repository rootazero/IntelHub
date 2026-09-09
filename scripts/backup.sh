#!/usr/bin/env bash
# backup.sh — pre-upgrade / scheduled restorable point (spec §5).
# Produces backups/<UTC-ts>/ with postgres dump, neo4j offline dump,
# qdrant snapshots, and a config/manifest/env tar. Retains newest 7.
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
DEST="$HUB_DIR/backups/$TS"
mkdir -p "$DEST/qdrant-snapshots"
cd "$HUB_DIR"

COMPOSE="scripts/hub-compose.sh"

echo "==> [1/4] postgres dumpall"
docker exec intelhub-postgres pg_dumpall -U intelhub | pigz > "$DEST/postgres-dumpall.sql.gz"

echo "==> [2/4] neo4j offline dump (brief stop/start)"
$COMPOSE stop neo4j
docker run --rm \
  -v intelhub-neo4jdata:/data \
  -v "$DEST:/backup" \
  --entrypoint neo4j-admin \
  "$(grep '^NEO4J_VERSION=' compose/.env | cut -d= -f2 | xargs -I{} echo 'neo4j:{}')" \
  database dump neo4j --to-path=/backup
$COMPOSE start neo4j

echo "==> [3/4] qdrant snapshots (per collection)"
COLS=$(docker run --rm --network intelhub-data curlimages/curl:8.14.1 \
  -fsS http://qdrant:6333/collections | jq -r '.result.collections[].name' || true)
for c in $COLS; do
  docker run --rm --network intelhub-data curlimages/curl:8.14.1 \
    -fsS -X POST "http://qdrant:6333/collections/$c/snapshots" >/dev/null
  echo "    snapshot requested: $c"
done
# snapshots land in the mounted backups/qdrant-snapshots dir; copy newest into DEST
if ls "$HUB_DIR"/backups/qdrant-snapshots/*.snapshot >/dev/null 2>&1; then
  cp "$HUB_DIR"/backups/qdrant-snapshots/*.snapshot "$DEST/qdrant-snapshots/" 2>/dev/null || true
fi

echo "==> [4/4] config + manifests + env tar"
tar -czf "$DEST/config.tar.gz" compose/ config/ manifests/ scripts/

echo "==> retention: keep newest 7"
ls -1dt "$HUB_DIR"/backups/20* 2>/dev/null | tail -n +8 | xargs -r rm -rf

echo "==> backup complete: $DEST"
ls -lh "$DEST"
