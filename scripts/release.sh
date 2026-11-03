#!/usr/bin/env bash
# Build the static release binary that runs on the host, then copy it to dist/.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

TARGET=x86_64-unknown-linux-musl
./scripts/dev.sh cargo build --release --target "$TARGET"

mkdir -p dist
install -m 0755 "target/$TARGET/release/koshi" dist/koshi
echo "dist/koshi: $(dist/koshi --version)"
file dist/koshi 2>/dev/null || true
