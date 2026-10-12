#!/usr/bin/env bash
#
# Check LinkedIn MCP readiness INSIDE the omp-server docker container.
# Runs OUTSIDE docker (host), inspects inside via `docker exec`.
#
# Checks, in order:
#   1. container is up
#   2. /root/.omp/agent/mcp.json inside contains mcp-server-linkedin
#   3. `uvx` exists inside the image
#   4. the package runs (uvx mcp-server-linkedin@latest --help)
#   5. session profile files exist (/root/.linkedin-mcp/profile)
#   6. LIVE READ-ONLY PROBE: get_my_profile actually returns a profile
#
# Step 6 is the only step that proves an agent can USE the MCP. Steps 1-5 all
# passed on 2026-09-26 while every tool call failed ("No valid LinkedIn session
# is available in Docker") -- the profile dir was non-empty but the session was
# not usable. Never report this MCP healthy on steps 1-5 alone.
#
# Usage: scripts/check-linkedin-docker.sh [--login]
#   --login  if the session is missing and we have a TTY, run the
#            interactive login inside the container (opens no window in
#            docker -- sign in via the --login-viewer URL it prints, or
#            use --import-from-browser on the host and mount the profile).
#            A host login also works: ~/.linkedin-mcp is bind-mounted :rw,
#            so the session it writes is the one the container reads.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/../docker-compose.yml"
if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "❌ wrong path: no docker-compose.yml next to this script ($COMPOSE_FILE)" >&2
  echo "   do not copy this script elsewhere -- run it from the omp-server checkout:" >&2
  echo "   omp-server/scripts/check-linkedin-docker.sh" >&2
  exit 2
fi
SERVICE="omp-server"
SERVER="mcp-server-linkedin"

fail=0
say()  { printf '%s\n' "$*"; }
ok()   { say "✅ $*"; }
bad()  { say "❌ $*"; fail=1; }
warn() { say "⚠️  $*"; }

CID="$(docker compose -f "$COMPOSE_FILE" ps -q "$SERVICE" 2>/dev/null || true)"
if [[ -z "$CID" ]]; then
  bad "container '$SERVICE' is not running (docker compose ps empty)"
  say "   start it: docker compose -f $COMPOSE_FILE up -d"
  exit 2
fi
ok "container up: $CID"

if docker exec "$CID" grep -q "$SERVER" /root/.omp/agent/mcp.json 2>/dev/null; then
  ok "config present inside: /root/.omp/agent/mcp.json has $SERVER"
else
  bad "config missing inside: $SERVER not in /root/.omp/agent/mcp.json"
  say "   host ~/.omp/agent/mcp.json is mounted rw -- check the mount and /mcp reload"
fi

UVX_OK=0
if docker exec "$CID" sh -c 'command -v uvx' >/dev/null 2>&1; then
  UVX_OK=1
  ok "uvx found inside image"
  if docker exec "$CID" uvx mcp-server-linkedin@latest --help >/dev/null 2>&1; then
    ok "package runs: mcp-server-linkedin@latest --help OK"
  else
    bad "package failed to run inside container"
  fi
else
  bad "no 'uvx' inside image (oven/bun:1-slim has no Python/uv)"
  say "   IMAGE GAP: Dockerfile must install uv + python + chromium for LinkedIn in docker"
fi

if docker exec "$CID" sh -c 'ls -A /root/.linkedin-mcp/profile 2>/dev/null | grep -q .' 2>/dev/null; then
  ok "profile files present inside (/root/.linkedin-mcp/profile) — necessary, NOT sufficient"
else
  bad "no LinkedIn session files inside container"
fi

# Advisory only: the profile is a HOST (macOS) Chromium dir bind-mounted into a linux
# container, so the tool runs it in "foreign runtime" bridge mode and explicitly does not
# verify source-cookie validity. Only the live probe below can settle it.
STATUS_OUT="$(docker exec "$CID" sh -c 'cd /root && timeout 120 uvx mcp-server-linkedin@latest --status 2>&1' 2>/dev/null || true)"
if grep -qi 'foreign runtime' <<<"$STATUS_OUT"; then
  warn "profile mode: foreign runtime (host profile bridged into linux) — cookie validity unverified by --status"
fi

# ---- Live read-only tool probe: the only check that proves an agent can USE the MCP ----
# get_my_profile is read-only: it changes nothing on the account.
ok "running live read-only probe: get_my_profile (this is the check that matters)"
docker exec -i "$CID" sh -c 'cat > /tmp/_li_probe.py' >/dev/null 2>&1 <<'PYEOF' || true
import json, subprocess, sys, time


def send(p, o):
    p.stdin.write(json.dumps(o).encode() + b"\n")
    p.stdin.flush()


def read_id(p, want, timeout):
    end = time.time() + timeout
    while time.time() < end:
        line = p.stdout.readline()
        if not line:
            time.sleep(0.2)
            continue
        line = line.strip()
        if not line.startswith(b"{"):
            continue
        try:
            m = json.loads(line)
        except json.JSONDecodeError:
            continue
        if m.get("id") == want:
            return m
    return None


p = subprocess.Popen(["uvx", "mcp-server-linkedin@latest"],
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
send(p, {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                    "clientInfo": {"name": "omp-probe", "version": "1"}}})
if read_id(p, 1, 180) is None:
    print("PROBE_FAIL initialize timed out")
    p.kill()
    sys.exit(2)
send(p, {"jsonrpc": "2.0", "method": "notifications/initialized"})
time.sleep(1)
send(p, {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
         "params": {"name": "get_my_profile", "arguments": {}}})
m = read_id(p, 2, 420)
p.kill()
if m is None:
    print("PROBE_FAIL no response to get_my_profile (timed out after 420s)")
    sys.exit(1)
r = m.get("result") or {}
t = " ".join(c.get("text", "") for c in (r.get("content") or []) if isinstance(c, dict))
if r.get("isError"):
    print("PROBE_FAIL tool returned isError: " + t[:300])
    sys.exit(1)
if "linkedin.com/in/" not in t:
    print("PROBE_FAIL unexpected payload (no profile URL): " + t[:300])
    sys.exit(1)
print("PROBE_OK " + t[:200].replace("\n", " "))
sys.exit(0)
PYEOF

PROBE_OUT="$(docker exec "$CID" sh -c 'cd /root && timeout 700 python3 /tmp/_li_probe.py 2>&1' 2>/dev/null | tail -3 || true)"
docker exec "$CID" rm -f /tmp/_li_probe.py >/dev/null 2>&1 || true
if grep -q 'PROBE_OK' <<<"$PROBE_OUT"; then
  ok "live probe: get_my_profile returned a real profile -> an agent CAN use this MCP"
  say "   $(grep -o 'PROBE_OK.*' <<<"$PROBE_OUT" | cut -c1-160)"
else
  bad "live probe FAILED — the MCP is NOT usable by agents right now:"
  grep -E 'PROBE_FAIL' <<<"$PROBE_OUT" | sed 's/^/     /'
  say "   re-login on the host (profile is shared into the container):"
  say "     uvx mcp-server-linkedin@latest --login"
  say "   or inside docker:  docker exec -it $CID uvx mcp-server-linkedin@latest --login --login-viewer"
  say "   NOTE: a passing 'profile exists' check above does NOT mean logged in."
fi

if [[ "$fail" -eq 0 ]]; then
  say "READY: LinkedIn MCP is set up inside docker AND a real tool call succeeded."
else
  say "NOT READY: see ❌ lines above."
  warn "host-side alternative works today: 'uvx mcp-server-linkedin@latest --login' on the Mac"
fi
exit "$fail"
