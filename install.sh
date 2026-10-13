#!/usr/bin/env bash
# install.sh — build the opencode binary and install the `opr` wrapper.
#
# Usage:
#   ./install.sh
#   ./install.sh --force    # rebuild even when nothing changed
set -euo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "$0")" && pwd)"
BIN_DST="$HOME/.local/bin/opr"
STAMP="$ROOT/target/.opr-build-stamp"

FORCE=0
for arg in "$@"; do
  case "$arg" in
    -h|--help)
      sed -n '1,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    --force) FORCE=1 ;;
    *) printf '[install] ERROR: unknown flag: %s (see --help)\n' "$arg" >&2; exit 1 ;;
  esac
done

log() { printf '[install] %s\n' "$*"; }
die() { printf '[install] ERROR: %s\n' "$*" >&2; exit 1; }

command -v cargo >/dev/null 2>&1 || die "cargo not found — install Rust: https://rustup.rs"

BIN_SRC="$ROOT/target/release/opencode"

# The stamp fingerprints every source file, so an unchanged tree skips the rebuild.
fingerprint() {
  find "$ROOT/crates" -name '*.rs' -type f -print0 2>/dev/null
  printf '%s\0' "$ROOT/Cargo.toml" "$ROOT/Cargo.lock"
}

WANT="$(fingerprint | shasum | cut -d' ' -f1)"
HAVE=""
[ -f "$STAMP" ] && HAVE="$(cat "$STAMP" 2>/dev/null || true)"

if [ "$FORCE" -eq 1 ] || [ "$WANT" != "$HAVE" ] || [ ! -x "$BIN_SRC" ]; then
  log "building opencode (release) — first build takes a few minutes"
  (cd "$ROOT" && cargo build --release --bin opencode)
  [ -x "$BIN_SRC" ] || die "build finished but $BIN_SRC is missing"
  printf '%s' "$WANT" > "$STAMP"
  log "build ok"
else
  log "code unchanged — reusing existing build (pass --force to rebuild)"
fi

mkdir -p "$HOME/.local/bin"

cat > "$BIN_DST" <<EOF
#!/usr/bin/env bash
# opr — opencode from ${ROOT}
# A known subcommand is forwarded verbatim, flags and all. Shifting it off handed
# opencode the subcommand's arguments as if they were global, so
# \`opr run --model x "p"\` became \`opencode --model x "p"\` and failed.
case "\${1:-}" in
  serve|run|tui|web|acp|session|stats|db|config|auth|agent|debug|mcp|export|import|upgrade|uninstall|github|models|generate|version|attach)
    exec "$BIN_SRC" "\$@"
    ;;
  -v|--version)
    exec "$BIN_SRC" --version
    ;;
  *)
    exec "$BIN_SRC" tui "\$@"
    ;;
esac
EOF
chmod +x "$BIN_DST"
log "installed $BIN_DST"

# `gilfoyle "p"` runs one prompt to completion as that named agent.
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
  log "installed $HOME/.local/bin/$name"
}

for agent_alias in gilfoyle setbon googlebot; do
  make_agent_alias "$agent_alias"
done

case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) log "note: $HOME/.local/bin is not in your PATH — add it to run opr from anywhere" ;;
esac

log "done. Try: opr"