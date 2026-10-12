#!/usr/bin/env bash
# install.sh — one-time setup for subworker-opencode-rust (native, no container).
#
# Usage:
#   ./install.sh
#
# What it does:
#   1. Checks prereqs: bun >= 1.3.14 (fatal), cargo (fatal), jq (fatal).
#   2. Runs `bun install` in control/ (upstream manifest lives there).
#   3. Builds the Rust server once into server/target/release (cargo build --release).
#   4. Mints ELIA_AUTH_TOKEN into the REPO-ROOT .env (chmod 600) if absent, else keeps it.
#   5. Ensures control/data + control/config exist.
#
# NO DOCKER. The harness is a native Rust binary; there is no image and no compose file.
#
# Next steps after install:
#   1. Edit control/config/subworkers.json
#   2. Run ./start.sh
#   3. curl http://127.0.0.1:5680/health
set -euo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "$0")" && pwd)"
CONTROL_DIR="$ROOT/control"
SERVER_DIR="$ROOT/server"
# Repo-root .env — start.sh sources this exact file, so the token is minted HERE
# and never into control/.env (two files = two tokens = silent auth failures).
ENV_FILE="$ROOT/.env"
MIN_BUN="1.3.14"

log() { printf '[install] %s\n' "$*"; }
die() { printf '[install] ERROR: %s\n' "$*" >&2; exit 1; }

for arg in "$@"; do
  case "$arg" in
    -h|--help)
      sed -n '1,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    --docker|--no-docker)
      die "docker is not supported in this repo (native Rust harness). Drop the flag."
      ;;
    *) die "unknown flag: $arg (see --help)" ;;
  esac
done

command -v bun >/dev/null 2>&1 || die "bun not found — install it: curl -fsSL https://bun.sh/install | bash"
BUN_VER="$(bun --version)"
log "bun $BUN_VER"
if [ "$(printf '%s\n%s\n' "$MIN_BUN" "$BUN_VER" | sort -V | head -n1)" != "$MIN_BUN" ]; then
  die "bun >= $MIN_BUN required (found $BUN_VER) — upgrade: bun upgrade"
fi

command -v cargo >/dev/null 2>&1 || die "cargo not found — install Rust: https://rustup.rs"
log "cargo $(cargo --version | awk '{print $2}')"

command -v jq >/dev/null 2>&1 || die "jq not found — brew install jq"

if [ -f "$ENV_FILE" ] && grep -q '^ELIA_AUTH_TOKEN=.\+' "$ENV_FILE" 2>/dev/null; then
  log ".env already has ELIA_AUTH_TOKEN — keeping existing token."
else
  command -v openssl >/dev/null 2>&1 || die "openssl not found — required to mint ELIA_AUTH_TOKEN"
  TOKEN="$(openssl rand -hex 32)"
  if [ -f "$ENV_FILE" ]; then
    grep -v '^ELIA_AUTH_TOKEN=' "$ENV_FILE" > "$ENV_FILE.tmp" || true
    mv "$ENV_FILE.tmp" "$ENV_FILE"
    printf 'ELIA_AUTH_TOKEN=%s\n' "$TOKEN" >> "$ENV_FILE"
  else
    printf 'ELIA_AUTH_TOKEN=%s\n' "$TOKEN" > "$ENV_FILE"
  fi
  chmod 600 "$ENV_FILE"
  log "minted ELIA_AUTH_TOKEN into .env (mode 600)."
fi

log "installing deps in control/: $CONTROL_DIR"
(cd "$CONTROL_DIR" && bun install)

mkdir -p "$CONTROL_DIR/data" "$CONTROL_DIR/config"
log "ensured control/data + control/config exist."

log "building the Rust harness (release): server/target/release/opencode"
(cd "$SERVER_DIR" && cargo build --release --bin opencode)

log "rust harness built: $(ls -1 "$SERVER_DIR"/target/release/opencode 2>/dev/null || echo '<binary not found>')"

cat <<EOF
[install] done. Next steps:
  1. Edit control/config/subworkers.json
  2. Run ./start.sh
  3. curl http://127.0.0.1:$(jq -r '.port // 5680' "$CONTROL_DIR/config/server.json" 2>/dev/null || echo 5680)/health
EOF