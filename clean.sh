#!/usr/bin/env bash
# clean.sh — stop everything this repo starts, then clear runtime state.
#
#   ./clean.sh            check, then clean if safe
#   ./clean.sh --yes      skip the confirmation prompt
#   ./clean.sh --up       restart with ./start.sh after cleaning
#
# Refuses to clean while an agent run is live. Warns (red) before killing any
# opencode that is NOT ours, and clears MCP residue our server spawned.
#
# Preserved: .env (ELIA_AUTH_TOKEN), control/config/*
# Cleaned:   control/data/{runs.json,frames,prompts,cache,runtime,*.log,*.pid}, server/data/
set -euo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "$0")" && pwd)"
DATA="$ROOT/control/data"
SERVER_DIR="$ROOT/server"

R=$'\033[0m'; G=$'\033[32m'; Y=$'\033[33m'; RED=$'\033[31m'; B=$'\033[1m'
TAB=$'\t'

log()  { printf '%s[clean]%s %s\n' "$B" "$R" "$*"; }
ok()   { printf '%s  ok %s %s\n' "$G" "$R" "$*"; }
warn() { printf '%s  ! %s %s\n' "$Y" "$R" "$*"; }
bad()  { printf '%s  X %s %s\n' "$RED" "$R" "$*"; }
die()  { printf '%s[clean] ERROR%s %s\n' "$RED" "$R" "$*" >&2; exit 1; }

ASSUME_YES=0; UP=0
for arg in "$@"; do
  case "$arg" in
    -h|--help) sed -n '1,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -y|--yes)  ASSUME_YES=1 ;;
    --up)      UP=1 ;;
    *) die "unknown flag: $arg (see --help)" ;;
  esac
done

# --- 1. agents running? then there is nothing to clean -------------------------
running_agents() {
  jq -r '
    (if type=="object" and has("runs") then .runs
     elif type=="object" then . else [] end)
    | .[]? | select(.status == "running") | .name // .session_id
  ' "$DATA/runs.json" 2>/dev/null | grep -v '^$'
}

if AGENTS="$(running_agents)"; then
  if [ -n "$AGENTS" ]; then
    printf '\n%s%s AGENTS RUNNING — NOTHING TO CLEAN %s\n' "$B" "$Y" "$R"
    printf '%s\n' "$AGENTS" | while read -r a; do warn "still running: $a"; done
    printf '\n%sStop them first (./stop.sh) or wait for completion.%s\n\n' "$Y" "$R"
    exit 0
  fi
fi

# --- 2. our own opencode rust servers ------------------------------------------
OURS="$(pgrep -f "$SERVER_DIR/target/release/opencode serve" 2>/dev/null || true)"
COUNT=0; [ -n "$OURS" ] && COUNT="$(printf '%s\n' "$OURS" | grep -c . || true)"

printf '\n%s[clean]%s opencode rust servers: %s\n' "$B" "$R" "$COUNT"
if [ "$COUNT" -eq 0 ]; then
  ok "none running"
elif [ "$COUNT" -eq 1 ]; then
  ok "exactly one — $OURS"
else
  bad "$COUNT running — keeping the oldest, killing the rest"
  printf '%s\n' "$OURS" | tail -n +2 | while read -r pid; do
    kill -TERM "$pid" 2>/dev/null && warn "killed duplicate pid $pid" || true
  done
fi

# --- 3. foreign opencode servers (not ours) -> RED + confirm -------------------
FOREIGN=""
while read -r pid; do
  [ -n "$pid" ] || continue
  cmd="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [ -n "$cmd" ] || continue
  case "$cmd" in
    *plugin-host*|*"$ROOT"*) continue ;;
  esac
  case "$cmd" in
    *opencode*serve*|*opencode-desktop*|*"opencode serve"*) ;;
    *opencode*) continue ;;
    *) continue ;;
  esac
  FOREIGN="${FOREIGN}${pid}${TAB}$(printf '%s' "$cmd" | cut -c1-70)
"
done <<EOF
$(pgrep -f opencode 2>/dev/null || true)
EOF

if [ -n "$FOREIGN" ]; then
  printf '\n%s%s FOREIGN OPENCODE PROCESSES DETECTED %s\n' "$B" "$RED" "$R"
  printf '%s' "$FOREIGN" | while IFS="$TAB" read -r pid cmd; do
    [ -n "$pid" ] && bad "pid $pid — $cmd"
  done
  printf '\n%sThese are NOT this repo. Killing them can lose unrelated work.%s\n' "$RED" "$R"
  if [ "$ASSUME_YES" -eq 0 ]; then
    printf '%sPress Enter to kill them, or Ctrl-C to abort... %s' "$B" "$R"
    read -r _ || true
  else
    printf '%s--yes given: killing.%s\n' "$Y" "$R"
  fi
  printf '%s' "$FOREIGN" | while IFS="$TAB" read -r pid _; do
    [ -n "$pid" ] || continue
    kill -TERM "$pid" 2>/dev/null && ok "killed pid $pid" || warn "could not kill pid $pid"
  done
  sleep 1
  printf '%s' "$FOREIGN" | while IFS="$TAB" read -r pid _; do
    [ -n "$pid" ] || continue
    kill -KILL "$pid" 2>/dev/null && warn "force-killed pid $pid" || true
  done
else
  ok "no foreign opencode processes"
fi

# --- 4. MCP residue spawned by our server --------------------------------------
RESIDUE="$(pgrep -f '@playwright/mcp|undetected-browser|personal_browser|parallel-browser-mcp|mcp\.js' 2>/dev/null || true)"
if [ -n "$RESIDUE" ]; then
  printf '\n%s[clean] MCP residue (left behind by agent runs):%s\n' "$B" "$R"
  printf '%s\n' "$RESIDUE" | while read -r pid; do
    [ -n "$pid" ] || continue
    kill -TERM "$pid" 2>/dev/null && ok "killed mcp residue pid $pid" || warn "could not kill pid $pid"
  done
  sleep 1
  printf '%s\n' "$RESIDUE" | while read -r pid; do
    [ -n "$pid" ] || continue
    kill -KILL "$pid" 2>/dev/null || true
  done
else
  ok "no MCP residue"
fi

# --- 5. runtime data -----------------------------------------------------------
[ -f "$DATA/server.pid" ] && kill -TERM "$(cat "$DATA/server.pid" 2>/dev/null)" 2>/dev/null || true

rm -rf "$DATA/frames" "$DATA/prompts" "$DATA/cache" "$DATA/runtime" 2>/dev/null || true
rm -f  "$DATA/runs.json" "$DATA/server.pid" "$DATA"/*.log 2>/dev/null || true
rm -rf "$SERVER_DIR/data/opencode" 2>/dev/null || true
mkdir -p "$DATA/frames" "$DATA/prompts"
ok "runtime data cleared (config + .env untouched)"

if [ "$UP" -eq 1 ]; then
  log "restarting via ./start.sh"
  "$ROOT/start.sh"
fi
