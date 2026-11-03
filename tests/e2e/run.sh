#!/usr/bin/env bash
# Build the debug binary, then run the end-to-end tests inside the dev container.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"

./scripts/dev.sh cargo build

SUITES=(tests/e2e/test_*.py)

for test in "${SUITES[@]}"; do
  ./scripts/dev.sh python3 "$test" "$@"
done
