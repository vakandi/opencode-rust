---
name: omp-subworker-run-verification
description: "Verify an omp-server subworker run — check config, live status on :5677, frames output, reasoning quality, and messages sent to confirm it works properly and report what it did. Use whenever the user says 'verify subworker', 'check if agent X is working', 'is the subworker running properly', 'audit subworker', 'what did agent X do', 'verify run for [agent]', or any request to audit a subworker's health and output."
compatibility: bash, python3, mcp-cli (macos-popup optional), omp-server on :5677 (host or docker)
---

# OMP Subworker Run Verification

Verify that a named omp-server subworker is running correctly — config sane, live status healthy, frames flowing, reasoning present, messages delivered — and report findings **only to the requester**.

Base paths below are repo-relative to `/Users/vakandi/Documents/omp-server` (the omp checkout). Server: `127.0.0.1:5677` (host `bun omp-server/server.ts` or docker `omp-server-omp-server-1`). Auth: `grep ELIA_AUTH_TOKEN omp-server/.env | cut -d= -f2`.

## Workflow

### Phase 0 — Resolve Agent Name

- Input: agent name (e.g. `refund-hunter`, `job-hunter-ch-lu`, `cve-hunter`, `app-clone-factory`, `deploymates-watch`)
- If no name given, ask: "Which subworker?"
- Normalize: strip, lower, replace spaces with `-`

### Phase 1 — Config Check (30s)

```bash
python3 -c "import json; d=json.load(open('omp-server/config/subworkers.json')); print(json.dumps(next((s for s in d['subworkers'] if s['name']=='<AGENT>'), 'NOT FOUND'), indent=1))"
head -30 subworkers/<AGENT>/PROMPT.md
python3 -c "import json; print(json.load(open('omp-server/data/state.json')).get('enabled', {}).get('<AGENT>', 'no-override'))"
```

