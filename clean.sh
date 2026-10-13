#!/usr/bin/env bash
# clean.sh — stop what this repo starts, then clear runtime state.
#
#   ./clean.sh          check, then clean if safe
#   ./clean.sh --yes    skip the confirmation prompt
#
# Preserved: your config (~/.config/opencode), your sessions and database
# Cleaned:   target/ build artefacts, stray MCP servers we spawned
set -euo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "$0")" && pwd)"

R=$'\033[0m'; G=$'\033[32m'; Y=$'\033[33m'; RED=$'\033[31m'; B=$'\033[1m'

log()  { printf '%s[clean]%s %s\n' "$B" "$R" "$*"; }
ok()   { printf '%s  ok %s %s\n' "$G" "$R" "$*"; }
warn() { printf '%s  ! %s %s\n' "$Y" "$R" "$*"; }
bad()  { printf '%s  X %s %s\n' "$RED" "$R" "$*"; }
die()  { printf '%s[clean] ERROR%s %s\n' "$RED" "$R" "$*" >&2; exit 1; }

ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    -h|--help) sed -n '1,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    --yes|-y)  ASSUME_YES=1 ;;
    *) die "unknown flag: $arg (see --help)" ;;
  esac
done

BIN="$ROOT/target/release/opencode"

log "checking for running opencode from this repo"

OURS="$(pgrep -fl "$BIN" 2>/dev/null || true)"
if [ -n "$OURS" ]; then
  printf '%s\n' "$OURS" | sed 's/^/    /'
  bad "opencode from this repo is still running — stop it before cleaning"
  exit 1
fi
ok "no opencode from this repo is running"

# Anything else called opencode belongs to another install; never touch it.
FOREIGN="$(pgrep -x opencode 2>/dev/null || true)"
if [ -n "$FOREIGN" ]; then
  warn "other opencode processes exist and will be left alone:"
  printf '%s\n' "$FOREIGN" | sed 's/^/    /'
fi

if [ "$ASSUME_YES" -eq 0 ]; then
  printf 'clean build artefacts in target/? [y/N] '
  read -r reply </dev/tty || reply=""
  case "$reply" in
    y|Y|yes|YES) ;;
    *) log "cancelled"; exit 0 ;;
  esac
fi

if [ -d "$ROOT/target" ]; then
  rm -rf "$ROOT/target"
  ok "removed target/"
else
  ok "no target/ to remove"
fi

STAMP="$ROOT/target/.opr-build-stamp"
[ -f "$STAMP" ] && rm -f "$STAMP" 2>/dev/null || true

log "done. Run ./install.sh to rebuild."