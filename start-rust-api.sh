#!/usr/bin/env bash
# Start only the RustingOpenCode API server.
# Run in a separate terminal to see API logs. Then run start-web-app.sh in another terminal.

set -e
cd "$(dirname "$0")"

# Same containment as start.sh. Without these the Rust server loads the GLOBAL
# ~/.config/opencode + ~/.omo, so opr and the TypeScript `opencode` fight over config.
ROOT="$(cd .. && pwd)"
export OPENCODE_PATHS_ROOT="$PWD/data"
export OMP_AGENT_DIR="$ROOT/config-home"
export OPENCODE_CONFIG_DIR="$ROOT/config-home"
export OPENCODE_CONFIG_DIR_ISOLATED=1
mkdir -p "$OPENCODE_PATHS_ROOT/opencode" "$OPENCODE_PATHS_ROOT/opencode/data"

RUST_PORT="${RUST_PORT:-4096}"

echo "Building opencode-cli (ensures /path returns worktree for E2E)..."
cargo build -p opencode-cli

echo "Starting Rust API server on http://127.0.0.1:${RUST_PORT}"
echo "Logs below. Use Ctrl+C to stop."
echo "---"
./target/debug/opencode serve --port "$RUST_PORT" --hostname 127.0.0.1
