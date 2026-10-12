# SKEPTIC FINDINGS — Round 1

**Member:** feasibility-skeptic
**TeamRunId:** bc3f103e-f405-4e5e-b685-014e4bc7f0b3
**Date:** 2026-10-05

---

## PREMISE UNDER TEST

> "Convert the opencode TypeScript server (204,808 LOC surviving at
> `/Users/vakandi/Documents/opencode-server-rust`) to a pure Rust server that handles
> 100 concurrent sessions in 8GB RAM, while ALSO fully implementing all features of a
> 27,425-file TypeScript plugin (oh-my-openagent at
> `/Users/vakandi/EliaAI/setup/oh-my-openagent`) plus a proxy plugin. A previous partial
> attempt exists at `/tmp/OpenCodeRust` (89,698 LOC)."

---

## VERDICT UP FRONT

**The premise is unsound.** Two of its three legs are arithmetically or architecturally
false, and the actual bottleneck is not the language.

Each attack below cites a `file:line` or a concrete measurement. A single well-evidenced
killer argument is worth more than twenty weak ones — the killer is **Attack 3**
(measured memory floor) combined with **Attack 4** (root cause is configuration).

**Note on method:** I retracted my own assigned hypothesis (that the plugin monkey-patches
opencode internals). It does not. I found something better, and I say so explicitly in
Attack 5 rather than quietly dropping it — the retraction is load-bearing for the rest of
the report's credibility.

---

## ATTACK 0 — The 8GB is not the server's budget

This kills the framing before any analysis begins.

`/Users/vakandi/EliaAI/subworkers/server/docker-compose.yml:9-10`
```yaml
mem_limit: 8g
memswap_limit: 8g
```

`memswap_limit == mem_limit` means **swap is disabled**. This is a hard 8,192 MB cgroup
ceiling with zero swap headroom, OOM-kill on breach.

And line 8, in the repo's own words:
> `# 8GB + swap prevents OOM-kills from parallel-browser-mcp + Chrome leaks.`

**The operator already knows Chrome is a first-class consumer of this 8GB.** So are the
Python FastAPI server, cloudflared, and every MCP child process. The premise's "100
sessions in 8GB" silently re-attributes the entire container to the thing being rewritten.

---

## ATTACK 1 — Measured memory floor: 8GB / 100 sessions is not reachable

I measured a live container rather than guessing.

`docker stats --no-stream` on the running `vouch-opencode` container
(**idle, 0.65% CPU, zero active sessions**), plus per-process `/proc/<pid>/status`:

| process | RSS | note |
|---|---|---|
| `opencode` (Bun server) | **465.3 MB** | zero sessions running |
| 5 x `python` MCP children | 278.0 MB | 5 configured MCP servers |
| 1 x `node` MCP child | 19.5 MB | |
| **total** | **812.1 MB** | matches `docker stats` 797 MiB |

Derived constants:
- **~49.7 MB per MCP child process** (298.0 MB / 6 children)
- **465 MB is the idle Bun server baseline**, with only 3 MCP servers configured

### Production scale-up

`/Users/vakandi/.config/mcp/mcp_servers.json` contains **54 servers**. Breakdown:

| launcher | count |
|---|---|
| `npx` | 14 |
| `sh` wrapper | 14 |
| `python3` | 12 |
| `uv` | 5 |
| `node` | 3 |
| `(none)` | 3 |
| `gsc-mcp` | 1 |
| `uvx` (abs path) | 1 |

**18 are node/npx-backed.**

### Single-directory floor (best case — all 25 agents share one project dir)

```
  465 MB   Bun server (measured; UNDER-counts, only 3 MCP loaded)
2,684 MB   54 MCP children @ 49.7 MB measured unit cost
--------
3,149 MB   consumed before ONE agent starts
```

Against 8,192 MB — and this is **before** Chrome (compose line 8 places it at 1-3 GB),
Python FastAPI, and cloudflared (~250 MB).

**Headroom for 100 concurrent sessions: ~1.8-3.8 GB => 18-38 MB per session.**

An LLM session carrying tool results (file reads, grep hits, shell output) retains
hundreds of KB to multiple MB of JSON per message. 18-38 MB/session is achievable only
with aggressive history externalization + streaming offload — **an architectural change
the TS server does not currently make**, and which has nothing to do with Rust.

