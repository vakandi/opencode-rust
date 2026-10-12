# omp-server optimization plan — 10 concurrent sessions on the local plugin proxy, zero crashes

Target: 10 engine runs in parallel, every run behind the vendored local
plugin proxy (own upstream conf, native rotation), container steady under
its cap, host untouched, EliaTopBar livestream + subagents panels faithful.
Method: follow omp's own optimization logic (`docs/`), never fight it.

Companion: `../SERVER.md` (usage, scripts, TopBar contract). This doc is the
plan + measurements log. Update both when behavior changes.

## 0. Baseline (measured 2026-09-20, host `bun omp-server/server.ts`)

| Probe | Result |
|---|---|
| Idle RSS (server, 0 runs) | ~90 MiB, flat |
| 15-burst admission | 8×200 `triggered` + 7×202 `queued_429`, ≤24 ms each, zero silent drops |
| 3 concurrent workers (post-mutex) | 3/3 `completed` |
| 2 concurrent workers, distinct proxies | `82.23.230.143:7947` + `191.96.170.244:5933`, 11 frames each, no credential leak |
| Synthetic gate | text+reasoning+tool+banner over `/ws`, SSE replay clean EOF |
| Pre-mutex burst | 7/8 failed `Agent Main was replaced during session initialization` → fixed by `creationGate` (in-process) + worker processes (no shared singleton at all) |
| Container | `omp-server:elia-verify` builds exit 0, `mem_limit: 4g`, `HOST: 0.0.0.0` in-container |

## 1. Architecture (as built — worker-per-run)

```
TopBar ──HTTP/WS──▶ omp-server/server.ts ──▶ admitRun ──▶ Bun.spawn ──▶ omp-server/worker.ts × N
  (mac client)        (1 process)        (fsync first)   (env isolated)   (1 process per run)
                                                                        │ vendored proxy-local forward
                                                                        │ own EventBus/session
                                                                        ▼ frames (stdout NDJSON)
                                                              parent: store + /ws broadcast
```

- One process per run = one Main singleton, one crash domain. A worker death
  fails exactly one run (failed + banner + Discord), never the server.
- Parent never touches SDK state; workers never touch the registry/WS.
  The only shared files: `runs.json` (admission/finalize), `frames/*.jsonl`
  (append), prompt files (created/deleted per run).
- Proxy: ONE system — the vendored `extensions/proxy-local.ts` loaded
  exclusively per worker. In-process residential forward (`127.0.0.1:18898`),
  per-worker upstream conf (`data/proxy-<session>.conf`, seeded distinct),
  native rotation on errors + every 4 min, monitored log
  (`data/proxy-local.log`). No separate forwarding daemon, container, or
  parent-side rotation. Model traffic proxied; tool shells direct.

## 2. Cache systems (mirror omp, don't reinvent)

omp's rule (`docs/fs-scan-cache-architecture.md`, `docs/prewalk.md`,
`docs/memory.md`): cache at discovery boundaries, share down the tree,
invalidate on write. Ours:

| Cache | Status | Action |
|---|---|---|
| `statusPayload()` rebuilt per `/status` + every 30 s WS tick | recompute each time | P1: memoize 1 s or rebuild on broadcast events only (trigger/enable/frame-done). Same JSON every 30 s today = wasted cycles × clients. |
| `subworkers.json` re-read per `getEntry`? No — in-memory `entriesCache`, disk only on `/config/reload` | done, keep | — |
| `runs.json` full read+parse per admission | OK at 10-way (KBs) | P2: keep in-memory list + fsync on write (admission already sync-writes; reads should not re-parse). |
| Frame append: `appendFileSync` per delta | funciona but syscall per `message_update` | P1: per-session in-memory buffer, flush every 500 ms or 50 frames or on done — mirrors omp's persist-behind-memory (`docs/memory.md`). Crash window ≤500 ms of deltas; run record stays fsync-first so no phantom runs. |
| Proxy LRU read+write JSON per pick | OK at this rate | P2: in-memory map, fsync every 30 s + on release. |
| omp-side (inherited free): workspace tree 5 s scan w/ AGENTS.md index, model-registry credential caches, pi-walker scan cache, session SQLite | inherited | never duplicate: no second workspace walk, no own model DB. Pass `cwd` through and let the SDK own it. |
| `next_run` recompute per scheduler tick | cheap date math | keep; tick stays 30 s. |

