#!/usr/bin/env bash
# RAM — stop everything running for this repo and hand the memory back to macOS.
#
# Same teardown as SubWorkerKil: both stop the Rust server, the control plane, the
# Electron UI and the plugin-host subprocesses, and both preserve sessions in
# server/data. Press either one; RAM just reads as "I'm out of memory".
#
# It used to call a kill_agents.sh under the old EliaAI tree, which targeted the
# retired Docker stack and knew nothing about this repo's processes.

set -uo pipefail

HERE="$(cd "$(dirname "$(realpath "$0")")" && pwd)"
exec "$HERE/SubWorkerKil.command" "$@"