### But the real topology makes it 8x worse — and that IS the root cause

`server/app/config/subworkers.json` — **25 subworkers, 25 distinct workspace
directories** (`/data/subworkers/<name>/workspace`, one `/data`, two host paths).
Measured: 24 unique `/data/subworkers/*/workspace` + `/data` + 2 host paths.

`app/services/runner.py:439-444` maps each to a distinct `directory=` argument:
```python
openworkspace = os.environ.get("OPENCODE_WORKSPACE", "")   # = "/data" (compose:16)
if openworkspace and workspace.startswith("/data/"):
    directory = workspace.replace("/data/", f"{openworkspace}/", 1)
```

And MCP state is cached **per directory, unboundedly**:

- `packages/opencode/src/mcp/index.ts:492` — `InstanceState.make<State>` for MCP
- `packages/opencode/src/effect/instance-state.ts:30-31` —
  `ScopedCache.make({ capacity: Number.POSITIVE_INFINITY, ... })`
- `packages/opencode/src/effect/instance-state.ts:49` — key is `yield* directory`
- `packages/opencode/src/effect/instance-state.ts:38` — invalidated only on explicit
  dispose, **never on size**

`mcp/index.ts:505-529` then eagerly spawns **all 54** configured servers for that
directory, with `{ concurrency: "unbounded" }`, each via
`new StdioClientTransport({ command: cmd })` (`mcp/index.ts:347-349`) — a real OS child
process. Comment at `mcp/index.ts:190-191` confirms the intent:
> `// match is an absolute filesystem path from Glob.scanSync(..., { absolute: true }).`

### The multiplier

```
25 directories x 54 MCP servers = 1,350 child processes (450 of them Node)
1,350 x 49.7 MB               ~= 67 GB
```

That is **8x past the entire 8 GB budget.**

### Maximum realistic N

| topology | max concurrent sessions | why |
|---|---|---|
| current (25 directories) | **~10** | matches reported 20-session stall order-of-magnitude; the stall is the cgroup, not the language |
| single directory + history externalization | **~25-40** | 3.1 GB floor, Chrome still competing |
| single directory + pruned MCP + externalized history + swap | **~100** | achievable without Rust |

**100 is not reachable in the current topology, in any language.**

### Unbounded cache blast radius

There are **23 separate `InstanceState.make` unbounded per-directory caches** in the
server. Each holds a full copy of tool registry / MCP clients / config per directory,
never evicted.

---

## ATTACK 2 — "Pure Rust" is architecturally false (the plugin is not the reason)

### The plugin is well-behaved at the source level

Measured import census across the plugin's 7,055 TS/TSX files (excluding `node_modules`
and `dist`):

| import | count |
|---|---|
| `@opencode-ai/plugin` | 248 |
| `@opencode-ai/sdk` | 51 |
| `@opencode-ai/plugin/tool` | 44 |
| `@opencode-ai/plugin/tui` | 2 |
| `@opencode-ai/sdk/v2` | 3 |

**Exactly 1 file** in 7,055 references the opencode source tree at all
(`packages/omo-opencode/src/shared/agent-sort-shim.ts`). There are **no** deep relative
imports escaping the package. This is a clean, versioned public boundary. **The premise
earns credit here, and I say so plainly.**

### But "pure Rust" dies on five hard requirements

**1. The server is a runtime TS module loader.**
`packages/opencode/src/tool/registry.ts:185-192`
```ts
const matches = dirs.flatMap((dir) =>
  Glob.scanSync("{tool,tools}/*.{js,ts}", { cwd: dir, absolute: true, dot: true, symlink: true }),
)
...
const mod = yield* Effect.promise(() => import(pathToFileURL(match).href))
for (const [id, def] of Object.entries(mod)) { if (!isPluginTool(def)) continue; ... }
```
It globs `*.{js,ts}` and `import()`s them, enumerating exports for tool definitions.
Comment at `:190-191`: *"Import it as `file://` so Node on Windows accepts the dynamic
import."*

