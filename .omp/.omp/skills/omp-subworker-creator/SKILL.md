---
name: omp-subworker-creator
description: >-
  Create a new autonomous subworker in the omp-server system (~/Documents/omp-server).
  A subworker is 4 things only: a workspace home (subworkers/<name>/ with PROMPT.md),
  an OMP persona file (~/.omp/agent/agents/<name>.md, loaded as appended system prompt),
  a registry entry in omp-server/config/subworkers.json which schedules and triggers it
  (Bun wrapper on port 5677, worker-per-run), and a workspace/ subdir for run outputs.
  No opencode.json, no Docker FastAPI :5656. This skill handles the
  FULL pipeline: workspace dir → PROMPT.md → persona .md → subworkers.json declaration → reload → verify.
  Use this whenever the user says "create a subworker", "add a new agent", "make a promoter",
  "new sub-worker", "add subworker", or any request to create a scheduled autonomous agent
  for the omp-server ecosystem. DO NOT use the elia-subworker-creator (opencode/:5656)
  procedure here — paths, registration, and server contract are all different.
---

# OMP Subworker Creator

## Why This Skill Exists

The original `elia-subworker-creator` targets the old stack (`/Users/vakandi/EliaAI`,
`opencode.json` + `~/.config/opencode/agents/` + FastAPI on `:5656`). Applying it inside
`/Users/vakandi/Documents/omp-server` silently breaks agents (wrong paths, stale
`opencode-zen/*` models, `team_*` tools that don't exist on the SDK bridge). This skill
enforces the omp-server pipeline from `omp-server/docs/HOWTOADDAGENTS.md` +
`omp-server/docs/SUBWORKERS_SYSTEM.md`: **registry entry + PROMPT.md + workspace dir,
~5 minutes, hot-reload, no restart.**

## Overview — The 4-Thing Pipeline

```
subworkers/<name>/workspace/ (mkdir -p, session cwd)
  → subworkers/<name>/PROMPT.md (verbatim run prompt, OMP HOME block first)
    → ~/.omp/agent/agents/<name>.md (persona, appended system prompt — Step 3)
      → omp-server/config/subworkers.json (declare + schedule)
        → POST /config/reload (hot-load, no restart)
          → verify via GET /status/<name> + POST /trigger/<name>
```

No per-agent trigger script. No `opencode.json`. Scheduled ticks and
`POST /trigger` share the same `admitRun` path (`omp-server/admit.ts`, sync <100 ms,
run record fsync'd to `omp-server/data/runs.json` BEFORE the HTTP response).

**Runtime:** Bun wrapper (`omp-server/server.ts`) on **127.0.0.1:5677** → scheduler
(30 s tick) / watchdog (30 s tick, `STALL_TIMEOUT_S` 300 / `MAX_RUN_S` 3600) → per-run
worker (`Bun.spawn(["bun", "omp-server/worker.ts", …])`, `cwd` = agent workspace,
isolated `EventBus`, own `HTTPS_PROXY` per run, 4-min forced rotation, no quarantine).
Over concurrency cap (default 8): `202 {status:"queued_429"}` — never silent.

**Reference (read-only, never import):** live Python stack on `:5656`
(`~/EliaAI/subworkers/server/`) is contract parity reference only.

---

## Step 0: READ FIRST — Understand the existing patterns

**Before creating anything**, read these reference files:

```bash
cat /Users/vakandi/Documents/omp-server/omp-server/docs/SUBWORKERS_SYSTEM.md
cat /Users/vakandi/Documents/omp-server/omp-server/docs/HOWTOADDAGENTS.md
cat /Users/vakandi/Documents/omp-server/omp-server/config/subworkers.json | head -100
cat /Users/vakandi/Documents/omp-server/omp-server/config/server.json
```

Also read 1 minimal prompt to copy the anatomy (NOT an old EliaAI prompt):

```bash
cat /Users/vakandi/Documents/omp-server/subworkers/test-delegator/PROMPT.md
```

Key facts to confirm while reading:
- Registry shape: `{name, enabled, agent_id, model, variant, schedule, workspace,
  timeout_minutes, max_retries, prompt_file: "PROMPT.md"}` — 21 entries, all
  `enabled: false` by default.
- `model` MUST be a live `zen-free/*` id (e.g. `zen-free/muse-spark-1.3-contributor-free`).
  NEVER `opencode-zen/*` (no key exists for it — runs fail instantly). NEVER `opencode/*`
  (old-stack prefix). Variant ∈ `low|medium|high|max`.
- `agent_id` is display/routing metadata only — but `name` ALSO selects the persona:
  the engine passes it to `worker.ts`, which loads `~/.omp/agent/agents/<name>.md`
  (Step 3) as appended system prompt. No persona file = run still works, but generic.
- `workspace` = `/Users/vakandi/Documents/omp-server/subworkers/<name>` (container-style
  `/data/subworkers/<name>/workspace` also resolves via the §3.1 existence probe, but
  prefer the repo-absolute form — it matches all 21 live entries).

---

## Step 1: Create the workspace

```bash
AGENT=<name>
mkdir -p /Users/vakandi/Documents/omp-server/subworkers/$AGENT/workspace
```

Rules (`SUBWORKERS_SYSTEM.md` §2–§3, §11):
- `subworkers/$AGENT/` is the agent home: `PROMPT.md` + small markdown/txt carry-over
  files ONLY — zero code, zero servers, zero configs.
- `workspace/` subdir is the session `cwd` (created automatically if missing, but create
  it now so the first run has dated-output dirs ready).
- Run outputs go under `workspace/` (dated `workspace/YYYY-MM-DD/` folders +
  `HANDOFF_NEXT_SESSION.md` rolling handoff + small dated verify files). Never beside
  `PROMPT.md` except carry-over files already present.
- Agents never write outside their home. Never touch `omp-server/data/`
  (`runs.json`, `frames/`, `prompts/`, `state.json` — server-owned).

**Naming:** `name` = kebab-case `[a-z0-9-]` (e.g. `bene2luxe-suppliers`, `mirrorpay-telegram`).
It is the routing key for every route param — must match the folder name exactly.

---

## Step 2: Write `subworkers/<name>/PROMPT.md`

Prompts are injected VERBATIM as the run prompt (no frontmatter parsed, no templating) —
what you write is what the model receives. `PROMPT.md` itself is read-only input at
runtime — agents never rewrite it.

**First two lines (exact, non-negotiable):**

```md
# <Name>
> OMP HOME: /Users/vakandi/Documents/omp-server/subworkers/<name>/ — all reads/writes stay inside this home (plus its workspace/ subdir). Never use /Users/vakandi/EliaAI/ paths in new work; old outputs keep working via the live bind-mount.
```

**Required sections after that (same anatomy as the 21 shipped prompts):**
1. **MISSION** — what this subworker does, one sentence.
2. **WORKFLOW** — step-by-step instructions (numbered, with the exact tools to call).
3. **TOOLS** — `task`/`subtask`/`hub`/`todo` + `read`/`bash`/`edit` (cwd = workspace) +
   MCP via `mcp-cli` where needed. NEVER `team_*` (opencode-only, doesn't exist here).
4. **OUTPUT CONTRACT** — end every run path with a `HANDOFF_NEXT_SESSION.md` update +
   dated `workspace/YYYY-MM-DD/verify_*.md` report (TopBar + verifiers read these).
5. **LIMITS** — file I/O constrained to `workspace/`; never `/Users/vakandi/EliaAI/`
   paths in new work; never outside the home; never rewrite `PROMPT.md`.

**Tool-surface notes** (`SUBWORKERS_SYSTEM.md` §6):
- Session events normalize to TopBar frames `{field: "text"|"reasoning"|"tool"}` in
  `eventToFrames()`. Tool frames carry BARE names (`tool:"task"`, never decorated).
- Every injected run prompt MUST also pin the workspace path rules block (§11):

```
Your home is /Users/vakandi/Documents/omp-server/subworkers/<agent_name>/.
It holds PROMPT.md (your task instructions) plus small carry-over markdown files.
Run outputs go in the workspace/ subdir: workspace/YYYY-MM-DD/ for dated work,
HANDOFF_NEXT_SESSION.md for the rolling handoff, plus small dated verify files.
NEVER read from or write to /Users/vakandi/EliaAI/ paths in new work — that tree
is the read-only live reference, not your workspace. NEVER write outside your home.
```

**Do NOT copy old-stack PROMPT.md sections:** no `SOURCE DE VÉRITÉ` personality list,
no `~/.config/opencode/agents/` references, no `prompt_append`, no `team_*` calls.

---

## Step 3: Write `~/.omp/agent/agents/<name>.md` (persona)

Every run loads this file — frontmatter stripped, body capped at 12k chars — as
**appended** system prompt (`engine.ts` passes the entry `name` to `worker.ts`, which
feeds it via the SDK `appendSystemPrompt` seam; the full default system prompt is
preserved). Missing file = silent skip (run still works, but generic). No rebuild,
no reload: `~/.omp/agent` is mounted `:rw` into docker (`/root/.omp/agent`), so edits
go live on the next run, host and container alike.

OMP-native format (copy `~/.omp/agent/agents/cve-hunter.md` anatomy — NOT the opencode
`~/.config/opencode/agents/` format, which the engine never reads):

```md
---
name: <name>
description: "<one-liner>"
tools: read, write, edit, bash, glob, grep, task
model: zen-free/muse-spark-1.3-contributor-free
output: markdown
thinkingLevel: xhigh
---

# <Name> — Identity

...who the agent is, behavioral rules, reporting contract (Discord channel IDs, etc.)
```

**Rules:**
- `name` MUST equal the subworker `name` (routing key) — otherwise the worker never loads it.
- `model` MUST be `zen-free/*` (same roster rule as the registry entry).
- `tools` = comma-separated subset the persona needs (`read, write, edit, bash` minimum for hunters).
- Body: identity + behavioral rules + reporting contract. No secrets, no tokens.
- Verify after creation: trigger a `{"prompt":"Reply with your agent name and purpose"}` run —
  the reply must quote the persona, and docker logs must show
  `[worker] session=ses_… agent=<name> persona=<N> chars` (not `persona=none`).

---

## Step 4: Register in `omp-server/config/subworkers.json`

Append one entry to `/Users/vakandi/Documents/omp-server/omp-server/config/subworkers.json`
(`"version": "1.0"`, top-level `"subworkers": [...]` array):

```json
{
  "name": "<name>",
  "enabled": false,
  "agent_id": "<name>",
  "model": "zen-free/muse-spark-1.3-contributor-free",
  "variant": "low",
  "schedule": { "type": "interval", "hours": [9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23], "minute": 0, "days": null },
  "prompt_file": "PROMPT.md",
  "workspace": "/Users/vakandi/Documents/omp-server/subworkers/<name>",
  "timeout_minutes": 30,
  "max_retries": 2
}
```

**Field notes:**
- `name` = routing key, matches folder + `agent_id` (display/routing metadata).
- `enabled: false` until proven — enable via TopBar or `POST /enable/{name}` only after
  the verify run completes with real model tokens.
- `model` = live `zen-free/*` id (check `GET /models` dropdown / `omp-server/data/models.json`
  for the live roster with NEW badges). `variant` = thinking effort (`low/medium/high/max`).
- `schedule` one of: `interval {hours, minute, days?}` (daily clock times) ·
  `every {every, hours?, days?}` (`every` in minutes, 1..1440) · `cron {expression}`
  (minimal 5-field matcher). `next_run` persists in `omp-server/data/state.json`.
- `prompt_file` always `"PROMPT.md"`. `timeout_minutes` / `max_retries` = watchdog
  expiry + scheduler retry budget.
- Manage routes (all `Bearer`-gated): `POST /enable/{name}`, `POST /disable/{name}`,
  `PUT /status/{name}` (allowed keys `agent_id, model, variant, timeout_minutes,
  max_retries, schedule`; empty → `422 {"detail":"No fields to update"}`),
  `POST /config/reload` (hot-reload, returns `{added, removed, unchanged, total}`).
  Unknown subworker → `404 {"detail":"unknown subworker <name>"}`.

**Main agent (`elia`):** same pipeline — no special files. The live entry uses the
standard home (`subworkers/elia`, `prompt_file: PROMPT.md`); designation as main is a
TopBar/manager concern (`omp-server/data/main-agent.json`), not a registry difference.

---

## Step 5: Hot-load without restart + verify (host, no TopBar needed)

```bash
T=$(grep ELIA_AUTH_TOKEN /Users/vakandi/Documents/omp-server/omp-server/.env | cut -d= -f2)
# 1. Hot-reload the registry (no restart, no docker)
curl -s -X POST -H "Authorization: Bearer $T" http://127.0.0.1:5677/config/reload
# 2. Entry resolves
curl -s -H "Authorization: Bearer $T" http://127.0.0.1:5677/status/<name> | head -c 500; echo
# 3. Verify run (expect {"status":"triggered",...,"session_id":"ses_…"(<100ms)})
curl -s -X POST -H "Authorization: Bearer $T" -H 'content-type: application/json' \
  -d '{"prompt":"Reply with exactly: AGENT OK"}' http://127.0.0.1:5677/trigger/<name>
# 4. Watch frames (text + reasoning + tool must all appear)
tail -f /Users/vakandi/Documents/omp-server/omp-server/data/frames/<session_id>.jsonl
# TopBar: open the agent popup → livestream + subagents + todos appear
```

**Auth:** `Authorization: Bearer <token>` OR `X-Elia-Token: <token>` (mirrors live
`app/core/auth.py`); `GET /health` always open (`{"status":"ok","uptime_s":N}`).
Empty/missing `ELIA_AUTH_TOKEN` → auth disabled (host dev only).

**Continue / stop a live session:**

```bash
curl -X POST -H "Authorization: Bearer $T" -H 'content-type: application/json' \
  -d '{"message":"keep going"}' http://127.0.0.1:5677/sessions/<name>/<session_id>/continue
curl -X POST -H "Authorization: Bearer $T" \
  http://127.0.0.1:5677/sessions/<name>/<session_id>/stop
```

`GET /sessions/{name}?limit=N`, `GET /sessions/{name}/list` (running first),
`GET /sessions/{name}/{id}/events` (SSE replay then clean EOF).

**Synthetic gate (no engine):** `POST /test/frames/{name}` emits text+reasoning+tool
frames + banner, or `bun omp-server/testFrames.ts [name]` — TopBar on `:5677` must show
all three streams.

---

## Step 6: Ship it

`subworkers/<name>/` (minus `workspace/*.log`, media >25M — see root `.gitignore`)
commits with the repo. Runtime state (`omp-server/data/`) never commits. Enable it
(`POST /enable/{name}`) only after the verify run completes with real model tokens.

---

## Common Mistakes — DO NOT REPEAT

| Mistake | Consequence | Correct |
|---------|-------------|---------|
| Following `elia-subworker-creator` (opencode.json + agents/*.md) | Files land in the wrong tree, server never sees the agent | 4 things only: workspace dir + PROMPT.md + `~/.omp/agent/agents/<name>.md` + `omp-server/config/subworkers.json` |
| Creating `~/.config/opencode/agents/<name>.md` or touching `opencode.json` | Dead files — engine reads OMP personas from `~/.omp/agent/agents/` only | OMP-native `<name>.md` in `~/.omp/agent/agents/` (Step 3); never the opencode dir |
| Skipping the persona file | Runs work but generic — no identity, no reporting contract | Always create Step 3; verify with the name+purpose probe run |
| Persona `name` ≠ subworker `name` | Worker never finds the file (`persona=none` in logs) | Identical kebab-case in both places |
| Using `/Users/vakandi/EliaAI/` paths in PROMPT.md or workspace | Writes hit the read-only live reference, not the agent home | OMP HOME block + §11 path rules; all I/O inside `subworkers/<name>/` |
| Missing OMP HOME block or `# <Name>` first lines | Agent drifts out of its home on run 1 | Exact two-line header from Step 2, then §11 rules block in the prompt body |
| `model: opencode-zen/*` or `opencode/*` | Instant run failure (no key) / old-stack prefix | Live `zen-free/*` id only (see `GET /models`) |
| Using `team_*` tools in the workflow | Tool calls fail — SDK bridge exposes `task`/`subtask`/`hub`/`todo` | `task`/`subtask`/`hub`/`todo`, never `team_*` |
| Frontmatter/templating in PROMPT.md | Parsed as literal prompt text (injected verbatim) | Plain markdown, no frontmatter |
| Forgetting `POST /config/reload` | Scheduler still runs the old registry | Reload after every registry edit; verify `GET /status/<name>` → 200 |
| `docker compose restart` / touching `:5656` | Restarts the wrong stack (live reference) | Host `./start.sh` / `bun omp-server/server.ts` on `:5677`; never `:5656` |
| Committing `omp-server/data/` | Runtime state (runs, frames, tokens) lands in git | Data dir is gitignored — ship `subworkers/<name>/` only |
| Writing code/servers/configs under `subworkers/` | Violates §2 (homes hold PROMPT.md + work files only) | Code lives ONLY in `omp-server/` |

## Verification Checklist

Before saying the subworker is ready, verify ALL of these:

```
[x] Step 1: subworkers/<name>/workspace/ exists (session cwd)
[x] Step 2: subworkers/<name>/PROMPT.md exists, starts with `# <Name>` + exact OMP HOME block, no frontmatter, no EliaAI paths, no team_* tools
[x] Step 3: ~/.omp/agent/agents/<name>.md exists, OMP-native frontmatter (name = subworker name, model zen-free/*), identity body
[x] Step 4: omp-server/config/subworkers.json has the entry (name = folder = agent_id, prompt_file PROMPT.md, workspace repo-absolute, model zen-free/*, variant, schedule, enabled: false)
[x] Step 5: POST /config/reload → entry in `unchanged`/`added`; GET /status/<name> → 200 with your fields
[x] Step 6: POST /trigger/<name> → {"status":"triggered","session_id":"ses_…"} (<100ms); frames/<session_id>.jsonl streams text+reasoning+tool; TopBar popup shows livestream
[x] Persona proof: name+purpose probe run quotes the persona; docker logs show persona=<N> chars
[x] Ship: only subworkers/<name>/ staged (no omp-server/data/, no *.log, no media >25M); still enabled: false until user approves
```

Use this checklist in your response to confirm completion.
