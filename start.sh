#!/usr/bin/env bash
# start.sh — start the control plane (control/server.ts) + its Rust harness child.
#
# Usage:
#   ./start.sh
#
# Native only. There is no container, no compose file and no image: the harness is
# the cargo-built Rust binary in server/target/release/opencode.
#
# The control plane binds 127.0.0.1:<port> where <port> comes from
# control/config/server.json. Identity is asserted BEFORE we declare health: a server
# on that port that answers with a different `repo` is somebody else's, and we refuse
# to touch it rather than reporting a silent green.
set -euo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "$0")" && pwd)"
CONTROL_DIR="$ROOT/control"
SERVER_DIR="$ROOT/server"
# Repo-root .env — the token must never come from a shared/live copy.
ENV_FILE="$ROOT/.env"
PID_FILE="$CONTROL_DIR/data/server.pid"
LOG_FILE="$CONTROL_DIR/data/server.log"
# Derive the port from the same file control/server.ts binds, so the health gate can
# never probe a different port than the server owns.
SERVER_JSON="$CONTROL_DIR/config/server.json"
PORT="$(jq -r '.port // 5680' "$SERVER_JSON" 2>/dev/null || printf '5680')"
BASE="http://127.0.0.1:$PORT"
REPO_ID="subworker-opencode-rust"

log() { printf '[start] %s\n' "$*"; }
die() { printf '[start] ERROR: %s\n' "$*" >&2; exit 1; }

for arg in "$@"; do
  case "$arg" in
    -h|--help)
      sed -n '1,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    --docker|--no-docker|--with-tunnel|--with-colima)
      die "docker is not supported in this repo (native Rust harness). Drop the flag."
      ;;
    *) die "unknown flag: $arg (see --help)" ;;
  esac
done

[ -f "$ENV_FILE" ] || die ".env not found at $ENV_FILE — run ./install.sh first."
set -a; . "$ENV_FILE"; set +a
[ -n "${ELIA_AUTH_TOKEN:-}" ] || die "ELIA_AUTH_TOKEN is empty in $ENV_FILE — re-run ./install.sh."

# Every path the Rust harness touches resolves through this root, inside this repo.
# This is the whole containment story: no $HOME override, no ~/Library/Application Support.
export OPENCODE_PATHS_ROOT="$SERVER_DIR/data"
# Personas live in config-home/agents, NOT in the live ~/.config/opencode.
export OMP_AGENT_DIR="$ROOT/config-home"
# Exclusive config: without this the global ~/.config/opencode is loaded too and its
# plugins block startup before the listener binds. See opencode-config loader.rs.
export OPENCODE_CONFIG_DIR="$ROOT/config-home"
export OPENCODE_CONFIG_DIR_ISOLATED=1
mkdir -p "$OPENCODE_PATHS_ROOT/opencode" "$OPENCODE_PATHS_ROOT/opencode/data"

# Drop inherited proxy variables for the harness.
#
# The desktop shell exports HTTP_PROXY/HTTPS_PROXY -> 127.0.0.1:18891, which is a
# residential proxy owned by a *different* app (omp). It is alive but refuses
# outbound HTTPS from this context, so every provider call through it fails with a
# network error while the same request succeeds directly. Verified: direct
# https://opencode.ai/zen/v1/models = 200 in 0.5s; via the proxy = connection
# failure. Inheriting it silently breaks every agent run.
#
# Still unset now that we ship a native proxy: the server publishes its OWN URL into the
# environment before building any provider client, so these get overwritten. Unsetting
# first means a failed native start falls back to DIRECT (works) rather than to omp's
# dead 18891 (instant outage). Never point the harness at a foreign listener.
unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy ALL_PROXY all_proxy

# Repo-local pool: gitignored (credentials), and deliberately NOT
# ~/.config/opencode/plugins/proxies.txt, whose |last: history belongs to another
# opencode instance. Ports start at 19400 to stay clear of the TS fleet's 18888-19387.
PROXY_POOL="$OPENCODE_PATHS_ROOT/opencode/proxy-pool.txt"
if [[ -s "$PROXY_POOL" ]]; then
  log "proxy pool=$(wc -l <"$PROXY_POOL" | tr -d ' ') entries rotate=120s oldest-first"