The production proxy is itself such a file — `setup/PROXY_LOCAL_PLUGIN.md` documents
`~/.config/opencode/plugins/proxy-local.ts`.

**2. The server must invoke arbitrary JS closures and introspect zod objects, on the
tool hot path.**
`registry.ts:154`
```ts
const result = yield* Effect.promise(() => def.execute(args as any, pluginCtx))
```
`registry.ts:130-137` takes the plugin's zod shapes, wraps them in `z.object(args)`,
guards with `Schema.declare`, and `registry.ts:375` calls
`z.toJSONSchema(schema, { io: "input", metadata: zodMetadataRegistry(schema) })` —
walking zod v4 internals to build the LLM-facing schema.
`packages/plugin/src/tool.ts:52` sets `tool.schema = z`, so the plugin hands the server a
**zod runtime object**, not JSON.

**3. The tool contract requires Web-standard + Bun/Node types.**
`packages/plugin/src/tool.ts:3-20`: `abort: AbortSignal`, `metadata()`, `ask(): Promise<void>`.
`packages/plugin/src/shell.ts:10-43`: `BunShell` is a **tagged-template function**
returning a `BunShellPromise extends Promise` that is *also* an `AsyncIterable<string>`,
with `.lines()`, `.json()`, `.quiet()`, `.nothrow()`. `shell.ts:74,106` use
`BufferEncoding` and `Buffer` globals.

**4. The in-process HTTP CONNECT proxy.**
`setup/PROXY_LOCAL_PLUGIN.md:3-4`:
> *"Le plugin fait tourner un forward proxy HTTP/CONNECT sur `127.0.0.1:18888` dans le
> process Bun d'opencode lui-même. Opencode pointe une fois vers localhost au startup;
> l'upstream résidentiel est relu a chaque requête."*

Every LLM token stream for every session traverses it. Upstream re-read per request from
`~/.proxychains.conf`; `setup/proxies.txt` holds 117 proxies. It is installed as
`proxy-local.ts` — a TypeScript file loaded by the same dynamic-import mechanism.

**5. `@opentui/*` type resolution survives any server-side tui severance.**
`packages/plugin/package.json` declares optional peerDependencies on `@opentui/core`,
`@opentui/keymap`, `@opentui/solid` (all `>=0.4.5`), and exports `"./tui": "./src/tui.ts"`.
`packages/plugin/src/tui.ts:19-20` imports `CliRenderer, KeyEvent, RGBA, Renderable,
SlotMode` from `@opentui/core` and `Binding, Keymap` from `@opentui/keymap` — a full
terminal renderer. omo consumes it at `packages/omo-opencode/src/tui.ts`.

**⇒ Severing `packages/tui` from the *server* does not free the *plugin host* from
`@opentui/*`.** A Rust plugin host must make that subpath resolvable even for a headless
server that never renders a frame. Either stub `@opentui/*` (not trivially stubbable —
`Renderable`/`SlotMode` are real generic signatures) or fork `@opencode-ai/plugin` to
carve a `plugin-server` export map, which means maintaining a divergence from a package
omo pins by exact version (`omo-opencode/package.json`: `@opencode-ai/plugin: 1.15.13`).

### The previous attempt already discovered this and capitulated

`/tmp/OpenCodeRust/crates/opencode-plugin/src/subprocess/mod.rs:3-5`
> *"This module provides the ability to load TS/JS plugins by spawning a `plugin-host.ts`
> child process and communicating over Content-Length framed JSON-RPC 2.0 (stdin/stdout)."*

- `loader.rs:29` -> `"no JS runtime found (install bun, deno, or node)"`
- `client.rs:151` -> `Command::new(runtime.command())`
- `client.rs:161` -> `let mut child = cmd.spawn()?;`

### The consequence

**The realistic architecture is Rust + a Bun/Node child. There is no "pure Rust"
server.** And that child lives in the same 8,192 MB cgroup. Moving 1.13M LOC of plugin
heap from the Rust process's address space into a sibling process **reduces total RAM by
zero bytes.** It also adds a JSON encode/decode + pipe round-trip to `chat.message`,
`tool.execute.before/after`, and `experimental.chat.messages.transform` — the per-turn
hot path. The previous attempt's `subprocess/mod.rs:27-40` enumerates 13 such hooks.

