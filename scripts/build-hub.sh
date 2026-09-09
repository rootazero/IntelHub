#!/usr/bin/env bash
# build-hub.sh — build the hub binary on the VM (run ON IntelHub).
# Build env = rust:trixie container == run env (Debian 13 glibc), so the
# glibc-linked binary runs natively on the host. Registry + target caches
# live in named volumes so iterations only recompile changed crates.
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$HUB_DIR/hub-core"
OUT="$HUB_DIR/core/hub"

echo "==> cargo build --release (rust:trixie)"
docker run --rm \
  -v "$SRC":/ws -w /ws \
  -v intelhub-hub-target:/ws/target \
  -v intelhub-cargo-release:/ws/target/release \
  -v intelhub-cargo-registry:/usr/local/cargo/registry \
  -v intelhub-cargo-git:/usr/local/cargo/git \
  rust:trixie cargo build --release --workspace

# target/release is a named volume; copy the binary out via a throwaway mount
docker run --rm -v intelhub-cargo-release:/rel -v "$HUB_DIR/core":/out alpine \
  sh -c 'cp /rel/hub /out/hub && chmod 755 /out/hub'

echo "==> built: $OUT"
ls -lh "$OUT"
