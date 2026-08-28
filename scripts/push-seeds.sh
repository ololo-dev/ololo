#!/usr/bin/env sh
# Push local seed fixtures (judges/*.md + projects/) to a running server.
#
#   scripts/push-seeds.sh https://your-deployment.example
#   scripts/push-seeds.sh https://your-deployment.example --dry-run
#   scripts/push-seeds.sh https://your-deployment.example --only hop-hop
#
# First argument is the deployment's base URL; everything after it is passed
# through to the push-seeds binary (see `--help`). Admin token comes from
# --token, $ARENA_ADMIN_TOKEN, or ~/.config/ololo/credentials.toml.
set -eu

cd "$(dirname "$0")/.."

url="${1:?usage: scripts/push-seeds.sh <base-url> [options]}"
shift

exec cargo run -q -p server --bin push-seeds -- --url "$url" "$@"