else
  log "WARN no proxy pool at $PROXY_POOL - provider traffic will go direct"
fi

# Returns 0 healthy · 1 unreachable · 2 IDENTITY MISMATCH (never treated as "not running")
server_healthy() {
  local payload repo
  payload="$(curl -sf "$BASE/health" 2>/dev/null)" || return 1
  repo="$(printf '%s' "$payload" | jq -r '.repo // empty' 2>/dev/null)" || repo=""
  if [ "$repo" != "$REPO_ID" ]; then
    printf '[start] ERROR: :%s answered as repo=%s, expected %s — a different server owns this port.\n' \
      "${PORT:-?}" "${repo:-<none>}" "$REPO_ID" >&2
    return 2
  fi
  curl -sf -H "Authorization: Bearer $ELIA_AUTH_TOKEN" "$BASE/server/health" 2>/dev/null \
    | tr -d '[:space:]' | grep -q '"health_status":"healthy"'
}

if server_healthy; then
  log "already running and healthy on :$PORT (repo=$REPO_ID) — leaving it alone."
  exit 0
else
  rc=$?
  # rc=2 is a foreign server holding the port: fail loudly, never fall through to a
  # start that cannot bind, and never exit 0 claiming someone else's server is ours.
  # Must be `if`, not `[ ] && die` — under `set -e` a false test aborts the whole script.
  if [ "$rc" -eq 2 ]; then
    die "repo identity mismatch on :$PORT — refusing to touch another server's port."
  fi
fi

health_gate() {
  # $1 = attempts, $2 = label
  local i out
  for i in $(seq 1 "$1"); do
    if server_healthy; then
      log "$2 healthy."
      return 0
    fi
    sleep 1
  done
  out="$(curl -sf -H "Authorization: Bearer $ELIA_AUTH_TOKEN" "$BASE/server/health" 2>&1 || true)"
  log "health gate failed; /server/health last said: ${out:-<unreachable>}"
  return 1
}

# Refuse to double-start our own pidfile owner.
if [ -f "$PID_FILE" ]; then
  PID="$(tr -d '[:space:]' < "$PID_FILE" 2>/dev/null || true)"
  if [ -n "${PID:-}" ] && kill -0 "$PID" 2>/dev/null; then
    if ps -p "$PID" -o command= 2>/dev/null | grep -q 'bun.*server\.ts'; then
      die "pidfile $PID_FILE points at live bun server.ts (pid $PID) but :$PORT is unhealthy — inspect $LOG_FILE, then ./stop.sh."
    fi
    log "WARNING: stale pidfile (pid $PID not a bun server.ts process) — replacing."
  fi
fi

command -v bun >/dev/null 2>&1 || die "bun not found — run ./install.sh first."
command -v cargo >/dev/null 2>&1 || die "cargo not found — run ./install.sh first."

# Build the Rust harness if there is no release binary yet. Cheap no-op when up to date.
if [ ! -x "$SERVER_DIR/target/release/opencode" ]; then
  log "no release binary — building the Rust harness (this can take a few minutes)…"
  (cd "$SERVER_DIR" && cargo build --release --bin opencode)
fi
[ -x "$SERVER_DIR/target/release/opencode" ] || die "cargo build produced no opencode binary at server/target/release/opencode"

mkdir -p "$(dirname "$PID_FILE")"
log "starting control plane: nohup bun control/server.ts >> control/data/server.log 2>&1 &"
(cd "$CONTROL_DIR" && nohup bun server.ts >> "$LOG_FILE" 2>&1 & echo $! > "$PID_FILE")

for i in $(seq 1 30); do
  curl -sf "$BASE/health" >/dev/null 2>&1 && break
  [ "$i" -eq 30 ] || sleep 1
done
curl -sf "$BASE/health" >/dev/null 2>&1 || {
  log "control plane did not answer /health within 30s — tail of $LOG_FILE:"
  tail -n 30 "$LOG_FILE" 2>/dev/null || true
  exit 1
}
health_gate 60 "control plane" || {
  log "tail of $LOG_FILE:"
  tail -n 30 "$LOG_FILE" 2>/dev/null || true
  exit 1
}

log "repo=$REPO_ID port=$PORT paths_root=$OPENCODE_PATHS_ROOT"