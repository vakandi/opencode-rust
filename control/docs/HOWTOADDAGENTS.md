# HOWTOADDAGENTS.md — add a custom agent A to Z

No frontmatter. No SDK changes. An agent is **3 things**: a registry entry, a
`PROMPT.md`, a workspace dir. Total time: ~5 minutes.

## 1. Create the workspace

```bash
AGENT=my-agent
mkdir -p subworkers/$AGENT/workspace
```

`subworkers/$AGENT/` is the agent home. `workspace/` is the session cwd
(created automatically if missing — see `effectiveWorkspace` in
`omp-server/auth.ts`, which resolves repo homes first, live tree fallback).

## 2. Write `subworkers/$AGENT/PROMPT.md`

First line: `# <Name>`. Second line: the OMP HOME block (exact):

```md
> OMP HOME: /Users/vakandi/Documents/omp-server/subworkers/<name>/ — all reads/writes stay inside this home (plus its workspace/ subdir). Never use /Users/vakandi/EliaAI/ paths in new work; old outputs keep working via the live bind-mount.
```

Then: mission, workflow, tools, limits, verification — same anatomy as the
25 shipped prompts (copy `test-delegator/PROMPT.md`, rewritten for omp:
`task`/`subtask`/`hub`/`todo`, NEVER opencode `team_*`). Rules that matter:

- Prompts are injected VERBATIM as the run prompt (no frontmatter parsed,
  no templating) — what you write is what the model receives.
- Constrain file I/O to `workspace/` (the engine sets cwd there).
- End every run path with a `HANDOFF_NEXT_SESSION.md` update + dated
  `workspace/YYYY-MM-DD/verify_*.md` report (TopBar + verifiers read these).

## 3. Register it (two places, same shape)

**a) `omp-server/config/subworkers.json`** — append:

```json
{
  "name": "<name>",
  "enabled": false,
  "agent_id": "<name>",
  "model": "opencode-zen/muse-spark-1.3-contributor-free",
  "variant": "low",
  "schedule": { "type": "interval", "hours": [9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23], "minute": 0, "days": null },
  "workspace": "/Users/vakandi/Documents/omp-server/subworkers/<name>",
  "timeout_minutes": 30,
  "max_retries": 2
}
```

Field notes: `name` = routing key (`[a-z0-9-]`); `enabled: false` until
proven (enable via TopBar or `POST /enable/{name}`); `agent_id` is
display/routing metadata (sessions list, status detail) — the engine does
NOT load personalities by it; `model` must be a live `opencode-zen/<id>`
(see `docs/models-dynamic-plan.md` — the dropdown only offers live ids);
`variant` = thinking effort (`low/medium/high/max`); `schedule` one of
`interval|every|cron`.

**b) Hot-load without restart:** `POST /config/reload` re-reads the file
into the scheduler. Verify: `GET /status/<name>` → 200 with your fields.

## 4. Models for the new agent

No per-agent model files. The TopBar dropdown (`GET /models`, dynamic Zen
roster with NEW badges) offers only live `opencode-zen/*` ids; picking one
sends `PUT /status/{name} {model, variant}`. Default at rest: the entry's
`model`/`variant`. Nothing else to configure.

## 5. Verify (host, no TopBar needed)

```bash
T=$(grep ELIA_AUTH_TOKEN omp-server/.env | cut -d= -f2)
curl -s -H "Authorization: Bearer $T" http://127.0.0.1:5677/status/<name> | head -c 300; echo
curl -s -X POST -H "Authorization: Bearer $T" -H 'content-type: application/json' \
  -d '{"prompt":"Reply with exactly: AGENT OK"}' http://127.0.0.1:5677/trigger/<name>
# watch: tail -f omp-server/data/frames/<session_id>.jsonl
# TopBar: open the agent popup → livestream + subagents + todos appear
```

## 6. Ship it

`subworkers/<name>/` (minus `workspace/*.log`, media >25M — see root
`.gitignore`) commits with the repo. Runtime state (`omp-server/data/`)
never commits. Enable it (`POST /enable/{name}`) only after the verify run
completes with real model tokens.