## 3. CPU / RAM budget (container 4 Go, host capable)

- Measure first: sample worker RSS at start/idle/done for 3 prompt sizes
  (greeting / file task / web task) → `docs/` table here. Expected shape from
  omp's design: workers are the cost center (model stream + tools), parent is
  a thin router (~90 MiB). Size `MAX_CONCURRENT_RUNS` from data, not hope:
  `cap = floor((4096 − 512 parent/reserve) / p95_worker_RSS)`, clamp 4..10.
  Live `.env` ships 100 — do NOT inherit blindly; our default stays 8 until
  measured, target 10 only if p95 fits.
- Follow omp's cheap-process rules (`docs/natives-*.md`, `docs/stream.md`):
  no fork/exec on hot paths (we `Bun.spawn` once per run — correct; never per
  frame), in-process grep/glob via SDK tools (never shell out per delta),
  backpressure on stdout pipes (drain or pause, never unbounded buffer).
- Watchdog stays the OOM backstop: `MAX_RUN_S` kills runaway wall-clock;
  add frame-rate guard (P1): >X frames/s sustained → mark failed (looping
  tool storm) before RAM follows. Container `mem_limit` is the only hard cap —
  no in-app RSS gates (deleted pattern stays deleted).
- Bun flags: keep default GC; if p95 shows sawtooth pressure, set
  `BUN_OPTIONS=--smol` on workers only (parent keeps full runtime for WS).
- Host rule: NOTHING runs on host in production except EliaTopBar (a client,
  not a server). Host bun today = testing only. Cutover = compose up.

## 4. Code quality — write like omp, not around it

Checklist for every omp-server/ wrapper change (from `docs/`, `AGENTS.md`):

- Reuse before invent: central utils (`@oh-my-pi/pi-utils`, `src/utils/`,
  `@oh-my-pi/pi-tui`) — duplicate helpers are bugs. Shell via `` $ `` with
  `.quiet().nothrow().cwd()`; never shell out for what a Bun API covers.
- No `any`, no `ReturnType<>`, no inline imports (only the two documented
  lazy-load/pre-warm sites), `#private` fields, `Promise.withResolvers()`.
- Prompts never in code (`.md` + Handlebars). Barrels `export *`.
- `node:*` namespaced; Bun prefs (`Bun.file/write/sleep`, `bun:sqlite`).
- Logging: `console.*` ONLY in standalone/server paths (ours is a server —
  allowed), structured `[area] key=value` lines, NEVER tokens/creds/proxy
  userinfo (use `maskProxy`, mirror `proxyLogTarget` in
  `packages/ai/src/utils/proxy.ts`).
- Proxy env discipline (mirrors `packages/ai/src/utils/proxy.ts`):
  per-provider `PI_PROXY_*` is memoized per process — NEVER set it;
  per-process `HTTPS_PROXY` is read fresh per request — the ONLY knob workers
  use. Local/metadata hosts bypass automatically — never override.
- One invariant per change; reproduce → fix → keep the repro (our repros are
  `bun -e` scripts + curl gates, not test files — elia has no framework by
  design).

## 5. Crash-proofing the 10-way (checklist)

- [ ] Worker spawn failure / instant exit (no `done` line) → failed + banner + Discord (already).
- [ ] `stdout` pipe backpressure: async line iterator with highWaterMark; on overflow pause worker (SIGSTOP/SIGCONT) — P1.
- [ ] Zombie reaping: always consume `proc.exited` (even after `done`); `unref` rotation timers (already).
- [ ] Stdin write after exit guarded (already) — plus `proc.kill()` on watchdog expiry so a stalled worker can't outlive its run record (P0: watchdog currently marks failed but leaves the child running).
- [ ] Prompt files: created with `0600`, deleted on finalize (already); stale sweep on boot (P2: delete `data/prompts/*` older than 1 h at `startServer`).
- [ ] Restart storm: scheduler `triggerFn` admission honors the cap → bursts degrade to `queued_429`, never fork-bomb (already proven 7/7).
- [ ] Discord debounce per (name, kind) 300 s (already) — burst failures won't spam.
- [ ] ` Bun.spawn` `cwd: repoRoot` (already) — workers resolve `packages/*` identically in container and host.
- [ ] Container health: `HEALTHCHECK /health` (already); compose `restart: unless-stopped` (already).

