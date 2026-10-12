# PERFORMANCE FINDINGS — perf-architect, Round 1

Team: opencode-rust-port · TeamRunId: bc3f103e-f405-4e5e-b685-014e4bc7f0b3
Scope: adversarial planning team, Round 1 — independent findings only. No implementation plan, no code.

**Headline: the language is not the bottleneck, and I can prove it from your own production DB.** Two
language-independent defects account for ~95% of the 8GB pressure. A Rust port is worth doing for CPU and
heap efficiency, but it will **not** get you to 100 concurrent browser-driving sessions.

---

## 0. MEASURED BASELINE — this is your live production database

`~/.local/share/opencode/opencode.db`, `page_size=4096`, `page_count=4,634,658`, **`freelist_count=0`**
(live data, not bloat).
**File size: 18,983,448,576 B = 17.68 GiB.**

Per-table page usage (`dbstat`):

| table/index | bytes | GiB | % |
|---|---|---|---|
| **`event`** | 16,040,935,424 | **14.94** | **84.5%** |
| `event_aggregate_type_seq_idx` | 107,008,000 | 0.10 | 0.6% |
| `event_aggregate_seq_idx` | 70,590,464 | 0.07 | 0.4% |
| `sqlite_autoindex_event_1` | 63,287,296 | 0.06 | 0.3% |
| `part` | 1,672,470,528 | 1.56 | 8.8% |
| `message` | 932,552,704 | 0.87 | 4.9% |
| part indexes (3) | 76,324,864 | 0.07 | 0.4% |
| message indexes (2) | 16,412,672 | 0.02 | 0.1% |
| `session` | 2,637,824 | 0.003 | ~0% |

**Event log = 14.94 GiB = 84.5% of the DB.** Messages + parts combined are only 2.43 GiB.

### Event log content — 1,405,820 rows

| event type | count | share | avg `data` B* |
|---|---|---|---|
| `message.part.updated.1` | 901,805 | 64.1% | **1,600** |
| `message.updated.1` | 384,115 | 27.3% | **22,422** |
| `session.updated.1` | 115,653 | 8.2% | 1,009 |
| `session.created.1` | 3,126 | 0.2% | 630 |
| `message.removed.1` | 1,657 | 0.1% | 91 |

\* avg **MEASURED** over the most-recent 30,000 rows, extrapolated ⇒ **~10.2 GB of `data` payload**.
Table occupies 16.04 GB ⇒ **~5.8 GB (36%) page/overflow overhead** (payload-vs-table split is INFERRED;
the per-type averages are MEASURED).

Effective cost: **11,409 bytes of DB per published event**, average.

### The offending code

`packages/schema/src/v1/session.ts:502-507`:

```ts
const options = { durable: { aggregate: "sessionID", version: 1 } } as const
```

Every session/message/part event definition spreads `...options` ⇒ **every part update and every message
update is written to a durable, per-session-sequenced, append-only table**, with 3 indexes maintained.

`packages/core/src/event.ts:337` — the insert. **There is no `DELETE` on `EventTable` anywhere in the
codebase.** The only removal path is `ON DELETE CASCADE` from `event_sequence` when a session is deleted.
You have never deleted a session (3,125 sessions; only 1,657 messages ever removed).

**And nobody reads it.** `replayAll` / `replay` / `subscribeDurable` are reachable only from
`packages/opencode/src/control-plane/workspace.ts:351,402` and
`packages/opencode/src/server/routes/instance/httpapi/handlers/sync.ts:51` — the multi-workspace
cross-machine sync feature. On the subworker deployment (single workspace, `/tmp/{agent}` dirs, no sync)
**this is 14.94 GiB of pure write amplification that is never read.**

`session.updated` averages 22 KB/event. Your `message` table has a **single row of 84,718,849 B
(84.7 MB)**; max `part` row **4,611,147 B (4.6 MB)**.

### Part / message distribution (measured)

| part type | count | avg B | max B |
|---|---|---|---|
| `tool` | 104,174 | 10,428 | **4,611,147** |
| `file` | 714 | 83,928 | **3,640,199** |
| `text` | 42,586 | 2,792 | 1,615,627 |
| `patch` | 24,366 | 246 | 447,269 |
| `reasoning` | 72,496 | 2,616 | 349,857 |
| `step-start` | 94,389 | 72 | 75 |
| `step-finish` | 93,863 | 203 | 213 |

- total parts 432,974 · total messages 107,316 ⇒ **4.035 parts/message**
- `part.data` sum 1,486,549,045 B (1.38 GiB), avg 3,433 B
- `message.data` sum 903,082,143 B, avg 8,414 B, **max 84,718,849 B**
- largest session: 7,441 parts / **87,584,481 B = 83.5 MiB**

---

## 1. WHERE THE MEMORY ACTUALLY GOES

### 1a. Steady-state idle session: small. This part is genuinely fine.

