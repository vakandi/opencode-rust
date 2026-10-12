#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

GLOBAL_MCP="/Users/vakandi/.omp/agent/mcp.json"
PROJECT_MCP="/Users/vakandi/Documents/omp-server/.omp/mcp.json"

backup_one() {
  local src="$1"
  if [[ ! -f "$src" ]]; then
    echo "skip (missing): $src" >&2
    return 0
  fi
  local ts
  ts=$(date +%Y%m%d-%H%M%S)
  local dest="${src}.bak-${ts}"
  cp "$src" "$dest"
  echo "${src} -> ${dest}"
  python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$dest"
}

backup_one "$GLOBAL_MCP"
backup_one "$PROJECT_MCP"