---

## ATTACK 3 — THE ROOT CAUSE: configuration, not language

**This is the killer argument. It is short.**

### Evidence the failure mode is cgroup OOM, not GC or Effect

- `subworkers/server/app/services/idle_cleanup.py:1-13` (docstring) — production is kept
  alive by a Python daemon that, every 30s when idle, **SIGKILLs `parallel-browser-mcp`
  and `chromium_headless_shell`**, and at `MAX_IDLE_RESTARTS = 3` (`:30`) **restarts
  opencode**.
  > *"kills parallel-browser-mcp and Chrome headless processes that consume the majority of
  > container RAM. This prevents OOM-kills caused by stale browser instances"*

  **If the cause were Bun's GC or the Effect runtime, killing Chrome would not fix it.** It
  targets browser MCP children.

- `docker-compose.yml:7` — *"the entrypoint auto-restarts opencode after a cgroup OOM-kill"*.
  The documented failure is the cgroup.

### The cost function is data, not code

```
total RAM = directory_count x mcp_config_size x ~50MB, unbounded and un-evicted
```

All three factors are **configuration**. Concretely:

| fix | effort | effect on the multiplier |
|---|---|---|
| point all 25 agents at ONE shared project dir | hours (subworkers.json + runner.py:439-444) | 1,350 -> 54 children |
| `mcp.enabled: false` for the ~45 servers no subworker calls | hours | 54 -> ~9 children |
| bound `instance-state.ts:31` (`capacity: POSITIVE_INFINITY`) | ~1 day | stops unbounded growth |
| enable swap (`memswap_limit: 16g`) | minutes | spike degrades instead of OOM-kill |

Either of the first two alone moves the floor from **~67 GB to ~3 GB**.
**No Rust code required.**

### So the rewrite attacks 5.7% of the budget

The strongest honest case for Rust is the server's own **465 MB** idle heap. A Rust core
lands plausibly at 120-180 MB — a real ~300 MB win, worth maybe **8-15 extra concurrent
sessions**. But that is **5.7% of 8 GB**. The other 94% is MCP children and Chrome.

**The rewrite optimizes the 5.7%.**

---

## ATTACK 4 — Project killers

### Duration

Measured scope:

| artifact | measured |
|---|---|
| TS server, all `packages/*/src` | **160,687 LOC** |
| TS server, `packages/opencode/src` alone | 76,151 LOC |
| oh-my-openagent source (excl. `node_modules`, `dist`) | **1,135,619 LOC** across **7,055 TS/TSX files** (10,410 total files) |
| oh-my-openagent feature hooks (`config/schema/hooks.ts:3-62`) | **58** |
| oh-my-openagent tool groups (`src/tools/*`) | **16** |
| oh-my-openagent subprocess spawn sites | **37** files |
| `/tmp/OpenCodeRust` | 130,139 LOC across 218 `.rs` files, 360 test fns |

Realistic estimate for behavioural parity: **2-4 engineer-years** — and that assumes the
plugin stays in JS via subprocess. A pure Rust reimplementation of 1.13M LOC is a
separate, much larger project.

### What breaks silently, with no test coverage to catch it

`/tmp/OpenCodeRust` test distribution:

| crate | tests | LOC | ratio |
|---|---|---|---|
| `opencode-mcp` | **2** | 2,376 | 1 per 1,188 |
| `opencode-plugin` | **3** | 1,951 | 1 per 650 |
| `opencode-storage` | **0** | 1,263 | **NONE** |
| `opencode-server` | 5 | 6,832 | 1 per 1,366 |
| `opencode-cli` | **0** | 5,551 | **NONE** |
| `opencode-core` | 3 | 267 | 1 per 89 |
| `opencode-types` | **0** | 359 | **NONE** |
| `opencode-session` | 151 | 14,115 | 1 per 93 |
| `opencode-provider` | 73 | 17,489 | 1 per 240 |
| `opencode-tui` | 14 | 21,418 | 1 per 1,530 |

**The two crates carrying the entire premise's risk — MCP fan-out and plugin host — have
5 tests combined.** Storage (the SQLite layer) has zero. Meanwhile the TS original ships:

- **480 test files** (`packages/**/*.test.ts`, excluding `node_modules`)
- a whole-route exerciser: `packages/opencode/package.json:11` ->
  `bun run script/httpapi-exercise.ts --mode coverage --fail-on-missing --fail-on-skip`
  (+ `--mode auth`, `--mode effect`)
- LLM-output **snapshot** tests (`packages/opencode/test/snapshot/snapshot.test.ts`)

Those encode exact prompt/stream behaviour. Porting them is itself a multi-month project —
and *that* is the work before any real work starts.

**Worse: the failure mode is silent by construction.** A Rust server that spawns 54 MCP
children per directory but forgets to make them directory-scoped will not crash — it will
just get slower and get OOM-killed at 3am, and `idle_cleanup.py` will restart it, and the
agent will silently do less work. That is exactly the bug class that costs a week to find
in production.

### Migration / rollback for 25 live agents

Consumer contract is `/Users/vakandi/EliaAI/subworkers/server/app/services/runner.py`:
SSE streaming at `runner.py:637` (`/event?directory=...`), session create/delete,
permission prompts.

25 agents on cron schedules fire unprompted. A Rust server that mishandles one SSE
reconnect silently corrupts a day's Telegram/Discord/SEO output for
`mirorpay-community-organic`, `bene2luxe-promoter`, `teleorbit-seo`, and 21 others.
Rollback means running **two server implementations side by side** and flipping the client
— i.e. maintaining an opencode-compatible SSE/event contract in Rust forever. That is a
permanent tax, not a one-time cost.

---

## ATTACK 5 — The strongest case FOR the rewrite (stated fairly)

I am obliged to make this strong:

1. **465 MB idle for a server doing nothing is genuinely bad.** That is Bun's baseline
   heap (JSC) plus the Effect runtime plus the loaded plugin module graph. A Rust core
   plausibly lands at 120-180 MB. Against a tight cgroup that is a real ~300 MB win.
2. **The Effect runtime tax is real and language-attributable.** Every hot-path operation
   is `yield*`-bound through `Effect.gen` generators with `withSpan` instrumentation
   (`registry.ts:171-178`). Effect is not free, and it is an architectural commitment you
   cannot tune away in TS.
3. **A Rust provider/streaming layer fixes the right thing.** Zero-copy SSE decode,
   bounded channels, real backpressure, no GC pauses mid-stream. `docker stats` shows
   **0.65% CPU** today — enormous CPU headroom being wasted. The system is memory-bound,
   and Rust attacks memory *for the server's own heap only*.
4. **It is intellectually attractive and there is real momentum.** `/tmp/OpenCodeRust` at
   130K LOC is substantial work.

**All four are about the server's own 465 MB = 5.7% of the 8 GB budget.** The other 94% is
MCP children and Chrome, which the rewrite does not touch by one process.

---

## VERDICT

**IS THE PREMISE SOUND? NO.** Three independent failures:

1. **The 8GB/100-session target is arithmetically impossible** in the current topology.
   Measured floor: 3.1 GB before any agent runs (single directory); ~67 GB in the real
   25-directory topology. Max realistic N is **~10 today, ~25-40 with config fixes, ~100
   only with history externalization** — none of which require Rust.
2. **"Pure Rust" is architecturally impossible.** The server dynamic-imports `.ts` and
   invokes JS closures with zod schemas on the hot path. Five independent requirements
   (`.ts` import, zod introspection, `BunShell`/`Buffer`, in-process HTTP CONNECT proxy,
   `@opentui` type resolution) each independently force a JS runtime. The previous attempt
   shipped a Bun/Node child. That child's RAM is in the same cgroup — total unchanged.
3. **The root cause is configuration, not language.**
   `directory_count x mcp_config_size x 50MB`, amplified by 23 unbounded per-directory
   caches. Hours of TS/config work fixes it; the rewrite does not touch the multiplier.

### THE SMALLEST PREMISE THAT IS SOUND

> **Keep opencode in TypeScript. Make all 25 subworkers share one project directory so the
> MCP fleet is spawned once instead of 25 times; disable the ~45 MCP servers no subworker
> calls; bound the 23 `InstanceState` caches; externalize message history to SQLite and
> stop retaining tool output in memory; and enable swap (`memswap_limit: 16g`) so a
> transient spike degrades instead of OOM-killing.**

