#!/usr/bin/env bash
#
# Check Indeed/HasData MCP readiness INSIDE the omp-server docker container.
# Runs OUTSIDE docker (host), inspects inside via `docker exec` + a live
# HTTPS probe of https://mcp.hasdata.com/mcp?apis=indeed from the host
# (same endpoint the container dials, so the result applies to both).
#
# Checks, in order:
#   1. container is up
#   2. /root/.omp/agent/mcp.json inside contains hasdata-indeed
#   3. auth: literal x-api-key in the inside config OR HASDATA_API_KEY
#      in the container env (placeholder / ${VAR} without env = NOT READY)
#   4. live JSON-RPC initialize probe against the endpoint (200 = READY)
#
# Usage: scripts/check-indeed-docker.sh [--key KEY]
#   --key KEY  write KEY into omp-server/.env as HASDATA_API_KEY (chmod 600)
#              so the next container (re)start picks it up. Does NOT restart.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/../docker-compose.yml"
if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "❌ wrong path: no docker-compose.yml next to this script ($COMPOSE_FILE)" >&2
  echo "   do not copy this script elsewhere -- run it from the omp-server checkout:" >&2
  echo "   omp-server/scripts/check-indeed-docker.sh" >&2
  exit 2
fi
ENV_FILE="$SCRIPT_DIR/../.env"
SERVICE="omp-server"
SERVER="hasdata-indeed"
URL="https://mcp.hasdata.com/mcp?apis=indeed"

fail=0
KEY="${HASDATA_API_KEY:-}"
say()  { printf '%s\n' "$*"; }
ok()   { say "✅ $*"; }
bad()  { say "❌ $*"; fail=1; }
mask() { local k="$1"; printf '%s****%s' "${k:0:4}" "${k: -4}"; }

if [[ "${1:-}" == "--key" ]]; then
  KEY="${2:?--key needs a value}"
  touch "$ENV_FILE"; chmod 600 "$ENV_FILE"
  if grep -q '^HASDATA_API_KEY=' "$ENV_FILE"; then
    sed -i '' "s|^HASDATA_API_KEY=.*|HASDATA_API_KEY=$KEY|" "$ENV_FILE"
  else
    printf 'HASDATA_API_KEY=%s\n' "$KEY" >> "$ENV_FILE"
  fi
  ok "wrote HASDATA_API_KEY=$(mask "$KEY") to $ENV_FILE"
  say "   apply it: docker compose -f $COMPOSE_FILE up -d (recreates env)"
fi

CID="$(docker compose -f "$COMPOSE_FILE" ps -q "$SERVICE" 2>/dev/null || true)"
if [[ -z "$CID" ]]; then
  bad "container '$SERVICE' is not running"
  say "   start it: docker compose -f $COMPOSE_FILE up -d"
  exit 2
fi
ok "container up: $CID"

if docker exec "$CID" grep -q "$SERVER" /root/.omp/agent/mcp.json 2>/dev/null; then
  ok "config present inside: /root/.omp/agent/mcp.json has $SERVER"
else
  bad "config missing inside: $SERVER not in /root/.omp/agent/mcp.json"
fi

# Effective key: literal in inside config wins, else container env, else $KEY.
IN_LITERAL="$(docker exec "$CID" grep -o '"x-api-key": *"[^"]*"' /root/.omp/agent/mcp.json 2>/dev/null | head -1 | sed 's/.*: *"//; s/"$//' || true)"
if [[ -n "$IN_LITERAL" && "$IN_LITERAL" != *REPLACE_WITH* && "$IN_LITERAL" != *'$'*'{ '* && "$IN_LITERAL" != *HASDATA_API_KEY* ]]; then
  ok "literal API key present in inside config ($(mask "$IN_LITERAL"))"
  [[ -z "$KEY" ]] && KEY="$IN_LITERAL"
else
  IN_KEY="$(docker exec "$CID" printenv HASDATA_API_KEY 2>/dev/null || true)"
  if [[ -n "$IN_KEY" ]]; then
    ok "HASDATA_API_KEY is set inside container ($(mask "$IN_KEY"))"
    [[ -z "$KEY" ]] && KEY="$IN_KEY"
  else
    bad "no usable key: config has placeholder, container env HASDATA_API_KEY unset"
    say "   fix: scripts/check-indeed-docker.sh --key <key>  (then compose up -d)"
  fi
fi

if [[ -n "$KEY" ]]; then
  CODE="$(curl -s -m 20 -o /tmp/hasdata-probe.json -w '%{http_code}' -X POST "$URL" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' -H "x-api-key: $KEY" \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"omp-probe","version":"1.0"}}}' || true)"
  if [[ "$CODE" == "200" ]]; then
    ok "live probe 200 -- endpoint accepts the key"
  else
    bad "live probe HTTP $CODE -- key rejected or endpoint down"
    head -c 300 /tmp/hasdata-probe.json 2>/dev/null; echo
  fi
else
  bad "no key available for live probe (set HASDATA_API_KEY or pass --key)"
fi

if [[ "$fail" -eq 0 ]]; then
  say "READY: Indeed MCP is set up inside docker."
else
  say "NOT READY: see ❌ lines above."
fi
exit "$fail"
