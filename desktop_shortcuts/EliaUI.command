#!/bin/zsh
# EliaUI — launch the native subworker stack (no Docker) + the Electron UI.
#
# This file used to be a symlink into ~/EliaAI/setup/desktop_shortcuts and drove the
# retired Docker stack (elia-subworker-srv on :5656, plus the Discord bot). This repo
# has no container: the Rust server owns :4096, the control plane owns :5680, and the
# UI lives in ./ui_electron.
#
# Panels (top → bottom):
#   0: Control plane (:5680) — scheduler, status, triggers
#   1: Rust server log        — the process itself lives in session 'opencode-serve'
#   2: Electron UI            — this repo's ui_electron
#
# The proxy starts DISABLED so provider traffic egresses on this machine's real IP
# (turn your VPN on first). Re-enable afterwards with: ./proxy.sh on

set -uo pipefail

REPO="$(cd "$(dirname "${0:A}")/.." && pwd)"
SESSION="elia-ui"
SERVER_SESSION="opencode-serve"
CHILD_PORT="$(jq -r '.childPort // 4096' "$REPO/control/config/server.json" 2>/dev/null || printf '4096')"
CONTROL_PORT="$(jq -r '.port // 5680' "$REPO/control/config/server.json" 2>/dev/null || printf '5680')"
UI_DIR="$REPO/ui_electron"

log() { printf '[EliaUI] %s\n' "$*"; }
die() { printf '[EliaUI] ERROR: %s\n' "$*" >&2; exit 1; }

healthy() { curl -sf --max-time 3 "$1/health" >/dev/null 2>&1; }

command -v tmux >/dev/null 2>&1 || die "tmux not found"
command -v bun   >/dev/null 2>&1 || die "bun not found — run ./install.sh first"
[[ -d "$UI_DIR" ]] || die "missing $UI_DIR"
[[ -x "$REPO/start.sh" ]] || die "missing $REPO/start.sh"
mkdir -p "$REPO/control/data"
RUST_LOG="$REPO/control/data/rust-server.log"
touch "$RUST_LOG"

# ── 1. Rust server :4096, in its own tmux session so a crash cannot take the UI ──
# down. start.sh only launches the control plane; nothing else starts this child, so
# the team path 404s without this step.
if ! healthy "http://127.0.0.1:$CHILD_PORT"; then
  if ! tmux has-session -t "$SERVER_SESSION" 2>/dev/null; then
    log "starting Rust server on :$CHILD_PORT (session '$SERVER_SESSION')"
    # Built with printf rather than a heredoc: a nested heredoc inside the launcher got
    # interpolated at generation time and produced an EMPTY file, so the server never
    # started. No interpolation layer means no such failure.
    RUST_LAUNCHER="$REPO/control/data/serve-rust.sh"
    mkdir -p "$(dirname "$RUST_LAUNCHER")"
    # MUST be outside the repo. The plugin loader does find_up(".opencode") from the cwd,
    # so a runtime dir inside the repo still reaches the repo-root .opencode/ and pulls in
    # the oh-my-openagent TS plugin, whose handshake never completes — the server then waits
    # forever and never binds :4096. Verified: repo-internal cwd hangs, this one binds in 1.5s.
    RUNTIME_DIR="${TMPDIR:-/tmp}/subworker-opencode-runtime"
    mkdir -p "$RUNTIME_DIR"
    printf '%s\n' \
      '#!/usr/bin/env bash' \
      'set -uo pipefail' \
      "REPO='$REPO'" \
      "export OPENCODE_PATHS_ROOT='$REPO/server/data'" \
      "export OPENCODE_CONFIG_DIR='$REPO/config-home'" \
      'export OPENCODE_CONFIG_DIR_ISOLATED=1' \
      "export OMP_AGENT_DIR='$REPO/config-home'" \
      'export OPENCODE_PROXY_DISABLE=1' \
      'unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy ALL_PROXY all_proxy' \
      '# Own plugin cache: the loader otherwise reads ~/Library/Caches/opencode/plugins,' \
      '# whose TS plugins never finish their handshake and the server never binds its port.' \
      "export XDG_CACHE_HOME='$REPO/control/data/cache'" \
      'mkdir -p "$XDG_CACHE_HOME"' \
      '# Start from a neutral cwd. From the repo root the loader reads .opencode/ and pulls' \
      '# in the oh-my-openagent TS plugin, whose handshake never completes: the server then' \
      '# waits forever and NEVER binds :4096. Verified — repo root hangs, neutral cwd and' \
      '# repo root with .opencode moved away both bind in ~2s. OPENCODE_PATHS_ROOT still' \
      '# pins all writable state, so the cwd does not change where data lands.' \
      "RUNTIME_DIR='$RUNTIME_DIR'" \
      'mkdir -p "$RUNTIME_DIR"' \
      'cd "$RUNTIME_DIR" || exit 1' \
      "exec '$REPO/server/target/release/opencode' serve --port $CHILD_PORT --hostname 127.0.0.1" \
      > "$RUST_LAUNCHER"
    chmod +x "$RUST_LAUNCHER"
    tmux new-session -d -s "$SERVER_SESSION" -n "Rust" "$RUST_LAUNCHER 2>&1 | tee -a '$RUST_LOG'"
  else
    log "session '$SERVER_SESSION' exists but :$CHILD_PORT is down — killing it so it can respawn"
    tmux kill-session -t "$SERVER_SESSION" 2>/dev/null || true
    sleep 1
  fi
  for i in $(seq 1 30); do healthy "http://127.0.0.1:$CHILD_PORT" && break; sleep 1; done
  healthy "http://127.0.0.1:$CHILD_PORT" \
    && log "Rust server up on :$CHILD_PORT" \
    || log "WARNING: Rust server did not answer on :$CHILD_PORT — see control/data/rust-server.log"