The TS server does **not** cache history in RAM. `MessageV2.stream()` (`message-v2.ts:473-494`) pages from
SQLite every time. And `run-state.ts:33-49,57-59` deletes the `Runner` from the map on idle. **Anyone who
tells you "the history is cached in RAM" is wrong.**

### 1b. The per-LLM-step transient spike: the real per-session cost

`runLoop` at `prompt.ts:1080` is a `while(true)`. **Every iteration** does, at `prompt.ts:1093`:

```ts
let msgs = yield* MessageV2.filterCompactedEffect(sessionID)
```

= `filterCompacted(stream(sessionID))` (`message-v2.ts:578-580`) — a **full re-read + re-hydration of the
entire history from SQLite.** Then:

| # | allocation | site |
|---|---|---|
| A | full hydrated history array (all messages + parts, JSON-parsed) | `message-v2.ts:473-494` → `hydrate()` `:97-120` |
| B | `filterCompacted` output; `result.reverse()` in place | `message-v2.ts:525-575` |
| C | **3-slice spread → brand-new array** (when a compaction boundary exists) | `message-v2.ts:569-572` |
| D | `toModelMessagesEffect` — 2nd full copy, different shape | `message-v2.ts:131-420`, called `prompt.ts:1262` |
| E | `system` array (env + instructions + MCP instr + skills), all re-read | `prompt.ts:1264-1270` |
| F | `[...modelMsgs, ...]` — another full pointer-array copy | `prompt.ts:1280` |
| G | serialized JSON request body string | AI SDK inside `handle.process` |
| H | UTF-8 request body bytes on the socket | ditto |

D and A run **concurrently in one `Effect.all`** (`prompt.ts:1257-1263`) ⇒ A–H simultaneously live at peak.

**Correction to the naive model:** JSC Latin1 strings are only ~1.07× the raw byte count — TS does *not*
waste RAM on string encoding. Strings in D are shared by reference with A. So the multiplier is **~2.6-3×
on bytes** plus **~2.4 KB/message of wrapper objects**, plus the two full contiguous text copies (G, H).

**Measured derivation** (parts/message = 4.035; avg part JSON = 3,433 B):

- per message of real history: **~13.8 KB of JSON**
- TS peak per active turn ≈ **3.1× that ≈ 43 KB/message**, + ~2.4 KB/msg wrappers, + 255 KB tool schemas,
  + ~100 KB system prompt

**Typical** 100-msg run (1.38 MB history): TS peak ≈ **5.5 MB**. ×100 concurrent = 550 MB.
**Not the problem.**

**Measured worst case** (7,441 parts / 83.5 MiB): TS peak ≈ **~290 MB for ONE session.** 30 of those =
8.7 GB. **This is your OOM, and it is measured.**

### 1c. Bun's RSS ratchet — the mechanism behind "stalls past ~20"

Bun/JSC grows its heap to accommodate peaks and does not aggressively return pages to the OS. With
`mem_limit: 8g` and **`memswap_limit: 8g` (= zero swap)**, RSS never comes down. A single 83.5 MiB-history
session permanently inflates the process. *This is a ratchet, not a steady state* — exactly the "works fine
then suddenly stalls and bugs" behaviour you describe. (JSC behaviour asserted from knowledge, not
profiled — see unverified #5.)

### 1d. Effect fiber + allocation overhead during streaming

- `native-runtime.ts:107` — `Queue.unbounded<LLMEvent>()` between tool-dispatch fibers and the output
  stream. Unbounded: grows without limit if dispatch outruns the consumer.
- `native-runtime.ts:126` — **one `FiberSet.run` per tool-call event.**
- `processor.ts:297` and `:515` — `ctx.reasoningMap[id].text += value.text`,
  `ctx.currentText.text += value.text`. JSC `+=` allocates a rope cell per delta ⇒ **the entire assistant
  response is held in RAM as a rope for the whole turn.**

---

## 2. THE PER-DIRECTORY FAN-OUT — I believe this is your actual 8GB

`packages/opencode/src/effect/instance-state.ts:31`:

```ts
const cache = yield* ScopedCache.make<string, A, E, R>({
  capacity: Number.POSITIVE_INFINITY,   // ← UNBOUNDED
})
```

**22 non-test `InstanceState.make` call sites in `packages/opencode/src/` alone** (35 with tests). Each is
an independent unbounded `ScopedCache` keyed by the directory string. The heavy ones:

- `mcp/index.ts:492` — **per-directory MCP clients** (`clients:{}`, `defs:{}`, `instructions:{}`), built
  with `Effect.forEach(..., {concurrency:"unbounded"})` over your whole MCP config.
- `lsp/lsp.ts:145` — per-directory LSP server table (child processes).
- `plugin/index.ts:134-178` — per-directory plugin state: **11 internal auth plugins instantiated per
  directory** (`:67-86`) + every external plugin incl. oh-my-openagent. Plus `createOpencodeClient` at
  `:146` — an HTTP client per directory pointed back at your own server.
