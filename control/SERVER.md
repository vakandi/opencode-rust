# omp-server/ entrypoint — wrapper server (Bun)

Loopback-only Bun server that exposes the TopBar contract on **127.0.0.1:5677**.
It is the only code in this fork that is Elia-specific; `packages/*` is upstream
and is never edited here.

**Where the code lives:** ONLY in `~/Documents/omp-server` (this checkout).
`/Users/vakandi/EliaAI/subworkers-omp` is intentionally empty.
The live Python server on `:5656` (`~/EliaAI/subworkers/server/app/`) is untouched —
it is read-only reference for contract parity (tunnel manager, routes, watch script).
Plan (10 concurrent sessions, caches, CPU/RAM, TopBar livestream parity): [`docs/elia-optimization-plan.md`](docs/elia-optimization-plan.md).

## Scripts
```bash
./install.sh [--no-docker]  # one-time
./start.sh [--docker] [--with-tunnel]
./stop.sh [--with-colima]
```

- `./install.sh [--no-docker]` — one-time: checks `bun>=1.3.14` + docker CLI, `bun install` in `omp-server/`, mints `omp-server/.env` (`ELIA_AUTH_TOKEN`, chmod 600) unless present, ensures data/config dirs, `docker build -t omp-server` unless `--no-docker`.
- `./start.sh [--docker] [--with-tunnel]` — sources `omp-server/.env`; exits 0 if `:5677` already healthy (never disturbs); default boots `bun omp-server/server.ts` backgrounded (log `omp-server/data/server.log`, pid `omp-server/data/server.pid`) + health gate; `--docker` runs compose up instead, `--with-tunnel` adds the cloudflared sidecar (`--profile tunnel`).
- `./stop.sh [--with-colima]` — stops pidfile server + compose down; preserves `omp-server/data`; `--with-colima` also stops Colima.
- Shared env: `omp-server/.env` is a verbatim copy of live `~/EliaAI/subworkers/server/.env` (same `ELIA_AUTH_TOKEN`, plus `IDLE_TIMEOUT`, `IDLE_CHECK_INTERVAL`, `MAX_CONCURRENT_RUNS`), chmod 600, gitignored — both stacks accept the same token. Compose also reads it via `env_file`. Tunnel token likewise copied to `omp-server/config/tunnel.token` (chmod 600, gitignored), so `/tunnel/status` reports `configured:true` out of the box.
- Cloudflare: `cloudflared` service in `omp-server/docker-compose.yml` (profile `tunnel`, token-file mount, no `docker.sock`) mirrors live `server-cloudflared`; start with `./start.sh --docker --with-tunnel` or `docker compose --profile tunnel up -d`. Host alternative: `omp-server/tunnelWatch.sh` sidecar.

Repo layout: upstream (`package.json`/`Cargo.toml`/`bunfig.toml`/workspaces/...) lives in `omp-server/` alongside the wrapper entrypoint (`server.ts`, `engine.ts`, `routes/`, `config/`) — Bun workspaces, Cargo members, and Bazel all resolve from there. Our footprint is `omp-server/` (wrapper) + `install.sh`/`start.sh`/`stop.sh`/`reset.sh` + root `AGENTS.md`; `/Users/vakandi/EliaAI/subworkers-omp` stays intentionally empty; live `:5656` untouched.

Manual equivalents:

```bash
ELIA_AUTH_TOKEN=... bun omp-server/server.ts
curl -s http://127.0.0.1:5677/health
curl -s -H "Authorization: Bearer $ELIA_AUTH_TOKEN" http://127.0.0.1:5677/status | head -c 400
```

## Boot

```bash
cd ~/Documents/omp-server
ELIA_AUTH_TOKEN=test-token-123 bun omp-server/server.ts
# [omp-server] listening on http://127.0.0.1:5677 auth=on
```

`startServer()` (`omp-server/server.ts`) does, in order: `reloadEntries()` (load
`omp-server/config/subworkers.json`), `setTriggerFn()` (scheduler → same `admitRun`
path as `POST /trigger`), `startScheduler()` (30 s tick), `startWatchdog()`
(30 s tick), then `Bun.serve({ port, hostname: "127.0.0.1" })` with port from
`omp-server/config/server.json` (`{"port": 5677, "concurrency": 8}`).
A 30 s interval broadcasts `status_update` so TopBar never goes stale.
The engine (`omp-server/engine.ts`) is imported lazily — the listener boots without

## Auth

Shared-token auth (`omp-server/auth.ts`, mirrors live `app/core/auth.py`).
Empty/missing `ELIA_AUTH_TOKEN` → auth disabled (backward compat).