- Is it registered? `prompt_file: "PROMPT.md"`? Workspace repo-absolute (`/Users/vakandi/Documents/omp-server/subworkers/<AGENT>`)?
- Effective enabled = `state.json` override ?? entry `enabled` (overrides survive restarts — a `false` entry with no override means it won't run at all).
- What `schedule` (every/interval/cron, hours)? What `model`/`variant`?
- Model MUST be `zen-free/*` (never `opencode-zen/*` — no key exists, runs fail instantly).
- PROMPT.md starts with `# <Name>` + exact OMP HOME block? No frontmatter, no `team_*`, no `/Users/vakandi/EliaAI/` paths in new work? (There is NO personality file on omp — the engine never loads by `agent_id`.)

### Phase 1.5 — Live Status Check (SOURCE OF TRUTH for running state)

Local workspace files only show **finished** work — they NEVER tell you what is running RIGHT NOW.
The live state lives on `:5677` (host process or the `omp-server-omp-server-1` container, Bearer auth).
**Always query it — this is the source of truth for `running: true/false`.**

```bash
T=$(grep ELIA_AUTH_TOKEN omp-server/.env | cut -d= -f2)
# Health first (open endpoint)
curl -s -m 10 http://127.0.0.1:5677/health
# One agent (live: enabled, running, next_run, model, timeout, max_retries)
curl -s -m 10 -H "Authorization: Bearer $T" http://127.0.0.1:5677/status/<AGENT> | python3 -m json.tool
# All agents + who is running right now
curl -s -m 10 -H "Authorization: Bearer $T" http://127.0.0.1:5677/status | python3 -c "
import json,sys
d=json.load(sys.stdin)
for s in d.get('subworkers', d if isinstance(d,list) else []):
    flag = 'RUNNING' if s.get('running') else 'idle   '
    print(f\"{flag} | {s.get('name')} | enabled={s.get('enabled')} | next_run={s.get('next_run')} | {s.get('model')}/{s.get('variant')}\")"
```

- If `running: true` → the agent is executing RIGHT NOW (report this first — user may be looking at TopBar).
- If `running: false` → fall back to frames + workspace (Phase 2) for the last completed run.
- If connection refused / 401 → say so explicitly, don't guess. Check `docker ps | grep omp-server-omp-server-1` (docker) or `cat omp-server/data/server.pid` (host).
- Other useful live endpoints (same Bearer header): `GET /logs/<AGENT>?lines=N`, `GET /sessions/<AGENT>/list`, `GET /sessions/<AGENT>/<id>/events` (SSE replay then clean EOF). `POST /trigger/<AGENT>` and `POST /test/frames/<AGENT>` are ACTIONS — never call either during read-only verification (test-frames emits synthetic frames + banner, pollutes evidence).

### Phase 1.6 — Live Session Proof (MANDATORY before any 🟢)

`running: true` is a CLAIM, not proof. A stuck flag, a wedged worker, and a working run all report
`running: true`. Prove it by reading the live session's frames:

```bash
T=$(grep ELIA_AUTH_TOKEN omp-server/.env | cut -d= -f2)
# Find the live session id (running first)
curl -s -m 15 -H "Authorization: Bearer $T" "http://127.0.0.1:5677/sessions/<AGENT>/list" | python3 -m json.tool | head -30
# Replay its events — look for text/reasoning/tool frames
curl -s -m 15 -H "Authorization: Bearer $T" "http://127.0.0.1:5677/sessions/<AGENT>/<SES_ID>/events" | head -20
# Or read the frames file directly (session cwd = server data dir)
python3 -c "
import json
rows=[json.loads(l) for l in open('omp-server/data/frames/<SES_ID>.jsonl')]
from collections import Counter
print('frames:', len(rows), Counter(r.get('field','?') for r in rows))
texts=[r.get('text','') for r in rows if r.get('field')=='text']
print('text_chars:', sum(len(t) for t in texts))
print('last:', (texts[-1] if texts else 'NONE')[:200])"
```

Verdict rules for the LIVE run (no exceptions):
- Frames file has text/reasoning/tool frames with real content → genuinely working → 🟢 allowed.
- Session exists but zero frames after >10 min → `EMPTY_OUTPUT` failure → ❌, say "empty after N min".
- Only the trigger prompt, no frames yet, run age <10 min → ⏳ nascent, say "started N min ago, no output yet" — NEVER 🟢.
- Session query errors / no session → ⚠️ unverifiable, say so — NEVER 🟢.
- `GET /logs/<AGENT>` returns `total_lines: 0` AND no frames → corroborates "nothing produced" → ❌.

### Phase 2 — Latest Run Discovery (60s)

```bash
# Server-side sessions for this agent (running first)
T=$(grep ELIA_AUTH_TOKEN omp-server/.env | cut -d= -f2)
curl -s -m 10 -H "Authorization: Bearer $T" "http://127.0.0.1:5677/sessions/<AGENT>?limit=3" | python3 -m json.tool | head -40
# Admission history
python3 -c "import json; [print(r.get('started_at',r.get('created_at','?')), r.get('name'), r.get('status'), r.get('session_id')) for r in json.load(open('omp-server/data/runs.json')) if r.get('name')=='<AGENT>']" | tail -5
# Workspace daily folder + handoff
ls -lt subworkers/<AGENT>/workspace/ 2>/dev/null | head -10
cat subworkers/<AGENT>/workspace/HANDOFF_NEXT_SESSION.md 2>/dev/null | head -50
ls -lt subworkers/<AGENT>/workspace/20*/ 2>/dev/null | head -10
```

- When was the last run? `completed`, `failed`, or `queued_429` (over concurrency cap — scheduler will retry, not a failure)?
- What `session_id`? Do its frames exist under `omp-server/data/frames/`?
- What does `HANDOFF_NEXT_SESSION.md` say (what it did, what's next)?

### Phase 3 — Tools / Reasoning / Msgs Count (90s)

**Frames used:** which of `text`/`reasoning`/`tool` streams appeared, and did tool frames carry real activity:

```bash
python3 -c "
import json
rows=[json.loads(l) for l in open('omp-server/data/frames/<SES_ID>.jsonl')]
from collections import Counter
tools=Counter((r.get('tool') or 'n/a') for r in rows if r.get('field')=='tool')
texts=[r.get('text','') for r in rows if r.get('field')=='text']
print('N tools:', sum(tools.values()), dict(tools.most_common(5)))
print('AI msgs:', len(texts), 'chars:', sum(len(t) for t in texts))"
```

**Reasoning:** check the agent logged reasoning steps (not just empty output):

```bash
wc -l subworkers/<AGENT>/workspace/20*/*.md 2>/dev/null
cat subworkers/<AGENT>/workspace/HANDOFF_NEXT_SESSION.md 2>/dev/null | head -40
# Empty output = failure (0 frames, no handoff update)
```

**Msgs sent:** count messages actually delivered:

```bash
# Telegram refund agents: check sent_log
cat /Users/vakandi/Documents/mcps_server/refund-telegram-agents/history/sent_log.jsonl 2>/dev/null | wc -l
# Discord (cve-hunter, deploymates-watch)
mcp-cli call discord-server-mcp discord_discover '{}' 2>&1 | head -20
```

### Phase 4 — Verdict (TABLE FORMAT, REQUIRED)

Build the report as a **table, one row per agent**. Always include a relative timestamp
(seconds/minutes/hours ago) computed against current local time (`date +%s` vs file mtime / run end).

```bash
NOW=$(date +%s)
for f in subworkers/<AGENT>/workspace/HANDOFF_NEXT_SESSION.md omp-server/data/state.json; do
  MT=$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null)
  [ -n "$MT" ] && python3 -c "
s=$NOW-$MT
u, v = ('sec',s) if s<60 else (('min',s//60) if s<3600 else (('h',s//3600) if s<86400 else ('d',s//86400)))
print('$f'.split('/')[-1], '->', v, u, 'ago')"
done
```

⚠️ TIMEZONE RULE (learned 2026-09-15 — never repeat this mistake):
- Server timestamps + scheduler times are **UTC**; Wael's local time is **UTC+1** (verify with `date; date -u`).
- NEVER report a raw timestamp as local time. ALWAYS convert: `local = UTC + 1h`, and ALWAYS render relative (`N min ago`) computed from epoch (`stat` mtime vs `date +%s`) — epoch math is timezone-proof.
- Sanity check: if `:5677` says `running: true` but your table claims "last activity 2 h ago", your timezone is wrong — recheck.

Output template (fill every cell, never leave blank — write `n/a` with reason):

| Agent | Live (:5677) | Last activity (relative) | Frames (streams) | N tools | AI msgs | Reasoning | Msgs sent (# + where) | Last thing done | Health |
|---|---|---|---|---|---|---|---|---|---|
| <AGENT> | 🟢 RUNNING / ⚪ idle | 12 min ago (handoff 13:48) | text+reasoning+tool, 40 frames | 12 tool frames (task×5, read×4, …) | 9 text msgs, ~2.1k chars | handoff ✅, verify md ✅ | 0 (Telegram MCP down) | 6 sites scored, shortlist filed | ✅/⚠️/❌ |

Column rules:
- **Live (:5677):** from Phase 1.5 `/status` — 🟢 RUNNING (started ~X ago) or ⚪ idle (next run at HH:MM).
- **Last activity:** newest of (frames mtime, handoff mtime, workspace file mtime) rendered as `N sec/min/h ago (HH:MM local)`.
- **Frames:** which streams + count. Text-only with 0 reasoning/tool on a research agent = suspicious.
- **N tools:** MANDATORY — count tool frames per run AND break down by tool name (top 5, e.g. `task×5, read×4, bash×2`). Source: frames file `tool` field. Zero tool frames on a working run = flag ⚠️.
- **AI msgs:** MANDATORY — count `text`-field frames (assistant messages) + total chars. Source: frames file `text` field. E.g. `9 msgs, ~2.1k chars`. Zero AI msgs = ❌ empty output.
- **Reasoning:** handoff present/absent + dated verify md present/absent. No frames + no handoff update = failed run.
- **Msgs:** count + destination (Telegram topic 45 / Discord #channel / none + why). `> 3 msgs/day` same topic = duplicate spam.
- **Last thing done:** one line from handoff/frames (sites scored, shortlist filed, verdict counts).
- **Health:** ✅ working / ⚠️ degraded (empty output, too few frames, delivery debt) / ❌ broken (no run, disabled, no output). Effective-disabled = won't run at all.

Then ONE final line: `**Verdict: <emoji> <one sentence>.**`

### Phase 5 — Report Only to Requester

- **Default:** report inline in the current session (chat response) — never broadcast to Discord/Telegram/WhatsApp.
- Only if the user explicitly says "post to Discord" / "notify channel" should you send elsewhere.
- Keep the report to 8-15 lines + a one-line verdict. Use `open` + `mcp-cli call macos-popup show_popup` only if the user asks for a popup.

## Rules

- Never modify the subworker's PROMPT.md or config during verification (read-only).
- Never call `POST /trigger/<AGENT>` or `POST /test/frames/<AGENT>` during verification (both start real/synthetic runs — verification is read-only).
- **Emoji gating (anti-lying rule, learned 2026-09-15):** 🟢 is FORBIDDEN unless Phase 1.6 shows
  live frames with real content OR a completed run shows success + handoff update.
  `running: true`, session existing, TopBar green = circumstantial, max ⏳/⚠️.
  When in doubt, downgrade: ❌ over 🟢. A wrong 🟢 destroys trust; a wrong ❌ just triggers a recheck.
- **No generalization (anti-lying rule, learned 2026-09-15):** one agent's frames prove something
  about THAT agent only. Never extend one agent's timeout/error/model to others — verify each agent
  with its own evidence (its own frames, its own session messages, its own model).
- **Label PROVEN vs HYPOTHESIS in every verdict:** each claim gets one tag.
  PROVEN = cited frames line / API output observed this session. HYPOTHESIS = inference, must say
  "hypothesis:" + what single check would confirm or kill it. An unlabeled claim is a lie.
- **Model check:** `zen-free/*` only. `opencode-zen/*` in any entry = instant run failure, flag ❌.
- If the latest frames file is empty or missing, say so — don't invent data.
- If Telethon session is locked, fall back to `sent_log.jsonl` via mcp-cli.
- Always resolve the agent name first — don't guess.
- Report only to the current session unless explicitly told to broadcast.

## File Paths

All repo-relative to `/Users/vakandi/Documents/omp-server`:

- Registry: `omp-server/config/subworkers.json`
- Enabled overrides: `omp-server/data/state.json` → `enabled` map
- Prompt: `subworkers/<AGENT>/PROMPT.md` (no personality file on omp)
- Workspace: `subworkers/<AGENT>/workspace/`
- Handoff: `subworkers/<AGENT>/workspace/HANDOFF_NEXT_SESSION.md`
- Frames: `omp-server/data/frames/<session_id>.jsonl` (fields `text|reasoning|tool`)
- Runs: `omp-server/data/runs.json`
- Auth: `omp-server/.env` → `ELIA_AUTH_TOKEN`
- Host pid/log: `omp-server/data/server.pid`, `omp-server/data/server.log`
- Container: `omp-server-omp-server-1` (port `5677:5677`)
- Telegram sent log: `/Users/vakandi/Documents/mcps_server/refund-telegram-agents/history/sent_log.jsonl`
- Telegram topic: chat `-1004469055773`, topic `45` (Agent IA)
- Discord: guild `1489242790444662885`, channels via `discord-server-mcp`
