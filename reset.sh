#!/usr/bin/env bash
# reset.sh — clear runtime state WITHOUT losing config, token or the Cloudflare domain.
#
# Usage:
#   ./reset.sh [--up] [--no-build]
#
#   (no flag)   stop the control plane + Rust harness, then clear runtime data.
#   --up        after reset, restart with ./start.sh
#   --no-build  skip the cargo rebuild (clean only).
#
# NO DOCKER. There is no image to remove and no compose stack to tear down — the only
# artefacts are a bun process, a cargo binary and files under control/data.
#
# Preserved:
#   .env (ELIA_AUTH_TOKEN — the token IS the domain binding),
#   control/config/subworkers.json, control/config/server.json,
#   control/config/tunnel.json, control/config/tunnel.token (chmod 600).
# Cleaned:
#   control/data/{runs.json,frames/*,prompts/*,server.pid,server.log},
#   server/data/ (the repo-local Rust DB + provider auth — recreated on next start).
set -euo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "$0")" && pwd)"
CONTROL_DIR="$ROOT/control"
SERVER_DIR="$ROOT/server"

log() { printf '[reset] %s\n' "$*"; }
die() { printf '[reset] ERROR: %s\n' "$*" >&2; exit 1; }

UP=0
BUILD=1
for arg in "$@"; do
  case "$arg" in
    --up) UP=1 ;;
    --no-build) BUILD=0 ;;
    -h|--help)
      sed -n '1,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    --docker|--with-tunnel)
      die "docker is not supported in this repo (native Rust harness). Drop the flag."
      ;;
    *) die "unknown flag: $arg (see --help)" ;;
  esac
done

# ── 0. Preflight: token must exist or the domain can't come back ──
if [ ! -s "$CONTROL_DIR/config/tunnel.token" ] && [ ! -s "$CONTROL_DIR/config/tunnel.json" ]; then
  log "WARN: control/config/tunnel.token and tunnel.json both missing/empty — domain will NOT survive."
  log "Restore it (POST /tunnel/setup) before --up, or continue for a clean slate."
fi

# ── 1. Stop control plane + Rust harness child ──
log "stopping control plane and harness…"
"$ROOT/stop.sh" || log "WARNING: stop.sh reported a problem — continuing with the clean."

# ── 2. Clear runtime data, keep configs + tokens ──
rm -f "$CONTROL_DIR/data/runs.json" "$CONTROL_DIR/data/server.pid" "$CONTROL_DIR/data/server.log"
rm -f "$CONTROL_DIR/data/frames/"*.jsonl 2>/dev/null || true
rm -f "$CONTROL_DIR/data/prompts/"*.txt 2>/dev/null || true
rm -f "$SERVER_DIR/data/runs.jsonl" 2>/dev/null || true
# The repo-local Rust DB + provider auth live under server/data only. This path is
# reachable because OPENCODE_PATHS_ROOT points there — never ~/Library/Application Support.
rm -rf "$SERVER_DIR/data/opencode" 2>/dev/null || true
log "cleared runs/frames/prompts/pid/log + repo-local server/data/opencode (configs + tokens kept)."

# ── 3. Rebuild (unless skipped) ──
if [ "$BUILD" -eq 1 ]; then
  command -v cargo >/dev/null 2>&1 || die "cargo not found — re-run with --no-build."
  log "rebuilding the Rust harness…"
  (cd "$SERVER_DIR" && cargo build --release --bin opencode)
  log "rebuilt opencode."
fi

if [ "$UP" -eq 1 ]; then
  log "restarting…"
  exec "$ROOT/start.sh"
fi

log "reset done. Token + configs preserved."
echo "  Start: ./start.sh"