- HTTP: `Authorization: Bearer <token>` OR `X-Elia-Token: <token>`.
- WS (`/ws`): `?token=` OR the same two headers (plus `Sec-WebSocket-Protocol`).
- `GET /health` is always open. Everything else is gated in `server.ts`.
- Docs disabled: `/docs`, `/redoc`, `/openapi.json` → `404 {"detail":"not found"}`.
- WS: on open the server sends `{event:"initial_status", …status, opencode_health,
  scheduler_running}`; heartbeat `ping` → `{event:"pong"}`. Accepted ping shapes:
  raw `"ping"`, `{"type":"ping"}` (EliaTopBar), `{"event":"ping"}`. Other WS events:
  `status_update`, `subworker_started`, `run_log`, `run_banner`, `subworker_completed`.

```bash
curl http://127.0.0.1:5677/health
# {"status":"ok","uptime_s":12}

TOKEN=test-token-123
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:5677/status | head -c 400
curl -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"prompt":"hello"}' http://127.0.0.1:5677/trigger/elia
# {"status":"triggered","name":"elia","message":"Subworker 'elia' triggered successfully","session_id":"ses_..."}
```

## Route table

Auth-gated unless noted. Unknown subworker → `404 {"detail":"unknown subworker <name>"}`.

| Method | Path | Handler | Shape |
|---|---|---|---|
| GET | `/health` | inline (`server.ts`) | open; `{"status":"ok","uptime_s":N}` |
| GET | `/status` | `handleGetStatus` | `{scheduler_running, total, subworkers:[StatusItem]}` |
| GET | `/status/{name}` | `handleGetStatusOne` | StatusItem + `agent_id, timeout_minutes, max_retries` |
| POST | `/trigger/{name}` | `handleTrigger` → `admitRun` | body `{prompt?, model?, variant?}` → `200 {status:"triggered", name, message, session_id}`; unknown → `404 {status:"error",…}`; over cap → `202 {status:"queued_429", name, session_id}` |
| POST | `/enable/{name}` | `handleEnable` | `{status:"enabled"\|"already_enabled", name, enabled:true}` |
| POST | `/disable/{name}` | `handleDisable` | `{status:"disabled"\|"already_disabled", name, enabled:false}` |
| PUT | `/status/{name}` | `handleUpdateStatus` | allowed keys `agent_id, model, variant, timeout_minutes, max_retries, schedule`; schedule validated (`every` 1..1440 min, else `interval`/`cron` object); empty → `422 {detail:"No fields to update"}`; bad JSON/schedule → `422` |
| POST | `/config/reload` | `handleConfigReload` | `{status:"reloaded", added:[], removed:[], unchanged:[names], total}` |
| GET | `/logs/{name}?lines=N` | `handleLogs` | `{name, log_file, lines, total_lines}`; N clamped 1..2000 (default 100); frame-derived lines first, then live workspace `logs/*.log` tail (best-effort) |
| GET | `/sessions/{name}?limit=N&session_id=S` | `handleGetSession` | `{name, session_id, messages:[{info:{role,time_created}, parts:[{type:"text"\|"reasoning"\|"tool", text?\|tool?}]}], total_messages}`; limit 1..500 (default 50); session defaults running → last → null (empty → `session_id:null, messages:[]`) |
| GET | `/sessions/{name}/list` | `handleListSessions` | `{name, sessions:[{session_id, title, agent, time_created}]}`; running first (`"▶ Running"`), then last |
| POST | `/sessions/{name}/{id}/continue` | `handleContinueSession` | body `{message?}` (default `"continue the tasks"`); persists run `running`, lazy-imports engine `continueEngineSession` without awaiting → `{status:"continued", name, session_id, message:"Message sent to session …"}` |
| GET | `/sessions/{name}/{id}/events` | `handleSessionEvents` | SSE `text/event-stream`; replays buffered frames (`{name, session_id, field, text}`), then clean EOF on disconnect |
| GET | `/server/health` | `handleServerHealth` | `{state:"running", health_status:"healthy", pid, base_url:"http://127.0.0.1:5677", restart_count, last_health_check:{ok:true, at}}` |
| POST | `/server/restart` | `handleServerRestart` | bumps `restart_count`, broadcasts status → `{status:"restarted", message:"engine supervisor restarted (listener kept)", state:"running"}` |
| POST | `/server/cleanup` | `handleServerCleanup` | body `{restart_opencode?, run_idle_cleaner?}`; always `{ok:true, steps:[{name,ok,duration_ms}]}` (`idle_sweep`, plus `engine_restart_skipped` when requested — nothing engine-owned to reap) |
| GET | `/tunnel/status` | `handleTunnelStatus` | `{configured, tunnel_token_masked, setup:{status,…}}` |
| POST | `/tunnel/check` | `handleTunnelCheck` | body `{domain}` → `{token_ok, message}`; `422 {token_ok:false,…}` on bad JSON |
| POST | `/tunnel/setup` | `handleTunnelSetup` | body `{domain, token?, email?}`; missing domain → `422`; writes token file, returns `{status:"started", domain}` |
| POST | `/tunnel/stop` | `handleTunnelStop` | `{status:"stopped"}` |
| POST | `/tunnel/remove` | `handleTunnelRemove` | deletes token file → `{status:"removed"}` |
| GET | `/models` | `handleModels` | `{models:[{id, variants:[]}], total}`; parsed from `$OMP_AGENT_DIR/models.yml` (default `~/.config/omp/models.yml`), sorted by id |
| GET | `/main-agent` | `handleGetMainAgent` | `{name}`; default `"elia"` |
| POST | `/main-agent` | `handleSetMainAgent` | body `{name}` (`/^[a-zA-Z0-9_-]{1,64}$/`, else `422`) → `{name}`; persisted to `omp-server/data/main-agent.json` |
| POST | `/test/frames/{name}` | `handleTestFrames` | auth-gated synthetic gate: creates a `running` run, appends + broadcasts text/reasoning/tool frames and an info banner → `{status:"emitted", name, session_id}` |

