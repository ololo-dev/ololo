#!/usr/bin/env bash
# Local play loop: boot server + game-server on a scratch DB, log a local
# player in, and start a headless session — so a change to a project, judge,
# probe engine, or the CLI can be validated end to end without leaving the
# machine. Designed to be driven by a coding agent (Claude Code) or a human
# in a second terminal: probes execute in the printed workspace directory,
# you build there, and the session narrates itself to stdout.
#
#   scripts/dev-play.sh up [--fresh]           boot the stack (idempotent)
#   scripts/dev-play.sh start <slug> [--fresh] boot if needed + play a session
#                                              (foreground; Ctrl+C to leave)
#   scripts/dev-play.sh seeds [args...]        push local seeds to the stack
#   scripts/dev-play.sh llm-sync [--dry-run]   mirror dev's LLM providers/models
#   scripts/dev-play.sh status                 what is running
#   scripts/dev-play.sh stop                   stop the stack
#
# State lives in .play/ (gitignored): sqlite DB, logs, pids, workspaces.
# The stack: server on :8080, game-server on :8081, no frontend (use the
# API, or run `pnpm dev` separately — it proxies to :8080 by default).
# Judges that need an LLM use your environment's AI settings (AI_PROVIDER /
# OLLAMA_URL / OPENROUTER_API_KEY...) or the in-app /settings/ai config;
# without any provider their runs fail gracefully and probes still work.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLAY="$ROOT/.play"
LOGS="$PLAY/logs"
BIN="$ROOT/target/debug"
BASE_URL="http://localhost:8080"
PROFILE="local"
PLAY_EMAIL="play@local.test"
PLAY_PASSWORD="play-password-1"

# Shared environment for both processes. Explicit exports win over any .env
# the processes may load themselves (dotenv never overrides existing vars).
play_env() {
  export DATABASE_URL="sqlite://$PLAY/arena.db?mode=rwc"
  export JWT_SIGNING_KEY="local-play-signing-key-0123456789abcdef"
  export ARENA_FRONTEND_ORIGINS="http://localhost:5173"
  export SERVER_PORT=8080
  export GAME_SERVER_ID="00000000-0000-4000-8000-00000000a1a1"
  export GAME_SERVER_PORT=8081
  export GAME_SERVER_ADVERTISE_URL="ws://localhost:8081"
  export GAME_SERVER_ZMQ_BIND_ADDR="tcp://127.0.0.1:6000"
  export ARENA_PROJECTS_DIR="$ROOT/projects"
  export ARENA_JUDGES_DIR="$ROOT/judges"
  export ARENA_JUDGE_LOG_DIR="$PLAY/judge-logs"
  export ARENA_SESSION_LOG_DIR="$PLAY/session-logs"
  export OLOLO_ALLOW_UNSANDBOXED_EXEC=1
  # The repo's .env (dotenv-loaded by the binaries) may enable these for the
  # developer's own stack; the play stack always runs open.
  export TURNSTILE_ENABLED=false
  export EMAIL_PROVIDER=disabled
  export RUST_LOG="${RUST_LOG:-info}"
}

