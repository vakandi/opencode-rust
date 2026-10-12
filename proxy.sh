#!/usr/bin/env bash
# Turning the residential proxy on and off, without restarting the server.
#
#   ./proxy.sh off    provider traffic goes out on THIS machine's real IP
#                     (turn your VPN on first, so traffic follows the VPN)
#   ./proxy.sh on     provider traffic goes out through a rotating residential proxy
#   ./proxy.sh status what the server is doing right now
set -euo pipefail

PORT="$(jq -r '.port // 4096' "$(dirname "$0")/config/server.json" 2>/dev/null || printf '4096')"
BASE="http://127.0.0.1:$PORT"

curl -sf "$BASE/health" >/dev/null 2>&1 || { echo "server not answering on $BASE — start it first" >&2; exit 1; }

case "${1:-status}" in
  off)
    curl -sf -X POST "$BASE/proxy" -H 'Content-Type: application/json' -d '{"direct":true}' \
      | jq '{direct, upstream: .detail.upstream}' ;;
  on)
    curl -sf -X POST "$BASE/proxy" -H 'Content-Type: application/json' -d '{"direct":false}' \
      | jq '{direct, upstream: .detail.upstream}' ;;
  status)
    curl -sf "$BASE/proxy" \
      | jq '{mode: (if .direct then "real IP (VPN)" else "residential proxy" end), enabled, port, upstream, rotations_total, quarantined}' ;;
  *)
    echo "usage: $0 [on|off|status]" >&2; exit 2 ;;
esac
