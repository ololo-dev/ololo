#!/usr/bin/env bash
set -euo pipefail
# Wipes arena.db and re-runs all migrations from scratch (dev only).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

rm -f "${REPO_ROOT}/arena.db"
DATABASE_URL="sqlite://${REPO_ROOT}/arena.db?mode=rwc" \
  cargo run --manifest-path "${REPO_ROOT}/server/migration/Cargo.toml" --bin migration -- fresh
echo "arena.db reset complete."