pid_alive() { [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null; }

# Detach a daemon with its REAL pid recorded and no inherited stdio: `exec`
# keeps the subshell's pid, and the closed stdout is what lets a caller pipe
# this script without the daemon holding the pipe open forever.
start_daemon() { # name, bin...
  local name="$1"
  shift
  (cd "$ROOT" && exec "$@" >"$LOGS/$name.log" 2>&1 </dev/null) &
  echo $! >"$PLAY/$name.pid"
}

wait_http() { # url, tries
  local url="$1" tries="${2:-60}"
  for _ in $(seq "$tries"); do
    if curl -sf -o /dev/null "$url"; then return 0; fi
    sleep 0.5
  done
  return 1
}

build() {
  echo "==> building server, game-server, ololo"
  (cd "$ROOT" && cargo build -q -p server -p game-server -p ololo)
}

up() {
  local fresh=0
  [ "${1:-}" = "--fresh" ] && fresh=1
  mkdir -p "$PLAY" "$LOGS"
  if [ "$fresh" = 1 ]; then
    echo "==> fresh DB"
    stop || true
    rm -f "$PLAY"/arena.db*
  fi
  build
  play_env
  if ! pid_alive "$PLAY/server.pid"; then
    echo "==> starting server on :8080 (log: .play/logs/server.log)"
    start_daemon server "$BIN/server"
  fi
  wait_http "$BASE_URL/api/projects" 120 || {
    echo "server did not come up; tail of log:" >&2
    tail -20 "$LOGS/server.log" >&2
    exit 1
  }
  if ! pid_alive "$PLAY/game-server.pid"; then
    echo "==> starting game-server on :8081 (log: .play/logs/game-server.log)"
    start_daemon game-server "$BIN/game-server"
  fi
  wait_http "http://localhost:8081/health" 60 \
    || echo "note: game-server /health not answering (endpoint may differ); check .play/logs/game-server.log"
  ensure_login
  ensure_seeds
  echo "==> stack is up: $BASE_URL (profile '$PROFILE')"
}

# The boot seed skips when no admin exists yet — true on every fresh DB
# (the first registrant becomes admin AFTER boot). Push the working tree's
# seeds over the admin API instead of restarting the server.
ensure_seeds() {
  local n
  n="$(curl -sf "$BASE_URL/api/projects" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["projects"]))' 2>/dev/null || echo 0)"
  if [ "${n:-0}" = "0" ]; then
    echo "==> seeding projects + judges from the working tree"
    "$ROOT/scripts/push-seeds.sh" dev --url "$BASE_URL" >"$LOGS/seeds.log" 2>&1 \
      || { echo "seed push failed; see .play/logs/seeds.log" >&2; tail -5 "$LOGS/seeds.log" >&2; exit 1; }
  fi
}

# Register (first user = admin on a fresh DB), log the CLI in via the
# device-flow, confirming it over the API instead of a browser.
ensure_login() {
  local tok
  tok="$(profile_token || true)"
  if [ -n "$tok" ] && curl -sf -o /dev/null -H "Authorization: Bearer $tok" "$BASE_URL/api/users/me"; then
    return 0
  fi
  echo "==> logging in profile '$PROFILE'"
  # Register is idempotent-ish: an existing user just fails, then login works.
  curl -sf -o /dev/null -X POST "$BASE_URL/auth/register" \
    -H 'Content-Type: application/json' \
    -d "{\"email\":\"$PLAY_EMAIL\",\"password\":\"$PLAY_PASSWORD\",\"display_name\":\"Local Player\"}" || true
  local access
  access="$(curl -sf -X POST "$BASE_URL/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"email\":\"$PLAY_EMAIL\",\"password\":\"$PLAY_PASSWORD\"}" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')"
  [ -n "$access" ] || { echo "login failed" >&2; exit 1; }

  local login_log="$PLAY/login.log"
  : >"$login_log"
  "$BIN/ololo" --profile "$PROFILE" login --server "$BASE_URL" --no-browser >"$login_log" 2>&1 &
  local login_pid=$!
  local cli_token=""
  for _ in $(seq 40); do
    cli_token="$(grep -o 'cli_token=[0-9a-f]\{64\}' "$login_log" | head -1 | cut -d= -f2 || true)"
    [ -n "$cli_token" ] && break
    sleep 0.5
  done
  [ -n "$cli_token" ] || { kill "$login_pid" 2>/dev/null || true; echo "no cli_token in login output" >&2; cat "$login_log" >&2; exit 1; }
  curl -sf -o /dev/null -X POST "$BASE_URL/auth/cli/confirm" \
    -H "Authorization: Bearer $access" \
    -H 'Content-Type: application/json' \
    -d "{\"cli_token\":\"$cli_token\"}" || { echo "cli confirm failed" >&2; exit 1; }
  wait "$login_pid" || { echo "ololo login did not complete" >&2; cat "$login_log" >&2; exit 1; }
  tok="$(profile_token)"
  curl -sf -o /dev/null -H "Authorization: Bearer $tok" "$BASE_URL/api/users/me" \
    || { echo "stored token does not validate" >&2; exit 1; }
}

profile_token() {
  python3 - "$PROFILE" <<'EOF'
import sys, tomllib, pathlib
p = pathlib.Path.home() / ".config/ololo/credentials.toml"
if not p.exists(): sys.exit(1)
d = tomllib.load(p.open("rb"))
prof = d.get(sys.argv[1]) or {}
tok = prof.get("token", "")
print(tok) if tok else sys.exit(1)
EOF
}

start_session() {
  local slug="${1:?usage: dev-play.sh start <project-slug> [--fresh]}"
  shift || true
  up "${1:-}"
  play_env
  local work="$PLAY/workspace/$(date +%Y%m%d-%H%M%S)-$slug"
  mkdir -p "$work/.ololo"
  # Unattended play cannot answer the CLI's probe-permission prompt —
  # pre-approve every command for this throwaway workspace.
  printf '{"permissions": {"allow": ["*"]}}\n' > "$work/.ololo/settings.json"
  echo "==> workspace: $work"
  echo "==> build the product THERE; probes run in that directory."
  echo "==> session page API: $BASE_URL/api/sessions/by-code/<join_code>"
  cd "$work"
  exec "$BIN/ololo" --profile "$PROFILE" start "$slug" --no-tui
}

status() {
  for p in server game-server; do
    if pid_alive "$PLAY/$p.pid"; then
      echo "$p: running (pid $(cat "$PLAY/$p.pid"))"
    else
      echo "$p: stopped"
    fi
  done
  curl -sf -o /dev/null "$BASE_URL/api/projects" && echo "api: ok" || echo "api: down"
}

stop() {
  local stopped=0
  for p in server game-server; do
    if pid_alive "$PLAY/$p.pid"; then
      kill "$(cat "$PLAY/$p.pid")" 2>/dev/null || true
      stopped=1
    fi
    rm -f "$PLAY/$p.pid"
  done
  [ "$stopped" = 1 ] && echo "==> stack stopped" || echo "==> nothing was running"
}

# Mirror the dev deployment's LLM providers/pools/assignments/judge models
# onto the local stack, taking API keys from the environment (see
# scripts/llm-sync.py). SRC defaults to plum.ololo.dev with its stored PAT.
llm_sync() {
  # Provider API keys may live in the repo's .env — make them visible to
  # the sync (shell exports still win).
  if [ -f "$ROOT/.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$ROOT/.env"
    set +a
  fi
  local src_url="${LLM_SYNC_FROM:-https://plum.ololo.dev}"
  local src_token
  src_token="$(python3 - "$src_url" <<'EOF'
import sys, tomllib, pathlib
p = pathlib.Path.home() / ".config/ololo/credentials.toml"
d = tomllib.load(p.open("rb"))
host = sys.argv[1]
for prof in d.values():
    if isinstance(prof, dict) and prof.get("server_url", "").rstrip("/") == host.rstrip("/"):
        print(prof["token"]); break
else:
    sys.exit(1)
EOF
)" || { echo "no stored credentials for $src_url" >&2; exit 1; }
  python3 "$ROOT/scripts/llm-sync.py" \
    --from "$src_url" --from-token "$src_token" \
    --to "$BASE_URL" --to-token "$(profile_token)" "$@"
}

cmd="${1:-}"
shift || true
case "$cmd" in
  up) up "${1:-}" ;;
  start) start_session "$@" ;;
  seeds) play_env; exec "$ROOT/scripts/push-seeds.sh" dev --url "$BASE_URL" "$@" ;;
  llm-sync) llm_sync "$@" ;;
  status) status ;;
  stop) stop ;;
  *) sed -n '2,20p' "$0"; exit 1 ;;
esac