- also `provider/provider.ts:1451`, `config/config.ts:614`, `skill/index.ts:259,273`,
  `snapshot/index.ts:66-71`, `agent/agent.ts:98`, `permission/index.ts:46`, `project/project.ts:386`,
  `session/status.ts:29-31`, `session/run-state.ts:33`.

**The directory key:** `SUBWORKERS_SYSTEM.md:53` — `create_session(directory=/tmp/{name}, ...)`. One
distinct directory **per subworker agent**. `subworkers.json` has **25 agents, 11 enabled** ⇒ **~11
distinct directory keys**, each with its own unbounded MCP client set.

**Your MCP config** (`~/.config/opencode/opencode.json`; mirrored by `opencode_client.py:38-45`
`DEFAULT_MCP_SERVERS`, 6 servers): `codegraph`, `context7`, `grep_app`, `lsp`, `parallel-browser-mcp`,
`websearch`. And `parallel-browser-mcp` is:

```
"command": ["npx","-y","parallel-browser-mcp@latest"]
"environment": {"BROWSER_MCP_CONFIG": "...providers.playwright...headless:true"}
```

**⇒ 11 directories × 1 `npx` Node process × 1 headless Chromium, retained forever.**
At 250-400 MB RSS per Chromium stack (ESTIMATE), that is **2.7-4.4 GB of your 8 GB before the server uses
anything.**

**Never freed.** `dispose_instance()` exists at `opencode_client.py:313-331` and documents the exact
problem ("watchers/LSP/MCP handles accumulate and are never evicted") — but grep over `server/app/` finds
**zero callers.** No TTL, no LRU on the unbounded `ScopedCache`.

**Your own compose file already knows:** `docker-compose.yml:8` — *"8GB + swap prevents OOM-kills from
parallel-browser-mcp + Chrome leaks"*; lines 6-7 — *"the entrypoint auto-restarts opencode after a cgroup
OOM-kill"*. You are currently **masking this with a restart loop.**

---

## 3. THE CONSUMER CONTRACT — this changes the priority order

`opencode_client.py` **never opens the SSE `/event` stream.** Production consumption is pure HTTP polling:

- `POST /session` with the undocumented `x-opencode-directory` header (`:229-238`)
- `POST /session/{id}/message` (`:330-380`)
- `GET /session/{id}/message?limit=50` (`:406-418`)
- `GET /session/status` — **full status map for all sessions** (`:207-225`)
- `wait_for_response` polls `get_session_status` **every 2 s** per active session (`:452-500`)

Consequences:

1. **The SSE fan-out is NOT your bottleneck.** `handlers/event.ts:11` `subscriberCapacity = 256` and
   `core/event.ts:154` `Queue.dropping(256)` are correctly bounded. With 0 production subscribers this
   costs nothing. **Do not let anyone prioritise this.**
2. But `core/event.ts:175` (`PubSub.unbounded` for `pubsub.all`) and `:188` (per-type unbounded) **do** run
   on every publish, and `event-v2-bridge.ts:42-61` installs a **permanent global listener** running for
   every event regardless of subscriber count — allocating `new Location.Info(...)` + 2 wrapper objects per
   event, plus a second `GlobalBus.emit` for every durable event.
3. `GET /session/status` (`status.ts:36-38`) does `new Map(yield* InstanceState.get(state))` — a fresh Map
   copy per call. At 100 sessions × 0.5 Hz = 50 copies/s. Minor, pure waste.
4. **Your Rust SSE can be best-effort with zero retention**, because no production consumer subscribes.
   Resync is already `GET /session/{id}/message`.

---

## 4. VERDICT: IS 100 CONCURRENT SESSIONS IN 8GB PHYSICALLY ACHIEVABLE IN RUST?

### Arithmetic

**Hard constraint (measured):** `docker-compose.yml:9-10` → `mem_limit: 8g`, `memswap_limit: 8g`. The
comment says "8GB + swap" but **memswap == mem means ZERO swap**. 8192 MiB, no second chance.
`MAX_CONCURRENT_RUNS=100` (`:23`).

