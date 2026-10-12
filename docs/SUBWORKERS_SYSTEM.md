# Subworkers System

**Date:** 12 October 2026
**Version: 5.0 — Rust runtime (OpenCodeRust replaces the Docker/TS stack)**

> Technical documentation for the autonomous subworker agent system.
>
> **Current runtime**: Fully native Rust. Control plane = Bun/TypeScript on `127.0.0.1:5656`; the agent engine = **OpenCodeRust** binary on `127.0.0.1:4096`. No Docker, no container, no `opencode` CLI from npm. Both run on the host.
> **Purpose**: Single binary to build and reason about, direct filesystem access for debugging, no Colima VM tax. **Security isolation is WEAKER than v4.5** — the Docker container was a real boundary; on the host it is not. See §3.
> **Previous version**: `SUBWORKERS_SYSTEM_OLD.md` (v4.5 — Python FastAPI in `elia-subworker-srv`, OpenCode serve on 5655 in-container).
> **Historical copy**: `/Users/vakandi/EliaAI/subworkers/SUBWORKERS_SYSTEM.md` (v4.5 — do not follow it for current behaviour).

---

## Table of Contents

1. [Overview](#1-overview)
2. [Directory Structure](#2-directory-structure)
3. [Workspace Isolation & Per-Agent Permissions](#3-workspace-isolation--per-agent-permissions)
4. [Agent Registration](#4-agent-registration)
5. [Trigger & Control](#5-trigger--control)
6. [Tools & MCP Servers](#6-tools--mcp-servers)
7. [Runtime: Start / Stop / Diagnose](#7-runtime-start--stop--diagnose)
8. [Observability: Frames, Runs, Livestream](#8-observability-frames-runs-livestream)
9. [Known Bugs & Open Defects](#9-known-bugs--open-defects)

---

## 1. Overview

### What Are Subworkers?

Subworkers are autonomous AI agents that run on a schedule or on demand. Each subworker has:

- A **personality file** (`config-home/agents/<agent-id>.md`) — YAML frontmatter (name, model, tools, permissions) + body
- A **PROMPT.md** (`subworkers/<name>/PROMPT.md`) — task instructions, injected as the message content. **Line 2 is a mandatory `OMP HOME` header**
- A **workspace** (`subworkers/<name>/workspace/`) — all outputs, logs, scratch files
- A **schedule entry** (`control/config/subworkers.json`) — schedule, enabled state, model, timeout
- A **runner** — the control plane drives runs over the OpenCodeRust HTTP API (`POST /session` then `POST /session/{id}/prompt` on `:4096`)

### Architecture

```
EliaTopBar / curl / WebSocket :5656
  │
  └── subworker-opencode-rust/control (Bun, TypeScript)      ← control plane
        ├── server.ts        REST + WebSocket, auth via ELIA_AUTH_TOKEN
        ├── scheduler.ts     every / cron / interval, persists next_run
        ├── engine.ts        spawns a run, streams frames
        ├── promptPrefix.ts  rewrites OMP HOME anchors into the active repo root
        └── ws.ts            broadcasts subworker_started / run_log / status_update
              │
              │ HTTP + Bearer token
              ▼
        OpenCodeRust binary :4096                            ← agent engine (Rust)
              ├── server/crates/opencode-server     REST / SSE / WebSocket
              ├── server/crates/opencode-session    the prompt loop, tool dispatch
              ├── server/crates/opencode-agent      agents + permission evaluation
              ├── server/crates/opencode-tool      read/write/bash/glob/grep/web*
              ├── server/crates/opencode-provider  Zen / OpenAI / Anthropic adapters
              └── server/crates/opencode-proxy     per-session residential egress
              │
              ▼
        Proxy :19400  →  residential pool  →  Zen API / web

Frames written to control/data/frames/<session_id>.jsonl  (the only complete record)
```

### Control & Monitoring

| Endpoint | Method | Purpose |
|---|---|---|
| `/status` | GET | Which agents exist, running, enabled |
| `/status/<name>` | GET / PUT | One agent |
| `/trigger/<name>` | POST | Run it now, ignore the schedule |
| `/enable/<name>` | POST | Enable (persists across restart) |
| `/disable/<name>` | POST | Disable |
| `/config/reload` | POST | Re-read `subworkers.json` |
| `/ws` | WS | Live events |
| `/server/health` | GET | Control plane liveness |

All require `Authorization: Bearer $ELIA_AUTH_TOKEN`.

---

## 2. Directory Structure

```
subworker-opencode-rust/
├── control/                     control plane (Bun/TypeScript)
│   ├── server.ts scheduler.ts engine.ts ws.ts promptPrefix.ts
│   ├── config/subworkers.json   agent registry + schedules
│   └── data/
│       ├── runs.json            one entry per run (status, session_id, error)
│       └── frames/*.jsonl       transcript, one JSON per line
├── config-home/                 the config the Rust child reads
│   ├── opencode.json            providers (no MCP declared — see §6)
│   └── agents/<agent-id>.md     personality + permissions
├── subworkers/<name>/
│   ├── PROMPT.md                task instructions + OMP HOME header (line 2)
│   └── workspace/               outputs, logs, scratch
├── control/data/serve-rust.sh   canonical child launcher
├── start.sh                     control plane ONLY — does not start the child
└── server/                      the Rust workspace
```

### Creating a New Subworker

1. Create the folder and the PROMPT.md:

```bash
mkdir -p subworkers/<name>/workspace
```

2. Line 2 of `PROMPT.md` is mandatory:

```
> OMP HOME: /Users/vakandi/Documents/subworker-opencode-rust/subworkers/<name>/ — all reads/writes stay inside this home (plus its workspace/ subdir).
```

3. Personality file `config-home/agents/<name>.md` (see §4 for the permissions block).
4. Registry entry in `control/config/subworkers.json` (see §5).
5. Enable + trigger (see §5).

---

## 3. Workspace Isolation & Per-Agent Permissions

### 3.1 The honest status

The Docker container in v4.5 was a **real** boundary: no docker socket, no `/`. That boundary is gone. Today isolation is **application-level only**, and **it is incomplete**.

Measured on 2026-10-12 with `refund-hunter`:

| Attempt | Expected | Actual |
|---|---|---|
| `read /Users/vakandi/.zshrc` | denied | ✅ **denied** |
| `bash ls /Users/vakandi` | denied | ❌ **ran anyway** |
| `read .../subworkers/elia/PROMPT.md` (inside allowed tree) | allowed | ❌ **wrongly denied** |

**1 of 3.** Better than nothing. Not a sandbox. **Do not treat any agent as contained while `bash` works.**

### 3.2 Three defects that were fixed

| # | Defect | Consequence | Fix |
|---|---|---|---|
| 1 | `evaluate()` was called with a hardcoded `"*"` pattern | A path could never be tested; `read: allow` meant the whole disk | The tool call's real path is now passed |
| 2 | Config key is `permissions:` (plural); the struct reads `permission:` (singular) | **Every rule in all 22 agent files was silently discarded** | `#[serde(alias = "permissions")]` added |
| 3 | `ToolContext.ask` was never wired on the real run path | No permission check ran at all | Rules evaluated in `ask_permission` |

Defect 2 is the worst one: the rules were written correctly in the files all along and **nothing read them**.

### 3.3 Per-agent permissions template

In `config-home/agents/<name>.md`:

```yaml
permissions:
  bash: deny
  read:
    "*": deny
    "/Users/vakandi/Documents/subworker-opencode-rust/subworkers/*": allow
  write:
    "*": deny
    "/Users/vakandi/Documents/subworker-opencode-rust/subworkers/*": allow
  edit:
    "*": deny
    "/Users/vakandi/Documents/subworker-opencode-rust/subworkers/*": allow
  grep:
    "*": deny
    "/Users/vakandi/Documents/subworker-opencode-rust/subworkers/*": allow
  glob:
    "*": deny
    "/Users/vakandi/Documents/subworker-opencode-rust/subworkers/*": allow
```

The main agent (`elia`) additionally gets:

```yaml
    "/Users/vakandi/Documents/EliaAI/*": allow
```

### 3.4 Two config traps

- **Order is inverted from intuition.** The scan starts at the end of the ruleset and the first match wins. A `"*": deny` placed **last** shadows every specific rule. `"*": deny` must come **first**.
- **`/**` does not work.** The matcher strips one trailing `*` and tests `starts_with`, so `**` leaves a literal asterisk in the prefix and matches nothing. Use a single `*` — it already covers nested folders.

### 3.5 What is NOT path-filterable

**`bash`.** A command is text, not a path. There is no way to scope it. Until `bash` is genuinely blocked, an agent with bash can reach the whole machine. This is the single most important gap in the current system.

### 3.6 Known permission-name mismatch

The file tools do not ask for `read` when a path sits outside the session worktree — they ask for **`external_directory`**. A config that only defines `read` will therefore deny legitimate files inside `subworkers/`. Fix pending: the config must cover `external_directory`, and the tool-side permission name should be normalised.

---

## 4. Agent Registration

### 4.1 Personality file — `config-home/agents/<agent-id>.md`

```yaml
---
name: refund-hunter
slug: refund-hunter
description: RefundHunter — refund policy & resale research
model: opencode/space-bunny-free
variant: xhigh
temperature: 0.3
mode: all
tools:
  read: true
  write: true
  edit: true
  bash: true
  glob: true
  grep: true
  webfetch: true
  task: true
  websearch: true
permissions:
  bash: deny
  read:
    "*": deny
    "/Users/vakandi/Documents/subworker-opencode-rust/subworkers/*": allow
output:
  type: object
  properties:
    agent: { type: string }
    date: { type: string, format: date }
    results:
      type: array
      items:
        type: object
        properties:
          shop: { type: string }
          url: { type: string, format: uri }
          platform: { type: string }
          score: { type: integer, minimum: 0, maximum: 10 }
          verdict: { type: string, enum: ["🟢 CIBLE", "🟡 PARTIEL", "🔴 NON"] }
          evidence: { type: array, items: { type: string } }
        required: [shop, url, platform, score, verdict, evidence]
  required: [agent, date, results]
---
```

`permissions:` is the key actually used by every existing agent file. The Rust struct reads `permission:`; an alias bridges the two. **If the alias is ever removed, every rule in every agent file becomes a silent no-op.**

### 4.2 Verify registration

```bash
curl -s -H "Authorization: Bearer $ELIA_AUTH_TOKEN" \
  http://127.0.0.1:5656/status/refund-hunter | jq
```

`enabled: false` here is the single most common reason "nothing happens" — the scheduler skips disabled agents entirely.

---

## 5. Trigger & Control

### 5.1 Registry — `control/config/subworkers.json`

```json
{
  "name": "refund-hunter",
  "enabled": true,
  "schedule": { "type": "every", "every": 60, "hours": [9,10,11,12,13,14,15,16,17,18,19,20,21] },
  "workspace": "/Users/vakandi/Documents/subworker-opencode-rust/subworkers/refund-hunter/workspace",
  "agent_id": "refund-hunter",
  "model": "opencode/space-bunny-free",
  "variant": medium,
  "max_retries": 3,
  "timeout_minutes": 10,
  "proxy_enabled": false,
  "mcp_servers": [],
  "notify_discord": false
}
```

### 5.2 Schedule semantics

- `every {every, hours}` — clock-aligned, only inside `hours`
- `interval {hours, minute}` — fixed gap
- `cron {expression}` — standard cron
- Outside the configured hours the agent simply does not run. `POST /trigger/<name>` bypasses this.

### 5.3 Manual trigger

```bash
set -a; . ./.env; set +a
curl -X POST -H "Authorization: Bearer $ELIA_AUTH_TOKEN" \
  http://127.0.0.1:5656/trigger/refund-hunter
```

Returns a `session_id` immediately. The run proceeds in the background.

---

## 6. Tools & MCP Servers

### 6.1 Built-in Rust tools

`bash`, `read`, `write`, `edit`, `multiedit`, `apply_patch`, `glob`, `grep`, `codesearch`, `ls`, `todo`, `task`, `skill`, `webfetch`, `websearch`, `lsp`.

### 6.2 MCP servers: none

`config-home/opencode.json` declares **no MCP at all**. An agent has only the built-ins above.

An agent that starts installing Playwright, or hunts for a browser tool, is doing so **because the MCP is missing** — not because it is confused. Declaring an MCP means adding it under `mcp:` in `config-home/opencode.json` and restarting the child.

The TypeScript OpenCode config at `~/.config/opencode/opencode.json` declares `parallel-browser-mcp`, `undetected-browser`, `personal_browser` — **the Rust child does not read that file.**

### 6.3 Third-party engines

- `websearch` / `webfetch` in `server/crates/opencode-tool/src/` — the Exa-backed search carries a **free-tier rate limit**; parallel sessions get HTTP 429 and the agent then reports the failure rather than silently retrying.
- `curl` from inside `bash` is often the only way past a WAF/anti-bot block; `webfetch` gets 403 on several French retail sites.

---

## 7. Runtime: Start / Stop / Diagnose

### 7.1 Start

```bash
# control plane :5656
./start.sh

# Rust child :4096 — start.sh does NOT do this
nohup bash control/data/serve-rust.sh > /tmp/oclogs/child.log 2>&1 &
```

`serve-rust.sh` matters: it starts from a neutral cwd (the plugin loader hangs forever from the repo root), exports `ELIA_AUTH_TOKEN`, and points `OPENCODE_CONFIG_DIR` at `config-home`.

### 7.2 Health

```bash
curl -H "Authorization: Bearer $ELIA_AUTH_TOKEN" http://127.0.0.1:4096/health   # → 200
curl -H "Authorization: Bearer $ELIA_AUTH_TOKEN" http://127.0.0.1:5656/server/health
lsof -nP -iTCP:19400 -sTCP:LISTEN      # residential proxy
```

`/health` returning 200 proves **nothing about inference**. A 200 there coexisted for hours with every model call failing `403 FreeTierError`.

### 7.3 Build

```bash
cd server
cargo build --release --bin opencode
touch crates/<crate>/src/<file>.rs     # cargo will not notice a stale mtime otherwise
```

> **Trap**: `cargo build` prints `Finished in 0.4s` and does *not* recompile after a scripted edit when mtimes are stale. `touch` the file. A patch can sit in the source tree and never reach the binary — this happened repeatedly.

---

## 8. Observability: Frames, Runs, Livestream

### 8.1 Frames are the only complete record

There is no HTTP API that serves them. `/status` tells you who is enabled, nothing about what happened.

```bash
scripts/collect.py --agent refund-hunter
scripts/collect.py --all --limit 5 --only-problems
scripts/collect.py --session ses_xxxx --json
```

Each line:

```json
{"t": 1791422405185, "field": "text", "text": "…"}
{"t": 1791422408140, "field": "tool", "tool": "read", "text": "▸ read", "input": {"file_path": "…"}}
{"t": 1791422411425, "field": "tool", "tool": "read", "text": "▸ read", "output": "Error: File not found"}
```

A tool call is **two frames with no shared id** — one with `input`, the next with `output`. `field` is `text`, `reasoning`, or `tool`.

### 8.2 Run records

`control/data/runs.json` — `name`, `session_id`, `status`, `started_at`, `error`. The `error` field is often more direct than the transcript: `"engine error: ReferenceError: finish is not defined"` is a bug, not a thinking problem.

### 8.3 Livestream — verified

```
t=+0.3s  busy
t=+3.2s  first assistant token
t=+3.5s  tool executed
t=+5.9s  idle
```

Five distinct text deltas; 55 frames over 120 s growing continuously. Frames arrive **during** the run, not batched at the end.

```bash
curl -N -H "Authorization: Bearer $ELIA_AUTH_TOKEN" http://127.0.0.1:4096/event
```

### 8.4 Reading failure signatures

| Symptom | Meaning |
|---|---|
| frames file = 2 lines, prompt only | died before the first token (provider/proxy) |
| `running: true` for a long time | a turn was admitted — proves nothing; check `last_frame_at` |
| every tool returns `Error:` | wrong path, not bad reasoning |
| empty tool output, agent proceeds | silent wrongness, worse than a loud error |
| "I'll search now…" then silence | the narration bug: loop exits before results are read |

---

## 9. Known Bugs & Open Defects

| Area | Defect | Severity |
|---|---|---|
| Permissions | `bash` is not actually blocked despite `bash: deny` | **critical** |
| Permissions | `external_directory` not covered by config → false denials inside the allowed tree | high |
| Permissions | No real sandbox on the host (Docker boundary lost in v5.0) | high |
| Permissions | `opencode-agent/executor.rs` path is not the one HTTP runs use | medium |
| Config | `permissions:` plural relies on a serde alias; losing it is silent | medium |
| MCP | No MCP configured for any agent | medium |
| Title | Session title generation returns 403 → default title used (cosmetic) | low |
| Proxy | Exa free tier caps concurrency at ~1 session | low |
| Paths | `/Users/vakandi/EliaAI/subworkers/` still exists but is **no longer read** | low |
| Builds | Stale-mtime silent no-rebuild | low |

### Unresolved after systematic elimination

The **session-title 403** was chased with measured hypotheses, all eliminated: missing `tools` array, low `max_tokens`, `temperature`, `system` folding, `session_id` threading, rate limiting (delayed retry). No cause identified. It is cosmetic — the title falls back — but it fires on every new session.

### Process rule that came out of this

A repeated automated directive (`Fix all bugs in the payment system`) consumed hours before anyone noticed **there is no payment system in this repo**. A directive from a loop cannot *create* a task, only prioritise an existing one. When blocked, ask once with a concrete option — do not explore an unrelated codebase.