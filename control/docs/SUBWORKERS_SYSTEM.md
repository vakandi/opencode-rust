# Subworkers System — omp-server Docker

**Date:** 20 September 2026
**Version:** omp-server Docker (Bun wrapper on `:5677`, worker-per-run, 25 agents, all disabled by default)

> Technical documentation for the autonomous subworker agent system as re-hosted on omp-server.
>
> **Current runtime**: Bun wrapper server (`omp-server/server.ts`) on **127.0.0.1:5677** — host mode (`bun omp-server/server.ts`) or Docker (`omp-server` service + optional `cloudflared` sidecar, profile `tunnel`). No Python, no host `opencode` process. The live Python stack on `:5656` (`~/EliaAI/subworkers/server/app/`) is untouched — read-only reference for contract parity.
> **Purpose**: same agent homes, same PROMPT.md contract, same TopBar shapes — driven instead by `Bun.serve` + SDK sessions in isolated worker processes (`Bun.spawn(["bun", "omp-server/worker.ts", …])`, one distinct `HTTPS_PROXY` per run).
> **Previous version**: `~/EliaAI/subworkers/SUBWORKERS_SYSTEM.md` (v4.5 — FastAPI :5656 + in-container opencode :5655 era).

---

## Table of Contents

1. [Overview](#1-overview)
2. [Directory Structure](#2-directory-structure)
3. [Workspace Isolation](#3-workspace-isolation)
4. [Agent Registration](#4-agent-registration)
5. [Trigger Template](#5-trigger-template)
6. [Tools & MCP](#6-tools--mcp)
7. [Proxy](#7-proxy)
8. [Server (Bun wrapper)](#8-server-bun-wrapper)
9. [EliaTopBar client](#9-eliatopbar-client)
10. [Lifecycle scripts](#10-lifecycle-scripts)
11. [Prompt prefix — workspace path rules](#11-prompt-prefix--workspace-path-rules)

---

## 1. Overview

### What are subworkers here?

Subworkers are autonomous AI agents that run on a schedule or on demand. Each subworker has:

- A **personality** — loaded via `OMP_AGENT_DIR` when set, otherwise the SDK-default agent dir (`createAgentSession({ agentDir: process.env.OMP_AGENT_DIR?.trim() || undefined, … })` in `omp-server/worker.ts`).
- A **PROMPT.md** with task-specific instructions — read from the prompt file and injected as the session prompt (`readFileSync(promptFile)` in `omp-server/worker.ts`).
- A **workspace folder** — one agent home per agent under `subworkers/<name>/` in this repo (PROMPT.md + work files only, zero code).
- A **JSON registry entry** in `omp-server/config/subworkers.json` — 25 entries; defines schedule, enabled state, model/variant, retries, timeout.
- A **runner** — the Bun wrapper drives runs via SDK sessions in isolated worker processes: `Bun.serve` router (`omp-server/server.ts`) → admission (`omp-server/admit.ts`) → per-run worker (`omp-server/worker.ts`, `cwd` = agent workspace, isolated `EventBus`, explicit-only extension discovery for the vendored proxy-local copy).

### Architecture

```
./start.sh                       (host mode: bun omp-server/server.ts, :5677)
./start.sh --docker [--with-tunnel]  (compose: omp-server [+ cloudflared])
  └── omp-server/server.ts  (Bun.serve 127.0.0.1:5677, port from config/server.json)
        ├── scheduler.ts  (30 s tick → same admitRun path as POST /trigger)
        ├── watchdog.ts   (30 s tick, STALL_TIMEOUT_S / MAX_RUN_S)
        ├── admit.ts      (sync <100 ms admission, fsync run record first)
        └── engine.ts ──Bun.spawn──► worker.ts (ONE process per run)
                ├── createAgentSession({ cwd: workspace, agentDir, eventBus })
                ├── stdout: single-line JSON frames {field, text} + terminal {event:"done"}
                └── stdin: ROTATE <proxy-url> every 4 min (isolated child env)
```

### Control & monitoring

- **EliaTopBar** — status, Trigger Now, Enable/Disable, logs, live run popups via HTTP + WS on `:5677` (see §9).
- **REST API** — `http://localhost:5677` (`/health` open; everything else `Bearer $ELIA_AUTH_TOKEN`).
- **Full route contract**: [`SERVER.md`](../../../omp-server/SERVER.md) (route table, engine bridge, store, scheduler, watchdog, notifier, proxy, tunnel, Docker).

---

## 2. Directory Structure

```
<repo root>/
├── install.sh / start.sh / stop.sh / reset.sh   # lifecycle only, no code
├── README.md / AGENTS.md                        # repo docs
├── omp-server/                                  # ★ ALL code (upstream + wrapper server)
│   ├── docs/SUBWORKERS_SYSTEM.md                # this file
│   ├── SERVER.md                                # wrapper entrypoint contract
│   ├── server.ts                                # Bun.serve router, :5677
│   ├── engine.ts                                # SDK bridge, spawn/rotate/continue
│   ├── worker.ts                                # per-session engine worker
│   ├── admit.ts                                 # sync admission (<100 ms)
│   ├── store.ts / scheduler.ts / watchdog.ts    # runs.json, 30 s ticks
│   ├── notifier.ts / proxy.ts / tunnel.ts       # discord, LRU proxy, tunnel token
│   ├── auth.ts / ws.ts / statusView.ts          # auth, sockets, StatusItem
│   ├── routes/ (status, sessions, serverRoutes, misc)
│   ├── extensions/proxy-local.ts                # vendored proxy-local (native rotation)
│   ├── config/server.json                       # {"port": 5677, "concurrency": 8}
│   ├── config/subworkers.json                   # ★ ALL 25 subworker definitions
│   ├── data/ (runs.json, frames/, prompts/, state.json, proxy-*.json)
│   ├── Dockerfile                               # oven/bun:1-slim, ffmpeg + node 22
│   ├── docker-compose.yml                       # 5677:5677, mem_limit 4g
│   └── tunnelWatch.sh                           # host tunnel sidecar
└── subworkers/<name>/                           # ★ agent workspace homes ONLY
    ├── PROMPT.md                                # task instructions (every agent)
    ├── ANCHORED_MEMORY.md                       # (mirrorpay-telegram only)
    └── discovered_ideas.md / personality.md     # (reddit-saas-scraper only)
```

Rules:

- Code lives ONLY in `omp-server/`. Nothing executes from `subworkers/`.
- Agent homes hold PROMPT.md + small markdown/txt work files ONLY — zero code, zero servers, zero configs.
- `omp-server/docs/` otherwise holds upstream SDK docs; this file is the only Elia-specific doc there (the wrapper contract lives in `omp-server/SERVER.md` at the wrapper root).

---

## 3. Workspace Isolation

Every run executes with `cwd` = the agent's workspace (`createAgentSession({ cwd: workspace, … })` in `omp-server/worker.ts`; the parent spawns with `cwd: REPO_ROOT` and passes the workspace as argv).

### 3.1 PROMPT.md resolution (existence probe)

`effectiveWorkspace()` (`omp-server/auth.ts`) resolves the directory HOLDING `PROMPT.md` — i.e. the agent home, one level above `workspace/` (the `workspace/` subdir holds dated run outputs). It probes candidates and returns the first that actually contains `PROMPT.md`:

1. Repo home: `<root>/subworkers/<name>` (seeded homes in this checkout).
2. Host home: `~/EliaAI/subworkers/<name>` (`OPENCODE_WORKSPACE` override or its `/Users/vakandi/EliaAI` default).
3. Container path: `/data/subworkers/<name>` (bind-mount; compose maps `${SUBWORKERS_DIR:-${HOME}/EliaAI/subworkers}:/data/subworkers:ro`).

Registered `workspace` values in `omp-server/config/subworkers.json` are container-style (`/data`, `/data/subworkers/<name>/workspace`); the probe strips a trailing `/workspace` before testing, so all three layouts resolve.

### 3.2 Rules agents must obey

- Agents never write outside their workspace. The session `cwd` is the workspace; prompts (§11) pin the exact home path.
- Run outputs go under the `workspace/` subdir (dated folders + `HANDOFF_NEXT_SESSION.md`), never beside PROMPT.md except for the small carry-over files already present (see §2).
- The `data/` dir (`runs.json`, `frames/`, `prompts/`) is server-owned; agents never touch it.

---

## 4. Agent Registration

File: `omp-server/config/subworkers.json` — 25 entries, `"version": "1.0"`, **all `enabled: false` by default** (verified 20 Sep 2026).

### 4.1 Entry fields

Example (`elia`, lines 3–38):

```json
{
  "name": "elia",
  "enabled": false,
  "schedule": { "type": "interval", "hours": [9, …, 23], "minute": 0, "days": null },
  "prompt_file": "PROMPT.md",
  "workspace": "/data",
  "agent_id": "elia",
  "model": "opencode/muse-spark-1.2-contributor-free",
  "variant": "low",
  "max_retries": 3,
  "timeout_minutes": 60,
  "proxy_enabled": false,
  "mcp_servers": [],
  "notify_discord": false
}
```

| Field | Meaning |
|---|---|
| `name` | Registry key; matches the `subworkers/<name>/` home and every route param |
| `enabled` | Scheduler gate (`isEnabled` = override ?? entry default; `setEnabled` flips the in-memory override map) |
| `agent_id` | Personality id resolved via the agent dir (see §1) |
| `model` / `variant` | Per-run model override (nullable — falls back to agent default) |
| `schedule` | `interval {hours, minute, days?}` (daily clock times) · `every {every, hours?, days?}` (`every` in minutes) · `cron {expression}` (minimal 5-field matcher) |
| `workspace` | Container-style base; resolved by the §3.1 probe |
| `timeout_minutes` / `max_retries` | Watchdog expiry + scheduler retry budget |

Schedule math: `nextRunAfter(entry, from)` is strictly after `from`; null when unscheduled. `next_run` persists in `omp-server/data/state.json`.

### 4.2 Manage routes (all `Bearer`-gated)

| Method | Path | Effect |
|---|---|---|
| `POST` | `/enable/{name}` | `{status:"enabled"\|"already_enabled", name, enabled:true}` |
| `POST` | `/disable/{name}` | `{status:"disabled"\|"already_disabled", name, enabled:false}` |
| `PUT` | `/status/{name}` | Allowed keys `agent_id, model, variant, timeout_minutes, max_retries, schedule`; schedule validated (`every` 1..1440 min, else `interval`/`cron` object); empty → `422 {"detail":"No fields to update"}` |
| `POST` | `/config/reload` | `{status:"reloaded", added:[], removed:[], unchanged:[names], total}` — hot-reload, no restart |

Unknown subworker → `404 {"detail":"unknown subworker <name>"}`.

---

## 5. Trigger Template

There is no per-agent trigger script. One route admits every run — scheduled ticks call the same `admitRun` path as `POST /trigger`.

### 5.1 Exact trigger call

```bash
TOKEN=$ELIA_AUTH_TOKEN
curl -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"prompt":"hello"}' http://localhost:5677/trigger/elia
# {"status":"triggered","name":"elia","message":"Subworker 'elia' triggered successfully","session_id":"ses_…"}
```

- Body: `{prompt?, model?, variant?}` — all optional; registry values fill the gaps.
- Admission is synchronous (<100 ms, `omp-server/admit.ts`): the run record is written to `omp-server/data/runs.json` BEFORE the HTTP response.
- Over the concurrency cap (`MAX_CONCURRENT_RUNS`, default 8 from `config/server.json` `concurrency`): `202 {status:"queued_429", name, session_id}` — never silent, never a bare 429.
- Session ids: `ses_<epoch-ms-base36><rand>` via `snowflakeSessionId()` (`omp-server/store.ts`).

### 5.2 Continue / stop a live session

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"message":"keep going"}' http://localhost:5677/sessions/elia/ses_…/continue
# {"status":"continued","name":"elia","session_id":"ses_…","message":"Message sent to session …"}
# (body.message defaults to "continue the tasks"; engine re-inject is lazy, never awaited)

curl -X POST -H "Authorization: Bearer $TOKEN" \
  http://localhost:5677/sessions/elia/ses_…/stop
```

- `GET /sessions/{name}?limit=N&session_id=S` — transcript (`messages:[{info:{role,time_created}, parts:[{type:"text"|"reasoning"|"tool", …}]}]`; limit 1..500, default 50).
- `GET /sessions/{name}/list` — running first (`"▶ Running"`), then last.
- `GET /sessions/{name}/{id}/events` — SSE `text/event-stream`, replays buffered frames then clean EOF.

---

## 6. Tools & MCP

### 6.1 SDK tool surface

Session events normalize to TopBar frames `{field: "text"|"reasoning"|"tool"}` in exactly one place — `eventToFrames()` (`omp-server/engine.ts` + `omp-server/worker.ts`). Tool frames carry BARE names (`tool:"task"`, never decorated) plus `input` (call args) and `output` (truncated result). Named in code today: `task`, `subtask`, `hub`, `call_omo_agent`, `team_create`, `team_task_create` (subagent set, `SUBAGENT_TOOLS`), `todo*` events, and the browser/computer prelude definitions (pre-warmed so old-Bun `require()` of `with { type: "text" }` assets hits the module registry).

`read`/`bash`/`edit` run inside the session with `cwd` = the agent workspace (§3).

### 6.2 MCP parity (host ≡ container)

Compose bind-mounts (read-only — agents get the same list inside as outside, but cannot overwrite host secrets):

- `${HOME}/.config/mcp/mcp_servers.json:/root/.config/mcp/mcp_servers.json:ro`
- `${HOME}/.local/bin/mcp-cli:/usr/local/bin/mcp-cli:ro`
- `${HOME}/.omp/agent:/root/.omp/agent:rw` (persisted agent dir)

### 6.3 Image media/runtime deps

`omp-server/Dockerfile` installs `ffmpeg` (media pipelines: whisper, video, browser audio) + Node 22.17.0 from the official tarball with npm/npx (MCP server hosting; `mcp-cli` itself arrives via the §6.2 mount, never baked — no secrets in the image).

---

## 7. Proxy

`omp-server/proxy.ts` + vendored `omp-server/extensions/proxy-local.ts`, mirroring live `runner.py:323-385` + `forward-proxy.js`:

- **Per-session**: each worker child gets its own `HTTPS_PROXY` in an isolated env (`engine.ts` spawn) — concurrent runs keep distinct egress.
- **LRU pick**: oldest-used proxy first; assignment in `omp-server/data/proxy-map.json`, touch times in `omp-server/data/proxy-lru.json`. Pool from `~/EliaAI/setup/proxies.txt` (mounted `:ro`); `pickDistinctProxy` keeps simultaneous sessions on distinct endpoints.
- **Forced 4-min rotation**: `ROTATE_MS = 4 * 60 * 1000`. The parent timer writes `ROTATE <url>` over the child `stdin` pipe mid-session (`child.stdin.write(...)`); the worker's vendored proxy-local extension applies it natively (`ELIA_PROXY_LOG`, `PROXY_LOCAL_NO_SPAWN=1` in compose).
- **No quarantine anywhere** — poisoned sessions are abandoned fresh, never blocklisted.
- Model traffic goes through the session proxy; local shell (`bash`) runs direct.

---

## 8. Server (Bun wrapper)

`omp-server/server.ts` → `Bun.serve({ port, hostname: "127.0.0.1" })`, port from `omp-server/config/server.json` (`{"port": 5677, "concurrency": 8}`). Boot order: `reloadEntries()` → `setTriggerFn()` → `startScheduler()` (30 s tick) → `startWatchdog()` (30 s tick) → listen. The engine imports lazily — the listener boots without it. A 30 s broadcast keeps TopBar fresh.

- **Auth** (`omp-server/auth.ts`, mirrors live `app/core/auth.py`): `Authorization: Bearer <token>` OR `X-Elia-Token: <token>`; WS `/ws` also accepts `?token=` / `Sec-WebSocket-Protocol`. Empty/missing `ELIA_AUTH_TOKEN` → auth disabled. `GET /health` always open (`{"status":"ok","uptime_s":N}`); `/docs`, `/redoc`, `/openapi.json` → `404 {"detail":"not found"}`.
- **Scheduler** (`omp-server/scheduler.ts`): 30 s tick; triggers use the same admission path as `POST /trigger`.
- **Watchdog** (`omp-server/watchdog.ts`): no RSS-cap gate — container `mem_limit: 4g` is the only guard. Per-run `last_frame_at + started_at`; `STALL_TIMEOUT_S` (300) / `MAX_RUN_S` (3600). On expiry: mark `failed`, error banner on `/ws`, persist, Discord alert, release proxy. `checkOnce(nowMs)` returns expired session ids.
- **Notifier** (`omp-server/notifier.ts`): Discord webhook from `DISCORD_WEBHOOK_URL` env only (never logged); 300 s debounce per `(name, kind)`; 10 s fetch timeout; failures logged, never thrown.
- **Tunnel** (`omp-server/tunnel.ts` + `tunnelWatch.sh`): socketless — the manager only writes the runner token to `omp-server/config/tunnel.token` (chmod 600); the host sidecar polls (default 15 s) and starts/stops the sibling `cloudflared` service. Same 5-endpoint contract as live (status/check/setup/stop/remove). Never mounts `/var/run/docker.sock`.
- **Docker**: `EXPOSE 5677`, `HEALTHCHECK` on `http://localhost:5677/health`, `CMD ["bun", "server.ts"]`; compose sets `HOST: 0.0.0.0` so mapped 5677 is reachable in-container; `restart: unless-stopped`. Full route table: `omp-server/SERVER.md`.
- **testFrames gate**: `POST /test/frames/{name}` (auth-gated, emits text+reasoning+tool frames + banner without starting an engine) or `bun omp-server/testFrames.ts [name]` — TopBar on `:5677` must show all three streams.

---

## 9. EliaTopBar client

- **Base URL**: `http://localhost:5677` (`/server/health` reports `base_url:"http://127.0.0.1:5677"` — same listener, loopback literal).
- **WS events** (`/ws`):

| Event | When |
|---|---|
| `initial_status` | On WS open (`{…status, opencode_health, scheduler_running}`) |
| `status_update` | Every 30 s broadcast |
| `subworker_started` | Run admitted / continued (TopBar flips to LIVE) |
| `run_log` | One per frame (`{field:"text"\|"reasoning"\|"tool", …}`) |
| `run_banner` | Info banner (start, error, completion) |
| `subworker_completed` | Terminal state |
| `pong` | Reply to heartbeat `ping` — accepts raw `"ping"`, `{"type":"ping"}` (EliaTopBar), `{"event":"ping"}` |

- **Livestream fields**: frames are `{t, field: "text"|"reasoning"|"tool", text}` (`data/frames/<sessionId>.jsonl`); SSE replays `{name, session_id, field, text}` (qwen-shim shape for `LogPopoverView`).
- **Subagents panel**: tool parts carry BARE names (`tool:"task"`, `call_omo_agent`, `team_create`) plus `input` (call args incl. `description`, `subagent_type`, synthetic `sessionId: ses_…` minted per `toolCallId`) and `output`. `extractSubagents` requires the ses_-prefixed id — satisfied by construction.
- **Todos**: every tool frame carries `input`/`output`, so `todo*` calls flow into `extractTodos` (input/output/delta) with zero extra code.
- **Health**: `/server/health` → `{state:"running", health_status:"healthy", pid, base_url, restart_count, last_health_check:{ok:true, at}}` — `health_status` is the literal `"healthy"`.
- `StatusItem`: `{name, enabled, running, next_run, schedule_type, schedule, model, variant}` — `running` is registry-derived, never schedule-derived.

---

## 10. Lifecycle scripts

| Script | One-liner |
|---|---|
| `./install.sh [--no-docker]` | One-time: checks `bun>=1.3.14` + docker CLI, `bun install` in `omp-server/`, mints `omp-server/.env` (`ELIA_AUTH_TOKEN`, chmod 600) unless present, ensures data/config dirs, `docker build -t omp-server` unless `--no-docker`. |
| `./start.sh [--docker] [--with-tunnel]` | Default boots `bun omp-server/server.ts` backgrounded (log `data/server.log`, pid `data/server.pid`) + health gate; exits 0 if `:5677` already healthy (never disturbs); `--docker` runs compose up instead, `--with-tunnel` adds the cloudflared sidecar (`--profile tunnel`). |
| `./stop.sh [--with-colima]` | Stops the pidfile server + compose down; preserves `omp-server/data`; `--with-colima` also stops Colima. |
| `./reset.sh [--up] [--no-build]` | Compose down (incl. tunnel profile) + remove images + clear runtime data (`runs.json`, `frames/*`, `prompts/*`, `server.pid`, `server.log`), then rebuild; preserves `.env`, `subworkers.json`, `server.json`, `tunnel.token` (the token IS the domain binding); `--up` restarts WITH tunnel, `--no-build` skips the rebuild. |

Shared env: `omp-server/.env` is a verbatim copy of live `~/EliaAI/subworkers/server/.env` (same `ELIA_AUTH_TOKEN`, plus `IDLE_TIMEOUT`, `IDLE_CHECK_INTERVAL`, `MAX_CONCURRENT_RUNS`), chmod 600, gitignored — both stacks accept the same token. Tunnel token likewise copied to `omp-server/config/tunnel.token` (chmod 600, gitignored), so `/tunnel/status` reports `configured:true` out of the box.

---

## 11. Prompt prefix — workspace path rules

Every injected run prompt MUST pin these exact path rules (agents must use the repo homes — NEVER `/Users/vakandi/EliaAI/` paths in new work):

```
Your home is /Users/vakandi/Documents/omp-server/subworkers/<agent_name>/.
It holds PROMPT.md (your task instructions) plus small carry-over markdown files.
Run outputs go in the workspace/ subdir: workspace/YYYY-MM-DD/ for dated work,
HANDOFF_NEXT_SESSION.md for the rolling handoff, plus small dated verify files.
NEVER read from or write to /Users/vakandi/EliaAI/ paths in new work — that tree
is the read-only live reference, not your workspace. NEVER write outside your home.
```

Conventions:

- `workspace/` subdir for ALL run outputs (dated folders are idempotent — one per day).
- `HANDOFF_NEXT_SESSION.md` — rolling handoff the next run reads first.
- Dated verify files (e.g. `workspace/YYYY-MM-DD/*.md`) for per-run proof.
- PROMPT.md itself is read-only input — agents never rewrite it.