`StatusItem` (`omp-server/statusView.ts`): `{name, enabled, running, next_run,`
schedule_type, schedule, model, variant}`. `running` comes from the run
registry (`status=="running"`), never from the schedule. Unknown subworker on
`GET /status/{name}` → 404 via `notFound`.

## Engine bridge (`omp-server/engine.ts`)

Sole omp dependency: `createAgentSession` from
`./packages/coding-agent/src/sdk.ts`, per-run cwd, `agentDir`
`~/.config/omp/agents`, isolated EventBus. Session events are normalized by
`eventToFrames()` to TopBar frames `{field: "text"|"reasoning"|"tool"}` —
the field contract is fixed here, SDK names remapped at this one place.

- **Creation mutex:** the SDK replaces a process-wide Main-agent singleton
  during init, so concurrent `createAgentSession` calls abort each other;
  `creationGate` serializes creation only — prompts and turn waits stay
  parallel after the session exists.
- **Pre-warm note:** static ESM imports of the browser tool
  `prelude-definition.ts` warm the module registry so old-Bun (`<=1.3.x`)
  sync `require()` of `with { type: "text" }` assets doesn't link
  `prelude.js` as code. Side-effect-free; no-op on newer Buns.
- **Lazy load:** both `admitRun` and the continue path `import("./engine.ts")`
  dynamically and never await completion on the admission path.

## Store (`omp-server/store.ts`)

- `omp-server/data/runs.json` — full `RunRecord[]`
  `{name, session_id, status: running|queued_429|completed|failed|continued,
  created_at, last_frame_at, started_at, model?, variant?, error?}`.
  Written synchronously (fsync-equivalent) on every admission before the HTTP
  response. Helpers: `runningSessionId`, `lastSessionId`, `activeRunCount`.
- `omp-server/data/frames/<sessionId>.jsonl` — `{t, field: text|reasoning|tool, text}` lines.
- `omp-server/data/state.json` — `{next_run: {name: iso}, restart_count, main_agent?}`.
- Session ids: `ses_<epoch-ms-base36><rand>` via `snowflakeSessionId()`.

## Scheduler (`omp-server/scheduler.ts`)

Tick every 30 s; `triggers call the same admission path as POST /trigger`.
Schedule types per entry (`omp-server/config/subworkers.json`, 25 entries):

- `interval {hours:[9..23], minute:0, days?}` — daily clock times.
- `every {every, hours?, days?}` — `every` in minutes.
- `cron {expression}` — minimal 5-field matcher (`minute hour dom month dow`).

`nextRunAfter(entry, from)` is strictly after `from`; null when unscheduled.
`next_run` values persist in `state.json`. `isEnabled` = override ?? entry
default; `setEnabled` only flips the in-memory override map.

## Watchdog (`omp-server/watchdog.ts`)

No RSS-cap gate — container `mem_limit` is the only guard. Per run
`last_frame_at + started_at`; envs `STALL_TIMEOUT_S` (default 300) and
`MAX_RUN_S` (default 3600). On expiry: mark `failed`, error banner on `/ws`,
persist, Discord alert, release proxy. Runs never stay `"running"` forever.
`checkOnce(nowMs)` returns expired session ids; `startWatchdog()` ticks 30 s.

## Notifier (`omp-server/notifier.ts`)

Discord webhook from `DISCORD_WEBHOOK_URL` env only (never logged).
Debounce 300 s per `(name, kind)`; 10 s fetch timeout; failures logged, never thrown.

## Proxy (local plugin ONLY)

One proxy system: the vendored `omp-server/extensions/proxy-local.ts`,
loaded exclusively by every worker (`disableExtensionDiscovery` +
`additionalExtensionPaths`). It runs an in-process residential forward on
`127.0.0.1:18898`, sets `PI_PROXY`/`HTTPS_PROXY` to it, rotates upstreams
natively in-process (`PROXY_LOCAL_NO_SPAWN=1`, no shell scripts) on rate
limits and every 4 min, and appends to `omp-server/data/proxy-local.log`
(`ELIA_PROXY_LOG`, bind-mounted for host monitoring). Per-worker upstream
files via `ELIA_PROXY_CONF`. Model traffic is proxied; agent tool shells
stay direct by upstream omp design. No forwarding daemon, no extra
container, no quarantine.

## Tunnel (`omp-server/tunnel.ts` + `omp-server/tunnelWatch.sh`)

Socketless design mirroring live `tunnel_manager.py` + `cloudflared-watch.sh`.
The manager only writes the runner token to `omp-server/config/tunnel.token`
(chmod 600); `omp-server/tunnelWatch.sh` (host sidecar, no Docker socket) polls
(default 15 s, `TUNNEL_WATCH_POLL_S`) and starts/stops the sibling
`cloudflared` service (`$CLOUDFLARED_SERVICE`, default `cloudflared-omp`).
Setup completion is a 2 s background check on the token file.
Never mounts `/var/run/docker.sock`. Same 5-endpoint contract as live
(status/check/setup/stop/remove).

## Docker

`omp-server/Dockerfile`: `oven/bun:1-slim`, engine loopback-only inside the
container, `HEALTHCHECK` on `http://localhost:5677/health`,
`EXPOSE 5677`, `CMD ["bun", "server.ts"]`.

