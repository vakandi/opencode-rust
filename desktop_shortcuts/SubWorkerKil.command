#!/usr/bin/env bash
# SubWorkerKill — stop the native subworker stack, preserve sessions.
#
# Replaces the Docker version (docker-compose down on elia-subworker-srv). There is no
# container in this repo: what actually runs is the Rust server, the control plane, the
# Electron UI, and the plugin-host subprocesses the Rust loader spawns.
#
# Sessions survive because the database lives in the repo at server/data and nothing
# here deletes it. Restart with desktop_shortcuts/EliaUI.command (or ./start.sh for the
# control plane alone).

set -uo pipefail

SELF="$(realpath "$0")"
REPO="$(cd "$(dirname "$SELF")/.." && pwd)"
CHILD_PORT="$(jq -r '.childPort // 4096' "$REPO/control/config/server.json" 2>/dev/null || printf '4096')"
CONTROL_PORT="$(jq -r '.port // 5680' "$REPO/control/config/server.json" 2>/dev/null || printf '5680')"

log() { printf '[SubWorkerKill] %s\n' "$*"; }

# ── 1. Ask the control plane to mark every agent stopped, so no run is left ──
#    "running" in its records and the UI shows the truth.
if curl -sf --max-time 3 "http://127.0.0.1:$(jq -r '.port // 5680' "$REPO/control/config/server.json" 2>/dev/null || printf '5680')/status" >/dev/null 2>&1; then
  names=$(curl -sf --max-time 5 "http://127.0.0.1:$(jq -r '.port // 5680' "$REPO/control/config/server.json" 2>/dev/null || printf '5680')/status" \
    | jq -r '.subworkers[] | select(.enabled == true) | .name' 2>/dev/null || true)
  CONTROL_PORT="$(jq -r '.port // 5680' "$REPO/control/config/server.json" 2>/dev/null || printf '5680')"
  for name in $names; do
    curl -sf --max-time 5 -X POST "http://127.0.0.1:$CONTROL_PORT/disable/$name" >/dev/null 2>&1 \
      && log "disabled $name" || log "could not disable $name"
  done
else
  log "control plane not running — skipping agent disable"
fi

# ── 2. tmux sessions ───────────────────────────────────────────────────────
for session in elia-ui opencode-serve; do
  if tmux has-session -t "$session" 2>/dev/null; then
    tmux kill-session -t "$session" 2>/dev/null || true
    log "killed tmux session '$session'"
  else
    log "tmux session '$session' not present"
  fi
done

# ── 3. Native processes, scoped to this repo ───────────────────────────────
# pgrep -f matches the repo path, so another project's opencode is never touched.
kill_matching() {
  local label="$1" pattern="$2"
  local pids
  pids="$(pgrep -f "$pattern" 2>/dev/null || true)"
  if [[ -z "$pids" ]]; then
    log "$label: none running"
    return
  fi
  # shellcheck disable=SC2086
  kill $pids 2>/dev/null || true
  sleep 1
  pids="$(pgrep -f "$pattern" 2>/dev/null || true)"
  if [[ -n "$pids" ]]; then
    # shellcheck disable=SC2086
    kill -9 $pids 2>/dev/null || true
  fi
  log "$label: stopped"
}

# plugin-host first: these are the subprocesses the Rust plugin loader waits on, and a
# surviving one makes the next boot hang before it binds its port.
kill_matching "plugin-host"  "plugin-host.ts"
kill_matching "electron UI"  "$REPO/ui_electron"
# Launched as "./server/target/release/opencode", so the cmdline holds a RELATIVE
# path and an absolute-path pgrep -f never matched it.
kill_matching "Rust server"  "server/target/release/opencode serve"
# Also relative ("bun server.ts" from the control dir).
kill_matching "control plane" "control/server\.ts|bun server\.ts"

# ── 4. Wait for the ports to actually free ─────────────────────────────────
# Returning while a socket is still in TIME_WAIT/held makes the next EliaUI launch die
# with EADDRINUSE, which looks like a broken launcher rather than a race.
for port in "$CHILD_PORT" "$CONTROL_PORT"; do
  for i in $(seq 1 20); do
    lsof -ti :"$port" >/dev/null 2>&1 || break
    sleep 0.5
  done
  if lsof -ti :"$port" >/dev/null 2>&1; then
    log "WARNING: :$port still held by pid $(lsof -ti :"$port" | tr '\n' ' ')"
  fi
done

# ── 5. Verify, then report honestly ────────────────────────────────────────
sleep 1
still_up=0
curl -sf --max-time 2 "http://127.0.0.1:$CHILD_PORT/health" >/dev/null 2>&1 && {
  log "WARNING: :$CHILD_PORT still answering"; still_up=1; }

if [[ "$still_up" == "0" ]]; then
  log "stack stopped · sessions preserved in server/data"
  log "restart: desktop_shortcuts/EliaUI.command"
else
  log "something still holds :$CHILD_PORT — check: lsof -ti :$CHILD_PORT"
fi
