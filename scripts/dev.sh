#!/usr/bin/env bash
# Run a command inside the development container with the repo mounted at /w.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${KOSHI_CARGO_HOME:-$HOME/.cache/koshi/cargo}"
mkdir -p "$CACHE"

exec docker run --rm -i ${KOSHI_TTY:+-t} \
  -u "$(id -u):$(id -g)" \
  -v "$REPO":/w \
  -v "$CACHE":/cargo \
  -e CARGO_HOME=/cargo \
  -e HOME=/tmp \
  -e TERM="${TERM:-xterm-256color}" \
  -e CARGO_TERM_COLOR=never \
  -w /w \
  koshi-dev "$@"
