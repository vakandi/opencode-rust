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

# Match on the binary path in EITHER form. `pgrep -f "$BIN"` alone missed every
# server started as `./target/release/opencode`, because pgrep matches the
# literal command line — so clean.sh would happily delete target/ out from under
# a live server. The release binary is also checked by inode via lsof, which
# catches a copy renamed or reached by a relative path.
OURS=""
while read -r pid; do
  [ -n "$pid" ] || continue
  cmd="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  case "$cmd" in
    *"$BIN"*|*target/release/opencode*) OURS="${OURS}${pid}"$'\n' ;;
  esac
done <<EOF
$(pgrep -f opencode 2>/dev/null || true)
EOF

if [ -z "$OURS" ] && [ -f "$BIN" ]; then
  BINPID="$(lsof -t -- "$BIN" 2>/dev/null || true)"
  [ -n "$BINPID" ] && OURS="$BINPID"$'\n'
fi

if [ -n "$OURS" ]; then
  printf '%s' "$OURS" | while read -r pid; do
    [ -n "$pid" ] && printf '    pid %s  %s\n' "$pid" "$(ps -p "$pid" -o command= 2>/dev/null | cut -c1-80)"
  done
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

# Removes a tree, clearing the things that make `rm` fail with a bare
# "Permission denied": immutable/append flags (uappnd/uchg) and write bits that
# a previous partial clean may have stripped. Then it reports the specific paths
# that survived, instead of dying on set -e with no idea which ones.
# Returns 0 when the tree is gone, 1 when something is left.
remove_tree() {
  local dir="$1"
  [ -e "$dir" ] || return 0

  # macOS only: nouchg clears both immutable and append-only.
  if [ "$(uname -s)" = "Darwin" ]; then
    chflags -R nouchg "$dir" 2>/dev/null || true
  fi
  chmod -R u+rwX "$dir" 2>/dev/null || true
  rm -rf "$dir" 2>/dev/null || true

  if [ -e "$dir" ]; then
    warn "could not fully remove $dir — leftovers:"
    find "$dir" -maxdepth 3 2>/dev/null | head -10 | while read -r left; do
      printf '        %s\n' "${left#"$ROOT"/}"
    done
    # The usual cause on macOS is App Management (TCC): another app created the
    # file and this shell lacks permission to delete it until it is granted Full
    # Disk Access. Name it rather than leaving "permission denied" on its own.
    bad "grant this terminal Full Disk Access (System Settings → Privacy & Security) and re-run"
    return 1
  fi
  return 0
}

if [ -d "$ROOT/target" ]; then
  if remove_tree "$ROOT/target"; then
    ok "removed target/"
  else
    warn "target/ is still present — the rebuild below may reuse stale artefacts"
  fi
else
  ok "no target/ to remove"
fi

log "done. Run ./install.sh to rebuild."