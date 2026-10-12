#!/usr/bin/env bash
# tunnelWatch.sh — file-watcher sidecar for the omp-server tunnel (no Docker socket).
# Watches omp-server/config/tunnel.token: when present, ensures the sibling `cloudflared`
# service is running; when absent, stops it. Restart loop with backoff.
set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TOKEN_FILE="$SCRIPT_DIR/config/tunnel.token"
SERVICE_NAME="${CLOUDFLARED_SERVICE:-cloudflared-omp}"
POLL_S="${TUNNEL_WATCH_POLL_S:-15}"

log() { echo "[$(date '+%Y-%m-%dT%H:%M:%S')] $*" >&2; }

service_running() {
  if command -v systemctl >/dev/null 2>&1; then
    systemctl --user is-active --quiet "$SERVICE_NAME" 2>/dev/null
  else
    pgrep -f "cloudflared.*tunnel.*run" >/dev/null 2>&1
  fi
}

service_start() {
  log "starting $SERVICE_NAME"
  if command -v systemctl >/dev/null 2>&1; then
    systemctl --user start "$SERVICE_NAME" 2>/dev/null || log "systemctl start failed"
  else
    log "no systemctl; start cloudflared manually with --token-file $TOKEN_FILE"
  fi
}

service_stop() {
  log "stopping $SERVICE_NAME"
  if command -v systemctl >/dev/null 2>&1; then
    systemctl --user stop "$SERVICE_NAME" 2>/dev/null || log "systemctl stop failed"
  else
    pkill -f "cloudflared.*tunnel.*run" 2>/dev/null || log "nothing to stop"
  fi
}

log "watching $TOKEN_FILE (service=$SERVICE_NAME, poll=${POLL_S}s)"
while true; do
  if [[ -f "$TOKEN_FILE" ]]; then
    service_running || service_start
  else
    service_running && service_stop || true
  fi
  sleep "$POLL_S"
done
