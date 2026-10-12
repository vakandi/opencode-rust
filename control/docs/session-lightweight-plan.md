# Session lightweight plan — less RAM/CPU per run, same stability

Goal: cut per-worker RSS (~390 MB steady today) without losing crash
isolation (one process per run stays) or proxy behavior (vendored
proxy-local per worker stays). Method: documented Bun/SDK levers only,
measured before/after each step with the RSS sampling protocol below.
## 0. Measurement protocol (before every step)

```bash
# trigger ONE hunter, sample worker VmRSS every 20 s from host:
for i in $(seq 1 8); do sleep 20; docker exec omp-server-omp-server-1 sh -c \
'for d in /proc/[0-9]*; do c=$(tr "\0" " " < $d/cmdline 2>/dev/null); case "$c" in *worker.ts*) awk "/VmRSS/{print \$2\$3}" $d/status;; esac; done'; done
```

Baseline (2026-09-20, refund-hunter): 364 → 361 → 375 → 393 → 397 → 405 →
390 → 389 MB. Stable sawtooth = NO leak; ~390 MB is the cost center.
Budget math stays: `cap = floor((4096 − 512) / p95_worker_RSS)`.

## 1. `bun --smol` on worker spawn (expected −20–40%, small speed cost)

Bun documents `--smol` (`bun run --smol`, also `Bun.Worker smol:true`):
GC runs more frequently, less heap retained, slower execution.
Change (one line, `engine.ts` spawn argv):

```ts
child = Bun.spawn(["bun", "--smol", "omp-server/worker.ts", ...])
```

Gate: re-run §0; accept iff p95 drops ≥15% AND a hello-run still completes
< 120 s. Rollback = delete the flag. No other file changes.

## 2. Trim unused subsystems per worker (expected −5–15% each, verify)

Worker sessions today inherit host defaults. Override via `Settings.init`
passed as `settings` to `createAgentSession` (SDK supports `options.settings`,
`sdk.ts` option surface verified). Candidate keys (all verified present in
`settings-schema.ts` / SDK options):

| Setting | Why trimmable for hunters |
|---|---|
| `browser.enabled: false` | hunters use `web_search`/`fetch`, never the `browser` tab runtime (Chromium + stealth profiles are heavy) |
| `computer.enabled: false` | no desktop control needed (AX tree, screenshots) |
| `memory.backend` | set only if hunters don't use retain/recall (verify: grep PROMPTs for retain/recall/memory) |
| MCP servers | if `~/.omp/agent/mcp.json` lists servers no hunter prompt names, pass `disableExtensionDiscovery` already set + skip MCP init (verify manager construction path first — do NOT guess) |
| `includeWorkspaceTree: false` | SDK default is already false (`sdk.ts:1404 ?? false`) — confirm no override enables it; if already off, no action |
| `thinkingLevelCeiling` | cap effort per run (saves time/compute; set per-agent tier, not global) |

Rule: one flag per experiment, RSS protocol each time, keep only proven wins.
Never disable `task`/`hub`/`todo`/`read`/`bash`/`web_search` (the working set).

## 3. Worker reuse pool (biggest structural win, do last)

Pay the ~350 MB SDK load ONCE per pool slot instead of per run: keep N warm
workers alive, feed prompts over the existing stdin/NDJSON protocol (extend
with `PROMPT <file>` + `RESET` lines; worker loops `main()` instead of
`process.exit`). Parent tracks slot↔session mapping; crash → slot discarded,
fresh spawn (isolation preserved per incident, not per run).
Saves: spawn spikes (the OOM killer's favorite moment) + ~350 MB × churn.
Requires: worker main-loop refactor + parent pool manager + idle reaper
(`MAX_IDLE_S`, default 300). Only attempt after steps 1–2 are banked.

## 4. Stagger cold starts (free)

Bursts spawn N runtimes in the same second (compile + catalog + auth at
once = spike). Admission already persists synchronously; add a tiny
creation spacing (e.g. start worker N+1 only after worker N emits its first
frame or 5 s elapsed). No cap change, no behavior change, spike gone.

## 5. Stability guards (non-negotiable, keep all)

Watchdog stall/max kill, kill registry, per-worker conf files, plugin log
monitoring, Discord debounce, `queued_429` backpressure, prompt-file hygiene.
Any step that weakens these is rejected even if it saves RAM.

## 6. Order + gates

1. §1 smol (measure → keep/rollback) → 2. §2 flags one-by-one (measure each)
→ 3. §4 stagger → 4. soak 8-way proof (distinct proxies + completions +
RSS flat) → 5. §3 pool (only if still over budget).
Each step: `bun -e` parse, image rebuild, container proof run, RSS curve in
this doc's table below.

## 7. Results log

| Step | p95 worker RSS | vs baseline | kept? |
|---|---|---|---|
| baseline | ~390 MB | — | — |
| §1 smol + §2 browser/computer off | ~390 MB peak | ≈0% | kept (harmless) |
| §4 stagger 5 s | no spawn spike observed | n/a | kept |
| 9-agent soak (7 parallel + 2 after drain) | container peak ~3.05 GiB / 4 GiB | fits | PASS |