```bash
ELIA_AUTH_TOKEN=... docker compose -f omp-server/docker-compose.yml up --build
# or: docker build -f omp-server/Dockerfile .   (binds no ports)
```

Compose (`omp-server/docker-compose.yml`): `5677:5677`, envs `ELIA_AUTH_TOKEN`
(required), `MAX_CONCURRENT_RUNS` (8), `STALL_TIMEOUT_S` (300), `MAX_RUN_S`
(3600), `DISCORD_WEBHOOK_URL`; `mem_limit: 4g`; volumes `./data`, `./config:ro`,
`${SUBWORKERS_DIR:-~/EliaAI/subworkers}:/data/subworkers:ro`,
`~/.config/omp:ro`, proxies file, plus MCP parity mounts
(`~/.config/mcp/mcp_servers.json:ro`, `~/.local/bin/mcp-cli:ro` — same
mcp-cli list inside as outside); `restart: unless-stopped`.
Image ships ffmpeg + node 22 (agent MCPs: whisper, video, browser) for
~2.05 GB total; slim runtime closure ~1.43 GB (−28% vs unslimmed).
Linux `.node` addon packs via `bun --cwd=omp-server scripts/bazel-natives.ts host`
(`.dockerignore` exception lets `pi_natives.linux-*.node` into context).
Bind address follows `HOST` (default `127.0.0.1` loopback locally);
compose sets `HOST: 0.0.0.0` so the mapped 5677 is reachable in-container.

## testFrames gate

- HTTP: `POST /test/frames/{name}` (above) — auth-gated, emits in-process.
- CLI: `ELIA_AUTH_TOKEN=... bun omp-server/testFrames.ts [name]` (default `elia`).
- Both emit text + reasoning + tool `run_log` frames plus one `run_banner`
  on a fake session **without starting an engine**. TopBar on `:5677` must
  show all three streams in the run popup.

## TopBar contract notes

- Same `StatusItem` shape and `{scheduler_running, total, subworkers}` payload
  for `/status`, WS `initial_status`, and WS `status_update`.
- Session shapes match live for `LogPopoverView`; the events route follows the
  qwen shim SSE shape (`data: {name, session_id, field, text}`).
- `running` is registry-derived; scheduler state is informational only.
- Subagents panel: tool parts carry BARE names (`tool:"task"`, never `▶ task`)
  plus `input` (call args: `description`, `subagent_type`, synthetic
  `sessionId: ses_…` minted per `toolCallId`) and `output` (truncated result).
  TopBar `extractSubagents` matches `task`/`call_omo_agent`/`team_create` and
  requires the ses_-prefixed id — satisfied by construction.
- Todo lists: every tool frame carries `input`/`output`, so `todo*` tool calls
  flow into TopBar `extractTodos` (input/output/delta) with zero extra code.
- Live `:5656` is never written to — parity is verified by reading
  `~/EliaAI/subworkers/server/app/` only.
