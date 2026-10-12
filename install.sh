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

BIN_SRC="$SERVER_DIR/target/release/opencode"
BIN_DST="$HOME/.local/bin/opr"
STAMP="$SERVER_DIR/target/.opr-build-stamp"

FORCE=0
for arg in "$@"; do
  case "$arg" in
    -h|--help)
      sed -n '1,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    --force) FORCE=1 ;;
    *) die "unknown flag: $arg (see --help)" ;;
  esac
done

command -v cargo >/dev/null 2>&1 || die "cargo not found — install Rust: https://rustup.rs"

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

mkdir -p "$CONTROL_DIR/data" "$CONTROL_DIR/config" "$HOME/.local/bin"
log "ensured control/data + control/config exist."

current_stamp() {
  {
    find "$SERVER_DIR/crates" -name '*.rs' -type f -print0 2>/dev/null
    printf '%s\0' "$SERVER_DIR/Cargo.toml" "$SERVER_DIR/Cargo.lock"
  } | sort -z | xargs -0 shasum 2>/dev/null | shasum | cut -d' ' -f1
}

WANT="$(current_stamp)"
HAVE="$(cat "$STAMP" 2>/dev/null || true)"

if [ "$FORCE" -eq 0 ] && [ -x "$BIN_SRC" ] && [ "$WANT" = "$HAVE" ]; then
  log "code unchanged — skipping build"
else
  [ "$FORCE" -eq 1 ] && log "forced rebuild"
  log "building the Rust harness (release)"
  (cd "$SERVER_DIR" && cargo build --release --bin opencode)
  [ -x "$BIN_SRC" ] || die "build finished but $BIN_SRC is missing"
  printf '%s' "$WANT" > "$STAMP"
  log "build ok"
fi

cat > "$BIN_DST" <<EOF
#!/usr/bin/env bash
# opr — opencode from ${ROOT}
# A known subcommand is forwarded verbatim, flags and all. Shifting it off (the earlier version
# did \`run) shift\`) handed opencode the subcommand's arguments as if they were global, so
# \`opr run --model x "p"\` became \`opencode --model x "p"\` and failed.
case "\${1:-}" in
  serve|run|tui|web|acp|session|stats|db|config|auth|agent|debug|mcp|export|import|upgrade|uninstall|github|models|generate|version|attach)
    exec "$SERVER_DIR/target/release/opencode" "\$@"
    ;;
  -v|--version)
    exec "$SERVER_DIR/target/release/opencode" --version
    ;;
  *)
    exec "$SERVER_DIR/target/release/opencode" tui "\$@"
    ;;
esac
EOF
chmod +x "$BIN_DST"
log "installed $BIN_DST (opr: tui / serve / run)"

# `setbon "p"`, `googlebot "p"`, `gilfoyle "p"` each run one prompt as that named agent.
# On a case-insensitive filesystem (macOS default) `Setbon`/`GoogleBot`/`Gilfoyle` resolve to
# these same files, so both spellings work.
make_agent_alias() {
  local name="$1"
  cat > "$HOME/.local/bin/$name" <<EOF
#!/usr/bin/env bash
# $name — run one prompt to completion as the "$name" agent.
if [ \$# -eq 0 ]; then
  echo "usage: $name \"<prompt>\"" >&2
  exit 2
fi
exec "$BIN_DST" run --agent "$name" "\$@"
EOF
  chmod +x "$HOME/.local/bin/$name"
  log "installed $HOME/.local/bin/$name ($name: run one prompt)"
}

for agent_alias in gilfoyle setbon googlebot; do
  make_agent_alias "$agent_alias"
done

if [ ! -L "$ROOT/config-xdg/opencode" ]; then
  mkdir -p "$ROOT/config-xdg"
  ln -sfn ../config-home "$ROOT/config-xdg/opencode"
  log "linked config-xdg/opencode -> config-home"
fi

case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) log "note: $HOME/.local/bin is not in your PATH — add it to run opr from anywhere" ;;
esac

log "done. Try: opr"