## 6. EliaTopBar parity — livestream + subagents panels (REQUIRED)

TopBar is the client of record (`SubworkerManager.swift` wins over any model).
FastAPI must parse/produce EXACTLY what the panels need — same info density as
omp TUI Agent Hub (`docs/agent-hub.md`: roster, activity, usage, live
transcript, steer, kill).

| Panel needs | FastAPI source | Shape contract (frozen) |
|---|---|---|
| Agents roster (name, on/off, LIVE badge) | `GET /status` | `subworkers[{name,enabled,running:bool,schedule_type,schedule,model,variant,next_run}]` — `running` from registry, never schedule |
| Run popup livestream (3 streams) | `/ws` `run_log` | `{event,name,text,field}` with `field ∈ text\|reasoning\|tool`; unknown → `"text"` |
| Banners (errors/delays) | `/ws` `run_banner` | `{event,name,banner:{kind,error?,delaySeconds?}}` |
| Start/finish flips | `/ws` `subworker_started/completed/success/cancelled/error` | `{event,name}` (+`status`,`session_id` on completed) |
| Server icon | `initial_status`/`status_update` + `GET /server/health` | `opencode_health:"healthy"`, `health_status:"healthy"` literal |
| Session list (transcripts) | `GET /sessions/{name}/list` | `sessions[{session_id,title,agent,time_created}]`, live run prepended `title:"▶ Running"` |
| Transcript view | `GET /sessions/{name}?limit=` | `{session_id,messages[{info:{role,time_created},parts[{type,text,output}]}]}` rebuilt from frame JSONL |
| Steer (continue) | `POST /sessions/{name}/{id}/continue` | same session id, `subworker_started` rebroadcast → LIVE |
| Default agent selector | `GET+POST /main-agent` | `{name}` regex `^[a-zA-Z0-9_-]{1,64}$` |
| **Kill a run (GAP)** | none today | P0: add `POST /sessions/{name}/{id}/stop` → `proc.kill()` + failed record + `subworker_cancelled` broadcast (Hub can kill stuck workers; TopBar handles the event already) |

- Main-agent parsing: `omp-server/data/main-agent.json` (`{"name":"elia"}` default);
  scheduler workspace mapping honors it (see `effectiveWorkspace`).
- Parity test (no framework needed): add `omp-server/contractCheck.ts` (P1) asserting
  every shape above against a synthetic run (status/list/sessions/SSE/WS) —
  runs in CI in seconds, fails on any field rename. Until then: the manual
  gate in `../SERVER.md` (trigger → popup shows 3 streams → TopBar counts).
- omp-TUI equivalence target: roster ≡ `status`, activity ≡ `run_log` rates,
  transcript ≡ session messages, steer ≡ continue/stop, usage ≡ (future:
  expose per-run frame counts + duration in `GET /status/{name}` — P2).

## 7. Execution order

1. P0: watchdog kills worker child on stall (`proc.kill()`); `POST …/stop` + `subworker_cancelled`.
2. P0: measure worker RSS (3 prompt sizes) → set cap for 10-way → soak 10 real concurrent, distinct-proxy proof in log.
3. P1: frame buffer+flush; status memo; `contractCheck.ts`; stdout backpressure; stale prompt sweep.
4. P2: runs/LRU in-memory + periodic fsync; usage stats in status detail; `--smol` workers if needed.
5. Cutover: TopBar URL switch agent-by-agent; live `:5656` stays until last migration.

## 8. References

- `../SERVER.md` — usage, scripts (`install/start/stop`), manual gates, TopBar contract summary (points here for the plan).
- Upstream logic followed: `docs/fs-scan-cache-architecture.md`, `docs/memory.md`, `docs/prewalk.md`, `docs/stream.md`, `docs/agent-hub.md`, `docs/sdk.md`, `docs/natives-architecture.md`, `docs/provider-quirks.md`, `packages/ai/src/utils/proxy.ts` (env discipline), `packages/catalog/src/compat/rules/` (never hardcode models).
- Live parity reference (read-only): `~/EliaAI/subworkers/server/` — routes, runner proxy pattern, tunnel manager, cloudflared sidecar.