Expected: **100 concurrent sessions in 8 GB**, achievable in **days, not years**, with
**zero migration risk** because nothing about the client contract changes.

### IF the team still wants the Rust work

The sound version is narrow and honest:

- **Port only `provider` + `session` streaming** — 27,604 of the 160,687 LOC, and the 224
  tests that already exist for them (`opencode-session` 151 + `opencode-provider` 73).
- **Keep Bun hosting** the plugin, the MCP supervisor, and storage.
- Treat the 465 MB -> ~180 MB win as the **entire, capped upside**.
- **Do NOT port `mcp`, `plugin`, or `storage`** — they have 2, 3, and 0 tests
  respectively, and they are not where the memory problem is.

---

## CLAIMS I COULD NOT VERIFY

1. **"204,808 LOC surviving"** — I measured **160,687 LOC** across all `packages/*/src`
   (76,151 in `packages/opencode/src` alone). The premise figure is ~27% higher. I could
   not reproduce the counting method; it may include `test/`, `script/`, or `node_modules`.

2. **"27,425-file TypeScript plugin"** — real source is **7,055 TS/TSX files / 10,410
   total / 1,135,619 LOC** excluding `node_modules` and `dist`. 41,361 with
   `node_modules`. 27,425 looks like a `node_modules`-inclusive count. **The LOC figure is
   the one that matters and it is worse than the premise implies.**

3. **"the 20-session stall"** — I found **no captured trace, log, or heap dump** of the
   actual stall. Attack 1/3 is an arithmetic reconstruction from a *different* container
   (`vouch-opencode`, idle, 3 MCP servers) plus the compose/config facts. I could not
   measure the 25-directory production container because `elia-subworker-srv` was **not
   running**. **A 10-minute `docker stats` + `/proc` walk on the live production container
   would confirm or refute my central claim outright. I recommend someone do exactly that
   before the team commits.**

4. **Per-MCP-child 49.7 MB** is derived from 6 children in a container with different
   servers than production. npx-backed servers pulling large npm trees could be
   materially higher — which would make my numbers *better*, not worse.

5. **Chrome's actual RSS** in the production container — compose line 8 asserts it is a
   major consumer but gives no number. My 1-3 GB is an estimate, not a measurement.

6. **"89,698 LOC"** for the previous attempt — I measured **130,139 LOC across 218 `.rs`
   files**. May be a different commit or excludes `crates/opencode-tui`.

7. **Whether any Rust JS engine (QuickJS / Boa / V8) was evaluated.** `/tmp/OpenCodeRust`
   went straight to subprocess. An in-process engine would avoid the IPC cost but *adds*
   to the same heap — which is why I do not believe it rescues the RAM target either. **I
   did not benchmark one.** This is a genuine gap: a QuickJS-vs-Bun memory comparison could
   change the plugin-hosting recommendation, though not the verdict.

8. **Version skew, noted but not pursued** — `omo-opencode/package.json` pins
   `@opencode-ai/plugin: 1.15.13` while the server repo is at `1.18.34`. Not load-bearing
   for my argument; I did not investigate whether the installed 1.15.13 surface is
   compatible with the 1.18.34 server.

---

## METHOD NOTES / SUGGESTED NEXT ACTIONS FOR THE TEAM

- **Before committing to anything:** run `docker stats` + a `/proc` walk on the live
  `elia-subworker-srv` container and count actual child processes. If it is ~1,350, the
  premise is dead on arrival and no rewrite is needed. Ten minutes, decisive.
- **perf-architect** should read `packages/opencode/src/tool/registry.ts:185-192` and
  `mcp/index.ts:492-529` before producing the per-session memory table — the per-session
  framing may be the wrong unit given the per-directory fan-out.
- **plugin-host** should note that the plugin's public boundary is *clean* (my retraction),
  so the contract mapping is tractable — the hard part is the runtime, not the API surface.
- **ts-anatomy** should record the 23 `InstanceState.make` sites and their cache keys as
  first-class output; unbounded per-directory caches are a real defect independent of any
  port decision.