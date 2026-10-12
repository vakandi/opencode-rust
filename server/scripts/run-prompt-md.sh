#!/usr/bin/env bash
# Run a session from a PROMPT.md against the Rust OpenCode server and print the
# livestream: AI text, AI reasoning, and AI tool activity, as they arrive.
#
# The server streams on GET /event. A prompt is submitted to
# POST /session/{id}/prompt. This script wires the two together and renders the
# event stream as a readable transcript.
#
# Usage:
#   scripts/run-prompt-md.sh <PROMPT.md> [base-url] [model]
#
# Examples:
#   scripts/run-prompt-md.sh PROMPT.md
#   scripts/run-prompt-md.sh PROMPT.md http://127.0.0.1:4097 anthropic/claude-sonnet-4
#
# Exit codes:
#   0  session ran and the stream closed
#   1  usage / environment error
#   2  server rejected the request

set -euo pipefail

die() { printf '[run-prompt-md] %s\n' "$*" >&2; exit 1; }
log() { printf '[run-prompt-md] %s\n' "$*" >&2; }

PROMPT_FILE="${1:-}"
BASE_URL="${2:-http://127.0.0.1:4097}"
MODEL="${3:-}"

[[ -n "$PROMPT_FILE" ]] || { sed -n '2,20p' "$0"; exit 1; }
[[ -f "$PROMPT_FILE" ]] || die "no such file: $PROMPT_FILE"
command -v jq >/dev/null 2>&1 || die "jq is required"

EVENT_LOG="$(mktemp -t run-prompt-md-events.XXXXXX)"
cleanup() {
  [[ -n "${EVENT_PID:-}" ]] && kill "$EVENT_PID" 2>/dev/null || true
  rm -f "$EVENT_LOG"
}
trap cleanup EXIT

curl -sSf -m 10 "$BASE_URL/global/health" >/dev/null 2>&1 \
  || die "server not reachable at $BASE_URL (start it: ./target/debug/opencode serve --port 4097)"

log "creating session on $BASE_URL"
SESSION_ID="$(curl -sSf -m 15 -X POST "$BASE_URL/session" \
  -H 'content-type: application/json' -d '{}' \
  | jq -r '.id // empty')"
[[ -n "$SESSION_ID" ]] || die "server did not return a session id"
log "session $SESSION_ID"

# Subscribe before prompting so no early event is missed.
curl -sN -m 300 "$BASE_URL/event" >"$EVENT_LOG" 2>/dev/null &
EVENT_PID=$!
disown "$EVENT_PID" 2>/dev/null || true
sleep 1

PROMPT_TEXT="$(cat "$PROMPT_FILE")"
if [[ -n "$MODEL" ]]; then
  PAYLOAD="$(jq -n --arg m "$PROMPT_TEXT" --arg model "$MODEL" \
    '{message: $m, model: $model}')"
else
  PAYLOAD="$(jq -n --arg m "$PROMPT_TEXT" '{message: $m}')"
fi

log "prompt: $(wc -c <"$PROMPT_FILE" | tr -d ' ') bytes from $PROMPT_FILE"
RESPONSE="$(curl -sS -m 30 -X POST "$BASE_URL/session/$SESSION_ID/prompt" \
  -H 'content-type: application/json' -d "$PAYLOAD")" \
  || die "prompt request failed"
if printf '%s' "$RESPONSE" | jq -e '.error' >/dev/null 2>&1; then
  printf '%s\n' "$RESPONSE" | jq -r '.error.message // .error' >&2
  exit 2
fi
log "accepted: $(printf '%s' "$RESPONSE" | jq -c '{status, model}')"

# The engine emits at ~20 ticks/sec while streaming; poll status until idle.
DEADLINE=$(( $(date +%s) + 300 ))
while :; do
  sleep 1
  BUSY="$(curl -sS -m 10 "$BASE_URL/session/status" 2>/dev/null \
    | jq -r --arg s "$SESSION_ID" '.[$s].busy // false')"
  [[ "$BUSY" == "false" ]] && break
  if (( $(date +%s) > DEADLINE )); then
    log "timed out waiting for the session to go idle"
    break
  fi
done
sleep 1
kill "$EVENT_PID" 2>/dev/null || true
EVENT_PID=""

log "stream captured in $EVENT_LOG ($(grep -c '^data: ' "$EVENT_LOG" || true) frames)"

# SSE interleaves `event: message` lines with `data: {...}` lines, and the data line is
# not valid JSON until the prefix is stripped. Read raw lines, keep only data frames.
json_frames() {
  jq -R 'select(startswith("data: ")) | ltrimstr("data: ") | fromjson? // empty' "$EVENT_LOG"
}

json_frames | jq -r '
  select(.type == "message.part.updated")
  | .properties.part as $p
  | if $p.type == "text" then "TEXT      | " + ($p.text // "")
    elif $p.type == "reasoning" then "REASONING | " + ($p.text // "")
    elif $p.type == "tool" then "TOOL      | " + ($p.tool // "?") + " -> " + (($p.state.status // "?") | tostring)
    else empty end
' | awk '!seen[$0]++ || /^TOOL/'

echo
json_frames | jq -r '
  select(.type == "message.part.delta")
  | "DELTA     | " + (.properties.delta // "")
' | awk 'NR>1 { $1=""; sub(/^ +/,""); printf "%s", $0 } END { print "" }'

echo
echo "--- final session state ---"
curl -sS -m 10 "$BASE_URL/session/$SESSION_ID/message" \
  | jq -r '.[] | "\(.info.role): \(.parts | map(.type) | join(","))"' 2>/dev/null