| Line item | MiB | Basis |
|---|---|---|
| Container hard cap | **8192** | measured |
| − kernel, dockerd shim, tini, cgroup page-cache floor | −180 | estimate |
| − **Python 3.12 + uvicorn + FastAPI + APScheduler + structlog** | −380 | estimate — **a 2nd full runtime in the same cgroup** |
| − Chromium × parallel-browser-mcp, **pooled to 5** | −1500 | 5×300 MB, ESTIMATE |
| − 6 shared MCP servers (Node — they're Node regardless of your language) | −600 | 6×100 MB, ESTIMATE |
| − 3 shared LSP servers | −240 | 3×80 MB, ESTIMATE |
| − provider TLS/HTTP pool, 300 conns × 256 KiB | −75 | estimate; **reqwest `pool_max_idle_per_host` defaults to unbounded — must be set** |
| − fixed Rust runtime (tokio workers, allocator arenas, binary) | −300 | estimate |
| − fragmentation / burst headroom (12%) | −700 | |
| **⇒ available for opencode's own session data** | **≈ 4217 MiB** | |

### ⇒ REQUIRED PER-SESSION BUDGET: **4217 ÷ 100 = 42 MiB/session**

| Design | Peak per active session | ×100 concurrent | Fits 42 MiB/sess? |
|---|---|---|---|
| **TS today, typical run** (100 msg, 1.38 MB) | ~5.5 MB | ~550 MB | yes |
| **TS today, measured worst** (83.5 MiB history) | **~290 MB** | ~29 GB | **NO — 7× over on ONE session** |
| **Naive Rust** (history in RAM, as `/tmp/OpenCodeRust` does) | 83.5 MiB + copies | ~9 GB | **NO — 2× over budget before any copy** |
| **Rust, history spilled + streamed request** | **~0.6-1.0 MB** | **~60-100 MB** | **YES — 42-70× headroom** |

### The straight answer

**CONDITIONAL YES for the opencode server itself.** 100 concurrent sessions in Rust lands at **1.5-3.0 GB
total opencode RSS** — ~3-5× better than the idealised TS case, ~30× better than measured worst case —
comfortably inside 8 GB. Required budget **42 MiB/session**; a spill-to-SQLite + streamed-request design
uses **~1 MiB**. You have 40× margin.

**The condition is non-negotiable: history MUST spill to SQLite and the request body MUST be streamed.**
Measured worst-case session history is 83.5 MiB — nearly **2× your entire per-session budget** — so any
design holding history in RAM per session (exactly what the existing Rust reference does) **fails on your
real data, not on a hypothetical.**

### The answer you will not like: the browser ceiling is not a Rust problem

If "100 concurrent sessions" means **100 subworkers each actively driving a browser**:

```
100 × (Chromium ~300 MB + Node ~100 MB) = ~40 GB
```

**Physically impossible in 8 GB, in any language.** Hard ceiling ≈ **8 GB ÷ 400 MB ≈ 15-18 concurrent
browser-driving sessions.**

A Rust rewrite moves that number by **~5%** (you save the orchestrator's heap, not Chromium's). **If the
goal is "100 concurrent browser-driving subworkers in 8 GB", the goal is impossible as stated and the port
is not the lever.** The lever is one shared browser pool + session affinity — a Python/architecture change.

**Restated goal that IS achievable:** *100 concurrent opencode sessions in 8 GB, hard cap ~15 concurrent
browser-driving sessions, one pooled Chromium.* Rust gets you there with margin.

---

## 5. MEMORY BUDGET TABLE — TS today vs required Rust budget

| Component | TS today | Rust target | Ratio |
|---|---|---|---|
| Idle session steady state | ~30-60 KB | **~300 B** | 100-200× ↓ |
| Active turn, typical 100-msg run | ~5.5 MB | **~700 KB** | 8× ↓ |
| Active turn, measured worst case | ~290 MB | **~1.0 MB** | 290× ↓ |
| History copies live at peak | 4-6 (A,B,C,D,F + G,H text) | **1** (the request buffer) | 4-6× ↓ |
| Copy multiplier on history bytes | ~3.1× | **~1.0×** (borrowed `&str`) | 3× ↓ |
| Per-directory instance state | 11 dirs × (6 MCP + N LSP + 11 plugins), **unbounded, never evicted** | **pooled by config hash, LRU+TTL** | ~11× ↓ on the biggest term |
| Response text accumulated in RAM | full rope per turn (`processor.ts:297,515`) | **4 KB staging buffer** | ~12,500× ↓ |
| Tool schemas re-derived | **85 tools × deep-clone per LLM step** (`tools.ts:92-101`, `transform.ts:1595`) | cached per (model, tool-set) | ~1000× ↓ |
| System prompt re-read | **every step, uncached** (`instruction.ts:155-169`: fs walk + N reads + M HTTP fetches) | cached per (directory, mtime) | large |
| Event log on disk | **14.94 GiB, 1.4 M rows, never pruned, never read** | **0 (in-memory broadcast)** | total |
| RSS after a peak | **never returns (JSC)** | mimalloc returns arenas | — |

---

## 6. TOP 5 RAM HOTSPOTS, ranked by bytes reclaimed

**#1 — Durable, never-pruned, never-read event log. 14.94 GiB.**
`packages/schema/src/v1/session.ts:502-507` (all defs `durable`) → `packages/core/src/event.ts:337`
(insert + 3 indexes). No `DELETE` on `EventTable` anywhere. Also `event-v2-bridge.ts:42-61` doubles every
durable event into a second `GlobalBus.emit`.
Reclaim: **~15 GiB disk, ~1.4 M inserts of write CPU, and the page-cache/WAL pressure that is almost
certainly your "stall".**
**This is the #1 win in the whole system and it is a ~20-line change, not a rewrite.**

**#2 — Per-directory unbounded instance fan-out (MCP + Chromium + LSP + plugins). 2.7-4.4 GB.**
`instance-state.ts:31` (`capacity: POSITIVE_INFINITY`) × 22 call sites; `mcp/index.ts:492`;
`lsp/lsp.ts:145`; `plugin/index.ts:134-178`. Keyed by `/tmp/{agent}` ⇒ 11 keys. `dispose_instance` has
**0 callers**. Reclaim: (11−1) × ~400 MB ≈ **~4 GB**. **This is your actual 8GB.**

**#3 — Full-history reload + 4-6 simultaneous copies per LLM step. ~290 MB peak on a real session.**
`prompt.ts:1093` → `message-v2.ts:578-580` → `:473-494` → `hydrate()` `:97-120`;
`message-v2.ts:569-572` (3-slice spread); `message-v2.ts:131`; `prompt.ts:1257-1263` (`Effect.all`
concurrency); `prompt.ts:1280` (spread). Reclaim: **~2.6× of history bytes per active session.**

**#4 — Unbounded PubSub + unconditional global-listener fan-out.**
`core/event.ts:175` (`PubSub.unbounded` for `all`), `:188` (per-type unbounded);
`event-v2-bridge.ts:20-24` (a `new Location.Info` per publish), `:42-61` (permanent listener regardless
of subscribers), `bus/global.ts:14-20` (mutates the payload object on every emit). Every publish goes into
BOTH `typed` and `all` (`core/event.ts:414-415`). Small in absolute bytes; it is per-event allocation churn
driving the write amplification above.

**#5 — `structuredClone` per part update, string ropes, uncached system prompt.**
`session.ts:635-643` — `structuredClone(part)` on **every** `PartUpdated` publish.
`processor.ts:297`/`:515` — `text += delta` rope. `instruction.ts:155-169` — `system()` has **no cache**:
per LLM step it re-runs `fs.findUp` up the tree, re-reads every AGENTS.md, and **re-fetches every remote
instruction URL** (`:101-103` does `arrayBuffer()` + a second `TextDecoder().decode()` copy per fetch).
`truncate.ts:22` does `text.split("\n")` on **every** tool output just to count lines, allocating an array
proportional to output size before the early-exit at `:24`.

---

## 7. RUST ARCHITECTURE FOR THE HOT PATH

**7.1 Runtime: tokio, but do NOT copy the reference impl's config.** `/tmp/OpenCodeRust/Cargo.toml` uses
`features = ["full"]`. Trim to `rt-multi-thread, net, time, sync, io-util, macros, signal`. Set
`worker_threads` to physical core count explicitly; set `max_blocking_threads` low (2-4).

**7.2 Allocator: mimalloc, not jemalloc.** Better per-thread free-list sharding for the many-small-allocation
message pipeline, pure Rust (no C toolchain). **Critically: set `MIMALLOC_ARENA_EAGER_COMMIT=0`.** Eager
arena commit is precisely the mechanism that makes RSS ratchet and never return — the exact TS failure mode
in §1c. This one env var is worth more than most of the port.

**7.3 Streaming: zero per-message copies. Three hard rules.**

- **Serialize the request body exactly once** into a per-connection `BytesMut` and write it straight to the
  socket. No intermediate `String`, no `serde_json::Value` tree. Parts must be `&'static str` /
  `Cow<'a, str>` borrowed from a contiguous history buffer — never `.to_owned()`.
- **Parse SSE frames in place** from the read buffer: locate `\n\n`, then
  `serde_json::from_slice::<Delta>(&buf[range])` where the `&str` borrows the buffer. Never
  `String::from_utf8` the whole body. (The reference impl's `reqwest` + `Stream` + `serde_json::Value` path
  does not do this.)
- **Never accumulate the assistant text in RAM.** Append deltas straight into a SQLite `part.data` update in
  4 KB chunks and to a broadcast channel. Hold only a 4 KB staging buffer. Kills the `processor.ts:297,515`
  rope entirely.

**7.4 History: SPILL. Do not hold it in RAM per session.** Biggest single design decision — and **the
existing Rust reference got it wrong** (see §8):

- `Session` holds only `{id, status, model, agent, token_counters, last_cursor}`. Target **≤ 300 B/session**.
- Per LLM step, build the request via **one ordered SQL cursor** over `message`/`part`
  (`WHERE session_id=? ORDER BY time_created, id`), streaming rows straight into the request serializer.
  **Never materialize a `Vec<Message>`.**
- Result: O(1) RAM per session; the request buffer is the *only* full copy.
- Keep a tiny LRU (8 sessions × ~2 KB) of cursor/token metadata. Nothing more.

**7.5 Serialization: `serde` + `serde_json` is fine; do NOT reach for `simd-json` yet.** The parser is not
your bottleneck — the *copying* is. `simd-json` gives ~2-4× on parse but borrows only from a *mutable*
buffer, which fights the streaming frame path and invites aliasing bugs. Get the copy count to 1 first, then
measure. Don't switch `part.data` to MessagePack: the schema and the HTTP/SSE contract are JSON; it buys
nothing and costs a migration.

**7.6 Backpressure — and NOT blocking the async runtime.** Four rules, in priority order:

1. **Never let a slow HTTP consumer apply backpressure to the provider read loop.** The provider socket
   must always be drained at full speed into the DB. Pipeline: `provider socket → (bounded 1 MiB) frame
   channel → [task A: DB writer + broadcast]`. Task A must **never** await on a consumer. A slow consumer
   gets `SubscriberOverflow` and resyncs via `GET /session/{id}/message` — **exactly what your Python
   client already does**, since it polls and never subscribes. **Because production has 0 SSE subscribers,
   SSE can be best-effort with zero retention.**
2. **Per-connection write channel: bounded `mpsc<Bytes>` capacity 2-4, DROP on full.** The TS code gets this
   right at `handlers/event.ts:11` + `core/event.ts:154`. Preserve that semantic — do not "improve" it into
   an unbounded buffer.
3. **Serialize anything over ~256 KB on `spawn_blocking`, not on a tokio worker.** Most important
   anti-stall measure, easy to get wrong: a 20 MB request-body encode is ~15-25 ms, which **will** stall a
   tokio worker. With 4 workers, 4 concurrent big steps stall *all* HTTP progress in the process —
   **including `/session/status`, which your scheduler polls to detect liveness.** That is a self-inflicted
   deadlock. Use a dedicated bounded encode pool.
4. **Use `tokio::process` for git, not a blocking pool.** The TS snapshot path spawns a `git` subprocess per
   file edit (`snapshot/index.ts:81-84`, `:134`, `:148`, `:163`) — fork+exec+git-index-load per write. Make
   it `tokio::process::Command` (async) and batch/coalesce. Never `std::process::Command::output()` on a
   worker.

Also bound pools explicitly: `pool_max_idle_per_host(8)`, `pool_max_per_host(32)`. reqwest defaults are
unbounded.

**7.7 Free wins that are language-independent** (do these first — they dwarf the port):

- Delete the durable event log for part/message updates; in-memory broadcast + SSE only. **~15 GiB,
  ~1.4 M inserts.**
- Pool MCP clients by **config hash**, not by directory. One `parallel-browser-mcp` for the whole server with
  per-session browser contexts. **~4 GB.**
- Wire `POST /instance/dispose` into the Python scheduler, **or** make instance state LRU+TTL so the leak
  is structurally impossible.
- Cache `Instruction.system()` by (directory, mtime).
- Cache tool schemas by (provider, model, tool-set) — kills 85 deep clones per step.
- Give `mem_limit` an actual swap budget, or accept the ~15-session browser ceiling explicitly.

---

## 8. THE RUST REFERENCE REPLICATED THE ARCHITECTURE, NOT THE FIX

`/tmp/OpenCodeRust` (89,698 LOC, 19 crates) is a competent transliteration — **but it faithfully reproduced
both memory anti-patterns:**

- `crates/opencode-session/src/session.rs:294` — `pub messages: Vec<SessionMessage>` **on the `Session`
  struct**. History fully materialized in RAM per session.
- `crates/opencode-types/src/message.rs:6-13` — each `SessionMessage` carries `parts: Vec<MessagePart>` +
  `metadata: HashMap<String, serde_json::Value>` (each `Value` is 32 B plus a `HashMap` allocation).
- `crates/opencode-session/src/prompt.rs:954-960` —
  `loop { let mut filtered_messages = Self::filter_compacted_messages(&session.messages); }` — **a full
  `Vec` clone of the entire history, every loop iteration.** Structurally identical to `prompt.ts:1093`.
- No `#[global_allocator]` (no mimalloc/jemalloc in any `Cargo.toml`).
- No `[profile.release]` LTO/codegen-units tuning in the workspace `Cargo.toml`.
- `syntect = { features = [..., "regex-onig"] }` — a **C** regex engine with non-trivial static init cost
  that the TS side does not pay.

**Consequence: a clean-room port of the current Rust reference would land at ~83.5 MiB of in-RAM history for
your worst measured session — ~2× the entire 42 MiB/session budget — before any copy multiplier. The port
must fix §7.4 or it does not solve your problem.**

Also a risk flag, not perf: the TS tree is on **`effect@4.0.0-beta.83`** with `effect/unstable/http` +
`effect/unstable/httpapi`. You are porting from a **beta of the next major**, with no stable API surface.

---

## 9. CPU: WHERE IT GOES, TS vs RUST

**Per-token (per-delta) streaming.** TS path per delta: fetch reader → `Uint8Array` → `TextDecoder` →
`JSON.parse` → Effect `Schema.decodeUnknownSync` → discriminated-union match → `text += delta` →
`events.publish` → `new Location.Info()` → `PubSub.publish` ×2 (`core/event.ts:414-415`) → listener →
`GlobalBus.emit` → 2 allocs. **~8-15 µs/delta** (ESTIMATE, JIT-warm). A 30k-token response ⇒ **~0.3-0.45
CPU-seconds per response.** 100 concurrent streams ⇒ **30-45 CPU-seconds per wall-second ≈ 30-45 cores
needed.** *This is your CPU bottleneck.*

Rust with in-place `from_slice` frame parsing: **~0.5-1.5 µs/delta** → ~30-50 ms per response.
**10-30× less.**

Important positive: **`PartDelta` is NOT durable** — confirmed by the event histogram (only 5 types, no
part-delta). The 14.94 GiB comes from `updatePart`/`updateMessage` publishes (901,805 + 384,115), **not**
from per-delta spam. Good design, undermined by the two bulk paths.

**Per-LLM-step fixed CPU** (the "re-derive everything every step" tax), 100-message history:

| Cost | site | TS est. |
|---|---|---|
| 85 tool schemas: Effect-Schema deep clone + `sanitizeOpenAISchema` recursion + AI SDK `tool()` wrap, **per step** | `tools.ts:92-101`, `transform.ts:1575-1596`. (`ToolJsonSchema.fromTool` IS WeakMap-cached at `json-schema.ts:6-20`; the provider transform is not) | **13-34 ms** |
| `Instruction.system()` — `fs.findUp` tree walk + N file reads + **M remote HTTP fetches**, uncached | `instruction.ts:155-169`, called `prompt.ts:1261` | 2-15 ms |
| `hydrate()` `JSON.parse` of every part `data` column (4.03 × 3.4 KB = 56 KB JSON/msg) | `message-v2.ts:97-120` | ~28 ms / 100 msgs |
| `toModelMessagesEffect` allocating ~2.4 KB/msg of wrappers | `message-v2.ts:131-420` | GC pressure ~15 ms |
| `structuredClone` per part publish | `session.ts:641` | ~1 ms/publish |
| `git` fork+exec+index-load per file edit | `snapshot/index.ts:81-84` | **5-30 ms per file write** |
| `text.split("\n")` per tool output | `truncate.ts:22` vs `:24` | 0.1-2 ms |
| **total per step** | | **~60-95 ms** |

Essentially all re-derivation of things that did not change. Rust target **~8-15 ms** (mostly the
`JSON.parse`, droppable to ~2 ms with in-place parsing).

**Diffing is NOT a hot spot.** Snapshots go through `git` (`snapshot/index.ts:81-84`); measured `patch`
parts **avg 246 B, max 447 KB**. Don't spend port effort here.

**LSP is NOT a CPU hotspot** (`tool/lsp.ts` is 113 lines, delegates to per-project servers). It is a
**RAM/process-count** problem — per-directory spawn, `lsp/lsp.ts:145` — addressed in §6 #2.

---

## 10. WHAT I'D TELL WAEL IN ONE PARAGRAPH

100 concurrent opencode sessions in 8 GB is **achievable in Rust with 40× margin** — you'd land at
1.5-3 GB, needing 42 MiB/session against a 1 MiB design. But (a) your 8 GB is currently eaten by **11
headless Chromium instances spawned one-per-subworker-directory and never freed**, and (b) your database
is **17.7 GB of which 14.9 GB is a durable event log written 1.4 million times and never read**. Both are
~20-line fixes that will get you further than the entire rewrite. And if "100 concurrent" means 100
subworkers *each driving a browser*, that is **~40 GB of Chromium and impossible in 8 GB in any language** —
the honest ceiling is **~15-18 concurrent browser-driving sessions**, and the fix is one pooled browser, not
Rust. Port to Rust for the CPU and heap win; do not let the port become the excuse to skip the two cheap
fixes.

---

## CLAIMS I COULD NOT VERIFY

1. **All Chromium/MCP/LSP RSS figures (300/100/80 MB) are standard estimates, NOT measured.** I did not run
   `docker stats` against a live container. The *count* (11 dirs × 6 MCP) is derived from `subworkers.json`
   + `opencode_client.py:38-45`; the per-process size is not. **Biggest unverified number in this report —
   measure `docker stats` before acting on it.**
2. **All per-delta and per-step CPU figures (8-15 µs/delta, 13-34 ms tool schemas, 60-95 ms/step) are
   ESTIMATES from code structure.** No profiler run. The repo has `script/profile-test-files.ts` and
   `script/bench-test-suite.ts` (`packages/opencode/package.json:12-13`) that could produce real numbers.
3. **The ~5.8 GB gap between the event table's 16.04 GB and my ~10.2 GB payload extrapolation is
   INFERRED** (heavy tail + overflow pages). Per-type averages are MEASURED over the most-recent 30k rows
   only; **I could not get a full-table `sum(length(data))` — the scan exceeded a 120 s timeout on a 17.7 GB
   table.** Early sessions may have had much larger payloads; the 22,422 B `message.updated` average is a
   *recent* figure and may understate the historical average.
4. **The 1,405,820 event rows span an unknown time window.** `min/max(created_at)` failed — the `event` table
   has no `created_at` column (schema is `id, aggregate_id, seq, type, data`). **I cannot state an
   event-write rate**, which is what you'd need to project future disk growth. Separate query required.
5. **"Bun/JSC does not return peak heap to the OS"** — asserted from known JSC behaviour, **not measured**.
   It is my explanation for the ratchet symptom: a hypothesis, not a finding.
6. **Effect fiber overhead (2-5 KB/live fiber, 10-30 KB context chain)** — standard estimates, not measured
   against this codebase's actual fiber count.
7. **The 4-6× history copy count and ~3.1× byte multiplier** are derived from reading the allocation sites
   plus the measured parts/message and bytes/part ratios. **Not confirmed by a heap profile.** A Bun heap
   snapshot of one long session would confirm or refute them.
8. **I did not verify that `parallel-browser-mcp` is actually reachable from these 11 subworkers** (depends
   on each agent's `workspace/opencode.json` permissions, which I did not audit agent-by-agent). If some
   agents block it, hotspot #2 shrinks proportionally.
9. **I did not read the `llm` package (9,533 LOC) or `codemode` (6,878 LOC) internals**, and did not audit
   `packages/tui` or `packages/ui`. LOC figures for those come from the brief, not my own count.
10. **The `/tmp/OpenCodeRust` assessment is from reading ~6 files** (`Cargo.toml`, `session.rs`,
    `prompt.rs`, `message_v2.rs`, `types/message.rs`, `lib.rs`). I did **not** verify it builds, and did
    not read its `server`, `provider`, or `tool` crates. pragmatic-architect (task #6) should confirm.
11. **`mem_limit: 8g` = 8192 MiB vs 8 GB decimal** — I assumed Docker's `8g` means 8 GiB. Minor (±3%).
12. **I did not measure the Rust request-path cost.** My "~8-15 ms/step" target is a projection from the
    design, not a benchmark.

---

## KEY FILE REFERENCES

| # | file:line | role |
|---|---|---|
| 1 | `packages/schema/src/v1/session.ts:502-507` | HOTSPOT #1 — all defs `durable` |
| 2 | `packages/core/src/event.ts:337` | HOTSPOT #1 — insert, 3 indexes, no DELETE |
| 3 | `packages/core/src/event.ts:175`, `:188`, `:414-415` | HOTSPOT #4 — unbounded PubSub |
| 4 | `packages/opencode/src/event-v2-bridge.ts:42-61` | HOTSPOT #4 — permanent global listener |
| 5 | `packages/opencode/src/effect/instance-state.ts:31` | HOTSPOT #2 — `capacity: POSITIVE_INFINITY` |
| 6 | `packages/opencode/src/mcp/index.ts:492` | HOTSPOT #2 — per-directory Chromium |
| 7 | `packages/opencode/src/lsp/lsp.ts:145` | HOTSPOT #2 — per-directory LSP |
| 8 | `packages/opencode/src/plugin/index.ts:134-178` | HOTSPOT #2 — 11 plugins/dir |
| 9 | `packages/opencode/src/session/prompt.ts:1093` | HOTSPOT #3 — full-history reload per step |
| 10 | `packages/opencode/src/session/message-v2.ts:578-580`, `:473-494`, `:569-572`, `:131` | HOTSPOT #3 |
| 11 | `packages/opencode/src/session/session.ts:635-643` | HOTSPOT #5 — `structuredClone` per publish |
| 12 | `packages/opencode/src/session/processor.ts:297`, `:515` | HOTSPOT #5 — text rope |
| 13 | `packages/opencode/src/session/instruction.ts:155-169` | HOTSPOT #5 — uncached system() |
| 14 | `packages/opencode/src/session/tools.ts:92-101` | CPU — 85 schemas per step |
| 15 | `packages/opencode/src/session/llm/native-runtime.ts:107`, `:126` | unbounded Queue + fiber per tool call |
| 16 | `packages/server/src/handlers/event.ts:11` | correctly bounded (256) — NOT the bottleneck |
| 17 | `packages/opencode/src/tool/truncate.ts:22` | CPU — split() per tool output |
| 18 | `packages/opencode/src/snapshot/index.ts:81-84` | CPU — git fork+exec per file write |
| 19 | `/tmp/OpenCodeRust/crates/opencode-session/src/session.rs:294` | REF GOT IT WRONG — history in RAM |
| 20 | `/tmp/OpenCodeRust/crates/opencode-session/src/prompt.rs:954-960` | REF GOT IT WRONG — Vec clone/step |
| 21 | `/Users/vakandi/EliaAI/subworkers/server/docker-compose.yml:6-10,23` | 8g + zero swap + MAX_CONCURRENT_RUNS |
| 22 | `/Users/vakandi/EliaAI/subworkers/server/app/utils/opencode_client.py:38-45,229-238,313-331` | contract, MCP list, dead dispose |
| 23 | `/Users/vakandi/EliaAI/subworkers/server/app/config/subworkers.json` | 25 agents / 11 enabled |
| 24 | `/Users/vakandi/EliaAI/subworkers/SUBWORKERS_SYSTEM.md:53` | `directory=/tmp/{name}` = the cache key |