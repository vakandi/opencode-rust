#!/usr/bin/env bash
# stop.sh — stop the control plane and its Rust harness child. Data is preserved.
#
# Usage:
#   ./stop.sh
#
#   Kills the pid in control/data/server.pid (only if the process command matches
#   `bun.*server.ts`, else warns and keeps it) and removes the pidfile, then kills any
#   Rust `opencode serve` child whose OPENCODE_PATHS_ROOT points inside this repo.
#
# NO DOCKER. There is no compose stack and no container to tear down.
set -euo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "$0")" && pwd)"
CONTROL_DIR="$ROOT/control"
PID_FILE="$CONTROL_DIR/data/server.pid"

log() { printf '[stop] %s\n' "$*"; }
die() { printf '[stop] ERROR: %s\n' "$*" >&2; exit 1; }

for arg in "$@"; do
  case "$arg" in
    -h|--help)
      sed -n '1,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    --with-colima|--docker|--with-tunnel)
      die "docker is not supported in this repo (native Rust harness). Drop the flag."
      ;;
    *) die "unknown flag: $arg (see --help)" ;;
  esac
done

if [ -f "$PID_FILE" ]; then
  PID="$(tr -d '[:space:]' < "$PID_FILE" 2>/dev/null || true)"
  if [ -z "${PID:-}" ]; then
    log "WARNING: pidfile $PID_FILE is empty — removing it."
    rm -f "$PID_FILE"
  elif kill -0 "$PID" 2>/dev/null; then
    if ps -p "$PID" -o command= 2>/dev/null | grep -q 'bun.*server\.ts'; then
      log "stopping control plane (pid $PID)…"
      kill "$PID" 2>/dev/null || true
      for _ in $(seq 1 10); do
        kill -0 "$PID" 2>/dev/null || break
        sleep 1
      done
      if kill -0 "$PID" 2>/dev/null; then
        log "pid $PID still alive — sending SIGKILL."
        kill -9 "$PID" 2>/dev/null || true
      fi
      rm -f "$PID_FILE"
      log "control plane stopped; pidfile removed."
    else
      log "WARNING: pid $PID does not match \`bun.*server.ts\` ($(ps -p "$PID" -o command= 2>/dev/null || echo '<unknown>')) — not killing; remove $PID_FILE manually if stale."
    fi
  else
    log "stale pidfile (pid $PID not running) — removing it."
    rm -f "$PID_FILE"
  fi
else
  log "no pidfile ($PID_FILE) — no control plane to stop."
fi

# The Rust harness is spawned by the control plane, not by us, so it has no pidfile.
# Kill it only if its OPENCODE_PATHS_ROOT is provably inside THIS repo — never a
# foreign opencode, never the live one in ~/Library/Application Support.
CHILD_PIDS="$(pgrep -f 'opencode.*serve' 2>/dev/null || true)"
for pid in $CHILD_PIDS; do
  cmd="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  root_marker="$(ps eww -p "$pid" 2>/dev/null | tr ' ' '\n' | grep '^OPENCODE_PATHS_ROOT=' | head -n1 || true)"
  case "$root_marker" in
    "OPENCODE_PATHS_ROOT=$ROOT"*)
      log "stopping Rust harness child (pid $pid, $root_marker)…"
      kill "$pid" 2>/dev/null || true
      for _ in $(seq 1 10); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 1
      done
      kill -9 "$pid" 2>/dev/null || true
      ;;
    *)
      log "leaving pid $pid alone (no OPENCODE_PATHS_ROOT inside this repo): $cmd"
      ;;
  esac
done

log "control/data preserved (runs/frames/state untouched). Restart with ./start.sh."