fi

# ── 2. Electron dependencies (node_modules is gitignored, never committed) ──
# Never fatal: a flaky registry must not stop the Rust server and control plane from
# coming up. The UI pane then reports the failure itself and the other two panels work.
if [[ ! -d "$UI_DIR/node_modules" ]]; then
  log "installing ui_electron dependencies (first run only, needs network)…"
  if ! (cd "$UI_DIR" && npm install); then
    log "WARNING: npm install failed — the UI pane will not start."
    log "         the stack (Rust :$CHILD_PORT + control :$CONTROL_PORT) is still coming up."
    log "         retry with: cd '$UI_DIR' && npm install"
  fi
fi

# ── 3. Control plane + UI in a three-pane session ──────────────────────────
if ! tmux has-session -t "$SESSION" 2>/dev/null; then
  tmux new-session -d -s "$SESSION" -n "stack"
  tmux set-option -t "$SESSION" mouse on
  tmux set-option -t "$SESSION" mode-keys vi
  tmux set-option -s -t "$SESSION" copy-command "pbcopy"
  tmux bind-key -T copy-mode-vi MouseDrag1Pane select-pane \; send -X begin-selection
  tmux bind-key -T copy-mode-vi MouseDragEnd1Pane send -X copy-pipe-and-cancel "pbcopy"

  # Pane 0: control plane — owns :5680, the scheduler and every trigger
  tmux send-keys -t "$SESSION":0.0 "cd '$REPO' && ./start.sh 2>&1 | tee -a '$REPO/control/data/start.log'" Enter

  tmux split-window -v -t "$SESSION":0.0
  sleep 1
  # Pane 1: Rust server output
  tmux send-keys -t "$SESSION":0.1 "cd '$REPO' && tail -f '$RUST_LOG'" Enter

  tmux split-window -v -t "$SESSION":0.1
  sleep 1
  # Pane 2: Electron UI
  tmux send-keys -t "$SESSION":0.2 "cd '$UI_DIR' && npm start" Enter

  tmux resize-pane -t "$SESSION":0.0 -y 30%
  tmux resize-pane -t "$SESSION":0.1 -y 20%
  tmux select-pane -t "$SESSION":0.0
fi

for i in $(seq 1 30); do healthy "http://127.0.0.1:$CONTROL_PORT" && break; sleep 1; done
healthy "http://127.0.0.1:$CONTROL_PORT" \
  && log "control plane up on :$CONTROL_PORT" \
  || log "WARNING: control plane not answering on :$CONTROL_PORT — see control/data/server.log"

log "Rust :$CHILD_PORT · control :$CONTROL_PORT · provider traffic = your real IP"
log "agents:  curl -s localhost:$CONTROL_PORT/status | jq"
if [[ -z "${TMUX:-}" ]]; then
  tmux new-session -A -s "$SESSION"
else
  log "already inside tmux — attach with: tmux attach -t $SESSION"
fi
