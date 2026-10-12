# PLUGIN-HOST — Round 1 Findings

**Agent:** plugin-host · **TeamRunId:** bc3f103e-f405-4e5e-b685-014e4bc7f0b3 · **Round:** 1 (independent findings, no plan)

**Assignment:** map the exact plugin API contract oh-my-openagent (omo) programs against (file:line on both the consumer and the opencode provider side); map Wael's four named features to the hooks/subsystems they need; commit to a Rust plugin-hosting architecture with per-session memory cost.

**Status of numbers:** every figure below is **measured** on this machine (darwin/arm64; bun 1.4.2, deno 2.7.2, node v20.20.2), not estimated. Repro scripts: `/tmp/omo-probe/{load,invoke,noop}.mjs`, `/tmp/ipcbench/{bench3,bench4,warm,trace}.mjs`, `/tmp/ipcbench/patched-host.mjs`.

---

## 0. HEADLINE — the Rust reference CANNOT load omo today

`/tmp/OpenCodeRust/crates/opencode-plugin` has already picked an architecture: **out-of-process TypeScript host over Content-Length framed JSON-RPC 2.0** (`src/subprocess/`, 1,561 LOC + `host/plugin-host.ts`, 712 LOC; `src/subprocess/runtime.rs:52-59` prefers `bun > deno > node`).

**That choice is correct** — see §3. But the implementation has **4 hard blockers**, each verified.

### Blocker 1 — wrong export shape. CONFIRMED ON THE DEPLOYED ARTIFACT.

`host/plugin-host.ts:404`:
```ts
const pluginFn = mod.default ?? mod;
if (typeof pluginFn !== "function") { sendError(id, -32600, ...); return; }
```
It requires a **function**. omo's `dist/index.js` default-exports a `PluginModule` **object** `{ id, server }`.

Verified on the real deployed plugin file, not just `dist/`:

`~/.opencode/plugins/oh-my-opencode.js` — 118 bytes, a pure re-export shim:
```js
export { default } from "/Users/vakandi/EliaAI/setup/oh-my-openagent/dist/index.js";
```

Live probe of that module:
```
{"importMs":313,"rssAfterImportMB":87,"exportKeys":["id","server"],"isPluginModule":true}
```

So `mod.default` = `{ id, server }` → not a function → the host bails with `-32600`.

Reproduced through the **real** host shim:
```
$ bun plugin-host.ts → initialize(pluginPath=".../dist/index.js")
{ "code": -32600, "message": "Plugin module does not export a function" }
```

The TypeScript provider handles this correctly at `packages/opencode/src/plugin/index.ts:37`:
```ts
const plugin = readV1Plugin(load.mod, load.spec, "server", "detect");
if (plugin) { ...; hooks.push(await (plugin as PluginModule).server(input, load.options)); return; }
```
The Rust host has **no equivalent branch**. I wrote a patched host (`/tmp/ipcbench/patched-host.mjs`) adding `mod.server` / `mod.default.server` resolution; omo then loads cleanly → **14 hooks + 26 tools**.

**Independently corroborated:** `pragmatic-architect` captured `plugin.load_failed code=-32600` from a live run of the same artifact and reached the identical root cause by a different route.

### Blocker 2 — the tool registry is silently discarded
`host/plugin-host.ts:441-445` returns `{ name, hooks, auth }`. There is **no `tools` field**, and no `tool.execute` RPC. All 26 of omo's tools are unreachable — including every `team_*` tool, `call_omo_agent`, and `background_output`. There is also no bridge for `ToolContext.metadata()`, `.ask()`, or `.abort` (`packages/plugin/src/tool.ts:3-20`).

### Blocker 3 — the SDK client is a silent no-op. **This is the dangerous one.**
`host/plugin-host.ts:262-368 createPluginClient()` falls through to `createNoopClientProxy(["client"])` (`:242-260`) — a `Proxy` where every property access returns another Proxy and every call resolves `{}`.

I ran omo's setup with the byte-exact noop client from `plugin-host.ts:242-260`:
```
{"setupError": null, "hookCount": 14, "toolCount": 26}
```
**Setup succeeds with no error.** The breakage is therefore silent at startup and fatal at runtime. See §2 and §6.1.

### Blocker 4 — `event` is filtered out of the hook list
`host/plugin-host.ts:422`:
```ts
if (key === "auth" || key === "event" || key === "config") continue;
```
omo's entire event-driven state machine — ralph/ulw loop, session lifecycle, team handlers, model fallback — lives in `event` (`src/plugin/event.ts:116-222`).

### Also missing from the Rust hook mapping
`src/subprocess/mod.rs:25-42 hook_name_to_event` maps 13 hook names. Relative to omo's needs:
- `experimental.compaction.autocontinue` — **absent** (omo registers it; provider dispatches it at `packages/opencode/src/session/compaction.ts:500`)
- `dispose` — **absent** (provider calls it at `packages/opencode/src/plugin/index.ts:270` on finalizer)
- `config` — absent from `hook_name_to_event`, though `plugin-host.ts:465-473` special-cases its in-place mutation semantics correctly
- `event` — absent from `hook_name_to_event` *and* filtered at `:422`

---

## 1. THE PLUGIN API MAP (the contract)

### 1a. Contract definition — 22 hooks, one file

`/Users/vakandi/Documents/opencode-server-rust/packages/plugin/src/index.ts:222-335` — `interface Hooks`.

Entry types: `Plugin` at `index.ts:74`; `PluginModule` at `:76-80`; `PluginInput` at `:56-66`; `ToolContext` at `packages/plugin/src/tool.ts:3-20`; `ToolDefinition` at `tool.ts:54`; `tool()` factory at `tool.ts:45-53`.

`PluginInput` — what opencode **hands** the plugin (`packages/plugin/src/index.ts:56-66`):

| field | type | line |
|---|---|---|
| `client` | `ReturnType<typeof createOpencodeClient>` | :57 |
| `project` | `Project` | :58 |
| `directory` | `string` | :59 |
| `worktree` | `string` | :60 |
| `experimental_workspace.register` | `(type: string, adapter: WorkspaceAdapter) => void` | :61-63 |
| `serverUrl` | `URL` | :64 |
| `$` | `BunShell` | :65 |

**`$` is Bun-only.** The provider builds it at `packages/opencode/src/plugin/index.ts:89`:
```ts
$: typeof Bun === "undefined" ? undefined : Bun.$,
```
Plugins are *de facto* Bun programs. Any plugin host that is not Bun-hosted is second-class. omo's `dist/` additionally relies on Bun-specific APIs (`Bun.file`, `Bun.which`, `Bun.$` — e.g. `src/interactive-bash-availability.ts:8`).

### 1b. Every hook: contract line → provider dispatch site → does omo use it?

| # | Hook (contract line) | Provider dispatch site (consumer of the hook) | omo? |
|---|---|---|---|
| 1 | `tool` (`:226-228`) | `src/tool/registry.ts:201` — `Object.entries(p.tool ?? {})` | ✅ 26 tools |
| 2 | `event` (`:224`) | `src/plugin/index.ts:259` — `void hook["event"]?.({ event: { id, type, properties: event.data } })` — **fire-and-forget, NOT awaited** | ✅ |
| 3 | `config` (`:225`) | `src/plugin/index.ts:247` — `hook.config?.(cfg)` — **in-place mutation**; errors swallowed via `Effect.ignore` | ✅ |
| 4 | `auth` (`:229`; `AuthHook` `:88-163`) | provider auth bridge | ❌ |
| 5 | `provider` (`:230`; `ProviderHook` `:214-217`) | provider registry | ❌ |
| 6 | `chat.message` (`:234-243`) | `src/session/prompt.ts:999` | ✅ |
| 7 | `chat.params` (`:247-256`) | `src/session/llm/request.ts:114` | ✅ |
| 8 | `chat.headers` (`:257-260`) | `src/session/llm/request.ts:134` | ✅ |
| 9 | `permission.ask` (`:261`) | `src/session/processor.ts:372` (via the `permission.ask` service) | ❌ |
| 10 | `command.execute.before` (`:262-265`) | `src/session/prompt.ts:1460` | ✅ |
| 11 | `tool.execute.before` (`:266-269`) | `src/session/prompt.ts:307`; `src/session/tools.ts:106,175,258,338,402`; `src/tool/code-mode.ts:141` | ✅ |
| 12 | `tool.execute.after` (`:274-281`) | `src/session/prompt.ts:389`; `src/session/tools.ts:121,208,291,373,420`; `src/tool/code-mode.ts:180` | ✅ |
| 13 | `shell.env` (`:270-273`) | `src/session/prompt.ts:554`; `src/tool/shell.ts:417`; `src/server/routes/instance/httpapi/handlers/pty.ts:71` | ❌ omo / ✅ **proxy-local** |
| 14 | `experimental.chat.messages.transform` (`:282-290`) | `src/session/prompt.ts:1255`; `src/session/compaction.ts:379` | ✅ |
| 15 | `experimental.chat.system.transform` (`:291-296`) | `src/session/llm/request.ts:69`; `src/agent/agent.ts:381` | ✅ |
| 16 | `experimental.provider.small_model` (`:297`) | provider registry | ❌ |
| 17 | `experimental.session.compacting` (`:305-308`) | `src/session/compaction.ts:373` | ✅ |
| 18 | `experimental.compaction.autocontinue` (`:316-326`) | `src/session/compaction.ts:500` | ✅ |
| 19 | `experimental.text.complete` (`:327-330`) | `src/session/processor.ts:530` | ❌ |
| 20 | `tool.definition` (`:334`) | `src/tool/registry.ts:318` | ✅ |
| 21 | `dispose` (`:223`) | `src/plugin/index.ts:270` — `hook.dispose?.()` on finalizer | ✅ |
| 22 | **v2 API** (parallel surface) | `packages/plugin/src/v2/promise/context.ts:12-22` — `PluginContext` = 9 domains (`agent, aisdk, catalog, command, integration, plugin, reference, skill`); callback-registry type `Hooks<Spec>` at `v2/promise/registration.ts:9-11` | ❌ omo is v1-only |

**Dispatch mechanics (provider side):** `packages/opencode/src/plugin/index.ts:284-297` — `trigger(name, input, output)` iterates `state.hooks` in registration order, calls `fn(input, output)`, returns `output`. No priority, no short-circuit, no error isolation on the trigger path (each call is `Effect.promise(async () => fn(input, output))` at `:294`). Plugin state is scoped by `InstanceState.make<State>` at `:56` — **per instance, not per session.**

**Consumer side, verified by execution:**
- `packages/omo-opencode/src/plugin-interface.ts:36-104` assembles the v1 hooks
- `packages/omo-opencode/src/testing/create-plugin-module.ts:247-259` adds the two compaction hooks + `dispose`, returns `{ id: "oh-my-openagent", server: serverPlugin }` at `:263-266`
- `packages/omo-opencode/src/index.ts:1-8` — entry, `export default pluginModule`
- `PluginContext = Parameters<Plugin>[0]` at `src/plugin/types.ts:4` — confirms omo programs against exactly this `PluginInput`

Live probe output:
```
hookCount: 14
hookFunctionKeys: [chat.params, chat.headers, command.execute.before, chat.message,
  experimental.chat.messages.transform, experimental.chat.system.transform, config, event,
  tool.definition, tool.execute.before, tool.execute.after, experimental.session.compacting,
  experimental.compaction.autocontinue, dispose]
nonFunctionHookKeys: ["tool"]
toolCount: 26
tools: [background_cancel, background_output, call_omo_agent, glob, grep, interactive_bash,
  look_at, session_info, session_list, session_read, session_search, skill, skill_mcp, task,
  team_approve_shutdown, team_create, team_delete, team_list, team_reject_shutdown,
  team_send_message, team_shutdown_request, team_status, team_task_create, team_task_get,
  team_task_list, team_task_update]
```

**Coverage: 14/22 v1 hooks. Union with proxy-local = 15/22. Tools: 26 omo + 3 proxy-local = 29.**

### 1c. The host→plugin direction also carries a full SDK REST surface

omo calls the opencode client **~400 times across source** (`omo-opencode/src`, tests excluded), by frequency:

`client.session.messages` ×88 · `client.session.todo` ×42 · `client.session.get` ×30 · `client.session.summarize` ×29 · `client.session.status` ×22 · `client.session.promptAsync` ×22 · `client.session.abort` ×16 · `client.tui.showToast` ×9 · `client.trackActive` ×9 · `client.session.create` ×9 · `client.config.get` ×9 · `client.provider.list` ×8 · `client.app.agents` ×8 · `client.model.list` ×7 · `client.shutdown` ×3 · `client.session.list` ×3 · `client.session.message` ×2 · `client.session.children` ×2 · `client.app.skills` ×1 · `client.event.subscribe` ×1

**This is a REST client, not a helper, and it is the load-bearing dependency for three of Wael's four features.**

---

## 2. FEATURE → HOOK TABLE (Wael's four named features)

### 2a. unlimited-token AI → **this is `proxy-local`, NOT omo**

There is no "unlimited token" logic in omo. A search for `FreeUsageLimit|freeUsageLimit|unlimited` across `omo-opencode/src` returns exactly **one** hit, in `config/schema/background-task.ts`, and it is a schema field name, not behaviour.

The actual mechanism is `/Users/vakandi/Documents/opencode-proxy-plugin/proxy-local.ts`:

| Element | Location |
|---|---|
| plugin entry | `proxy-local.ts:1204` — `export const ProxyLocalPlugin: Plugin = async ({ client }) =>` |
| single-instance guard | `:1205-1209` — module-level `initialised` flag, refuses a second listener |
| rate-limit verdict + rotation | `:1302` `event` hook → `classifyProviderError(text)` at `:1360`, `runSwitchInBackground(...)` at `:1368` |
| watched events | `session.status` (skipped unless `/retry/i` matches, `:1358`), `session.error`, `session.updated` (agent cache, `:1332-1337`) |
| local CONNECT proxy | `:149` `http.createServer()`, `:164` `srv.listen(port, "127.0.0.1")` |
| **mutates host process env** | `:1261-1265` — `process.env.HTTP_PROXY / HTTPS_PROXY / NO_PROXY = local` |
| bash child env | `:1349-1356` `shell.env` hook |
| tools | `:1358` `proxy_status`, `:1379` `proxy_switch`, `:1400` `proxy_debug` |

**Port risk: LOW.** The "unlimited token" behaviour is (a) run a local HTTP CONNECT proxy on 127.0.0.1 tunnelling to a rotating upstream residential IP, (b) on a free-tier/rate-limit error verdict, rotate upstream, (c) advertise the local proxy to both the host's outbound HTTP client and bash children. `reqwest` honours `HTTP_PROXY`/`HTTPS_PROXY` natively. **This is ~600 LOC of Rust and should absolutely be ported natively.**

### 2b. ulw-loop / ralph-loop

| Element | Location |
|---|---|
| hook interface | `src/hooks/ralph-loop/ralph-loop-hook.ts:10-26` — `event` + `startLoop` / `resumeLoop` / `cancelLoop` / `getState` |
| construction | `hooks/ralph-loop/ralph-loop-hook.ts:44-70` |
| loop state | `hooks/ralph-loop/loop-state-controller.ts`, `storage.ts`, `session-reset-strategy.ts` |
| event handling | `hooks/ralph-loop/ralph-loop-event-handler.ts` |
| verification oracle | `plugin/tool-execute-before.ts:52-70` `buildUltraworkOracleVerificationPrompt` |
| **entry from a tool call** | `plugin/tool-execute-before.ts:102-127` — on `input.tool === "skill"`, parse `/ralph-loop` / `/ulw-loop`, then `hooks.ralphLoop.startLoop(...)` / `resumeLoop(...)` / `cancelLoop(...)` |
| keyword injection | `plugin/system-transform.ts` via `getUltraworkMessage` (`hooks/keyword-detector/ultrawork`) → `experimental.chat.system.transform` |
| continuation timing | `DEFAULT_IDLE_SETTLE_MS = 150` (`hooks/ralph-loop/constants.ts`), `DEFAULT_SESSION_IDLE_SETTLE_MS = 150` / `DEFAULT_SESSION_STATUS_TIMEOUT_MS = 5000` (`dist/index.js:7538`) |

**Hooks used:** `event` (primary — idle/finish detection), `tool.execute.before` (arg parsing + loop control), `chat.message` (first-message variant gate), `experimental.chat.system.transform` (keyword prompt injection).

**Subsystem reached:** the `event` bus, plus `session.promptAsync` to re-inject continuation turns into the session.

**Port risk: MEDIUM-HIGH.** It is a self-driving state machine that re-prompts the session over the SDK client. Needs `event` as a genuine notification channel and a working `session.promptAsync`.

### 2c. team mode (agent↔agent, not to the user)

| Element | Location |
|---|---|
| 11 `team_*` tools | `src/features/team-mode/tools/{lifecycle-create-tool,lifecycle-shutdown-tools,messaging,tasks,query}.ts` |
| registration | `src/plugin/tool-registry-team-tools.ts:21` `team_create`, `:41` `team_send_message`, `:42` `team_task_create` |
| **mailbox = filesystem** | `src/features/team-mode/team-mailbox/{send,inbox,poll,ack,reservation,pending-delivery-recovery}.ts` |
| **team state = filesystem + locking** | `packages/team-core/src/team-state-store/{store,locks,resume,session-liveness,runtime-cleanup,creating-resume,deleting-resume,reservation-reconciliation}.ts` — 78 files total in `team-core` |
| worktrees | `packages/team-core/src/team-worktree/{manager,cleanup}.ts` |
| tmux layout | `packages/team-core/src/team-layout-tmux/{layout,close-team-member-pane,rebalance-team-window,sweep-stale-team-sessions}.ts` |
| **live delivery = the only SDK dep** | `src/features/team-mode/tools/messaging-live-delivery-client.ts:1-15` requires exactly `client.session.promptAsync({ path:{id}, body:{ parts, agent, model, variant }, query:{directory} })` |
| delivery plumbing | `tools/messaging-live-delivery{,-recipient,-reservation,-state}.ts`, dispatched at `tools/messaging.ts:104` |
| **hook gating** | `plugin/tool-execute-before.ts:123` `teamToolGating` |

**Hooks used:** `tool` only (11 tools), plus `tool.execute.before` for gating.

**Port risk: HIGH.** The mailbox and the state machine are portable Rust (fs + fsync + locks — `team-core/src/tolerant-fsync.ts` exists for exactly this). **Live delivery dies instantly under a noop client**: messages land on disk and never reach the peer agent. That is precisely Wael's "agents talking to each other, not to the user" requirement failing silently.

### 2d. call_omo background tasks (without delegate_task)

| Element | Location |
|---|---|
| tool factory | `src/tools/call-omo-agent/tools.ts:111-225` (`createCallOmoAgent`) |
| **background path** | `src/tools/call-omo-agent/background-executor.ts` → `background-agent-executor.ts` → `src/features/background-agent/manager.ts` |
| **sync path** | `src/tools/call-omo-agent/sync-executor.ts` |
| session creation | `src/tools/call-omo-agent/session-creator.ts`, `subagent-session-creator.ts` |
| **completion polling** | `src/tools/call-omo-agent/completion-poller.ts` — polls `client.session.messages` |
| message processing | `src/tools/call-omo-agent/message-processor.ts` |
| sibling tools | `background_cancel`, `background_output` |

**Hooks used:** `tool` only.

**Subsystem reached:** `session.create` → `session.promptAsync` → poll `session.messages` → return; `session.abort` to cancel.

**Port risk: HIGH.** This is the full create-prompt-poll-return cycle over the SDK. Under a noop client it "succeeds" and returns garbage.

### 2e. Consolidated

| Feature | Hook(s) | Subsystem | Risk |
|---|---|---|---|
| unlimited-token AI | `event`, `shell.env`, `tool` (**proxy-local only**) | event bus + host HTTP client env | **LOW → port natively** |
| ulw-loop | `event`, `tool.execute.before`, `chat.message`, `experimental.chat.system.transform` | event bus + `session.promptAsync` | MEDIUM-HIGH |
| team mode | `tool` (11 tools) + `tool.execute.before` | `session.promptAsync` + filesystem mailbox | HIGH |
| call_omo | `tool` | `session.create/promptAsync/messages/abort` | HIGH |

**Conclusion of §2: three of four features are gated on the SDK client, not on hooks. Any host that stubs `client` satisfies the letter of the API and destroys the substance.**

---

## 3. THE ARCHITECTURAL DECISION

### Option (a) Embedded JS engine — V8 / QuickJS / Boa / deno_core

**Memory.** omo's module graph costs **~78 MB of JS heap**: bare bun baseline **8.9 MB** (measured 3×: 9,338,880 B), omo import **87.4 MB RSS** (91,652,096 B peak), after `plugin()` setup **106 MB**, under live hook traffic **111 MB**.
- deno_core / V8, **shared** isolate: 78 MB once. Acceptable.
- deno_core / V8, **per-session** isolate: **100 × 78 MB = 7.8 GB** → blows the entire 8 GB budget on plugin heap alone. Per-session isolation is not affordable.
- A shared isolate means shared module-level mutable plugin state (`internalMarkerCache` at `src/plugin/chat-headers.ts:19`; `initialised` at `proxy-local.ts:1205`). **That matches upstream semantics**, which scopes plugin state per *project instance* via `InstanceState.make<State>` (`packages/opencode/src/plugin/index.ts:56`), not per session.

**CPU.** deno_core/V8 is adequate. **QuickJS is disqualified** — 10-50× slower than V8, and omo ships zod v4 + `effect` + the MCP SDK, which assume a modern runtime. **Boa is disqualified** — not production-ready.

**Effort.** High. Every npm dependency (zod, effect, MCP SDK, picomatch, jsonc-parser, js-yaml, commander, posthog-node) must resolve inside the engine.

**Verdict: viable but strictly worse than (c).** Same memory when shared, catastrophic when isolated, more work.

### Option (b) Reimplement omo natively in Rust, thin TS shim

**Memory: ~0.** 26 tools + 14 hooks in Rust ≈ tens of MB. The big win.

**Effort: the killer.** The 14 hooks fan out to **56 internal guard components**:

| Factory | Components | File |
|---|---|---|
| `createSessionHooks` | 23 | `src/plugin/hooks/create-session-hooks.ts` |
| `createToolGuardHooks` | 17 | `src/plugin/hooks/create-tool-guard-hooks.ts` |
| `createTransformHooks` | 7 | `src/plugin/hooks/create-transform-hooks.ts` |
| `createContinuationHooks` | 7 | `src/plugin/hooks/create-continuation-hooks.ts` |
| `createSkillHooks` | 2 | `src/plugin/hooks/create-skill-hooks.ts` |

spanning 20 workspace packages, plus tmux integration, an **LSP client** (`src/features/comment-checker/cli.ts:1,149-150` spawns a language server), a team state machine with fsync/locking/resume, a model-fallback chain resolver, and a rule-injection engine with `readFileSync`/`statSync` caching (`src/hooks/rules-injector/parsed-rule-cache.ts:1`).

**The decisive objection is not effort — it is drift.** omo is actively developed (`CHANGELOG.md` 21 KB; `dist/` rebuilt Sep 29; `omo-opencode/package.json` version `0.1.0`, private, actively churning). A native port is a permanently-diverging fork, already behind on day one. Wael's constraint is *every feature must FULLY WORK* — a lagging fork cannot guarantee that, whereas re-hosting the real artifact can.

**Verdict: correct for `proxy-local` only** (~600 LOC, 2 hooks + 3 tools, no zod/effect — imports are only `node:http, node:net, node:fs, node:os, node:crypto` per `proxy-local.ts:1-7`). **Loser for omo.**

### Option (c) Out-of-process IPC, reimplement only the hook protocol ✅

Already the reference's choice: `crates/opencode-plugin/src/subprocess/{protocol,client,loader,runtime,auth}.rs` + `host/plugin-host.ts`. Content-Length framed JSON-RPC 2.0 over stdio; `loader.rs:19` embeds the host via `include_str!` and writes it to `~/.cache/opencode/plugin-host.ts`; `loader.rs:268-301` handles npm installs.

**Memory — the decisive number: O(1) in sessions, not O(n).** One bun process: **87 MB** after import, **106 MB** after setup, **111 MB** under live traffic. Independent of session count. **100 sessions = 106 MB.** Plus proxy-local, dependency-light (≈35-50 MB). Total ≈ **150 MB of the 8 GB budget — under 2%.** This is the entire argument.

**CPU/latency — IPC is NOT the bottleneck.** Measured round-trips through the real host shim over the real framing:

| Hook | Payload | Round-trip |
|---|---|---|
| `chat.headers` | small | **0.097 ms** |
| `tool.execute.before` | small | **0.12-0.30 ms** |
| `event` | small | **0.6-1.1 ms** |
| `experimental.chat.messages.transform` | **1.75 MB** (200 msgs × 3 parts × 3 KB) | **6.87 ms** |

Compare against the hook's *own* work: **~60 ms** (§6.2). IPC is 0.1-1% overhead — three orders of magnitude below the cost it would be replacing.

**Isolation bonus.** An omo segfault or a bad `import()` cannot kill the Rust server; the supervisor restarts the host. In-process embedding gives neither.

**Effort.** Lowest of the four, and mostly already done. The remaining work is fixing 4 blockers, not building a mechanism.

### Option (d) WASM

**Fatal.** omo needs `Bun.$` (shell), `node:child_process` (spawn), `node:fs`, `node:crypto`, `node-pty`, the MCP SDK, and heavy async/await. WASI preview-2 has **no process spawn** — every spawn would need proxying back to the host. `asyncify`/stackful-fiber support is required for the async surface and is fragile. zod/effect would compile; the MCP SDK realistically would not.

**Verdict: loser.** Highest risk, no memory advantage over (c).

### 🏆 COMMITMENT

**Option (c) — one long-lived `bun` process per plugin, plugins out-of-process, hook protocol reimplemented, and the SDK client exposed as a real HTTP client pointed at the Rust server's own REST API.**

- **Per-session cost: 0 MB.**
- **Per-plugin cost: ~106 MB (omo) + ~40 MB (proxy-local), paid once.**
- **100-session budget: ~150 MB / 8 GB = 1.9%.**

**Named loser: option (b), full native Rust reimplementation of omo** — not on effort, but on drift: it guarantees a permanently-diverging fork of a fast-moving upstream, directly violating Wael's "every feature must FULLY WORK" constraint. Option (b) *for proxy-local alone* is a strict win and should be done.

**Runner-up loser: (a) embedded engine** — identical memory when shared, catastrophic when isolated, strictly more work.

### Required changes to make (c) actually work

1. **Resolve `PluginModule` shape** — mirror `readV1Plugin` (`packages/opencode/src/plugin/index.ts:37`): accept `mod.server`, `mod.default.server`, `mod.default` (function), `mod` (function). *Fixes Blocker 1.*
2. **Return `tools` from `initialize`** with zod→JSON-Schema conversion, and add a `tool.execute` RPC bridging `ToolContext.metadata()`, `.ask()`, `.abort`. *Fixes Blocker 2.*
3. **Guarantee a real SDK client; fail loudly if unresolvable — never fall back to the noop proxy silently.** *Fixes Blocker 3, and is the single most important change.*
4. **Add `event` as a host→plugin notification**, fire-and-forget, matching `void hook["event"]?.(...)` at `packages/opencode/src/plugin/index.ts:259`. Also add `experimental.compaction.autocontinue` and `dispose` to `hook_name_to_event`. *Fixes Blocker 4.*
5. **Drop `node` from the fallback chain or require ≥22.6** — `runtime.rs:28-31` uses `--experimental-strip-types`, which needs Node ≥22.6; this machine has **v20.20.2**, so that branch is dead here.
6. **Budget the serial plugin-load at startup.** `pragmatic-architect` measured up to **30 s** per plugin load on a warm `dist` import plus a 20 s process spawn allowance. Since plugin load gates the server's port bind, this is on the critical path.
7. **Add a compact binary codec or fetch-by-reference for `messages.transform`.** See §8.5.

---

## 4. Embedded-engine spec (recorded for completeness; NOT recommended)

- **Engine:** `deno_core` (V8) — the only candidate that runs omo unmodified. QuickJS: 10-50× slower, no modern stdlib. Boa: not production-ready.
- **Isolate mapping:** one **shared** V8 isolate + one omo instance per *project instance*, mirroring `InstanceState` (`packages/opencode/src/plugin/index.ts:56`). **Never per-session** — that is 78 MB × 100 = 7.8 GB.
- **MB per isolate:** empty V8 isolate ≈1-3 MB; omo's module graph forces the isolate heap to **~78 MB**, per-isolate, unavoidable.
- **Confidence:** high on the engine ranking; **medium** on the MB figures (single machine, single sample, no isolate-level instrumentation).

---

## 5. EXERCISED vs DEAD WEIGHT

### Exercised — must keep
- **15/22 v1 hooks** (14 omo + `shell.env` for proxy-local).
- **29 tools** (26 omo + `proxy_status`, `proxy_switch`, `proxy_debug`).
- **~400 SDK client call sites** — the real dependency.
- **Bun as the runtime** (hard requirement, `packages/opencode/src/plugin/index.ts:89`).

### Dead weight — safe to skip
- **7 hooks:** `permission.ask`, `auth`, `provider`, `experimental.provider.small_model`, `experimental.text.complete`, plus 2 of the 3 non-omo hooks.
- **The entire v2 API** — `packages/plugin/src/v2/{promise,effect}/*`, 9 domains, Effect/Promise duality. omo never touches it. Only relevant if another of Wael's plugins uses it (unverified — §8.6).
- **24,860 of 27,425 files.** Only **4,495** `.ts` files sit outside `node_modules/` + `dist/`. Of those, `omo-opencode` alone is **2,574**.
- **11 prebuilt platform binaries** — `packages/oh-my-opencode-{darwin,linux,windows}-{arm64,x64}[-musl|-baseline]` — not needed on the server path.
- **`packages/omo-codex` (566 MB)** — a separate product (Codex CLI integration), not on the opencode plugin path. Also excluded: `lsp-tools-mcp` (110 MB) and `lsp-daemon` (110 MB), which are standalone binaries, though note §6.2 — the comment-checker LSP behaviour *is* on the hot path.
- **56 internal guard components** collapse into the 14 hook functions. A native port would reimplement *behaviour*, not files.

---

## 6. Two performance findings that matter for the port decision

### 6.1 The noop-client trap (restated as the #1 validation risk)
omo's setup under the byte-exact `plugin-host.ts:242-260` noop client: **14 hooks, 26 tools, no error.** Any smoke test that checks "did the plugin load and register its tools" will **pass** while team mode, call_omo, and ulw-loop are all dead at runtime. A validation harness must assert on a **real** `session.promptAsync` round-trip, not on registration.

### 6.2 🔥 The 60 ms per-tool-call tax — the best news in this report
`tool.execute.before` measured **68.8 / 58.9 / 62.7 / 60.0 ms** on four consecutive serialized calls, and **0.12-0.30 ms** when 20 run concurrently. Concurrency collapses the cost ⇒ **it is I/O wait, not CPU.**

- Ruled out: any `setTimeout` in the 40-90 ms band — instrumented `globalThis.setTimeout`, **0 hits**.
- Cause: `src/plugin/tool-execute-before.ts:106-123` fans out **sequentially** to 17 guards, including `directoryAgentsInjector` (`:113`), `directoryReadmeInjector` (`:114`), `rulesInjector` (`:115`) — all re-reading AGENTS.md / README.md / rule files from disk per call (`src/hooks/rules-injector/parsed-rule-cache.ts:1` imports `readFileSync, statSync`) — plus `commentChecker` (`:112`), which **spawns a language server** (`src/features/comment-checker/cli.ts:1,149-150`).

Full fan-out list with line numbers: `tool-execute-before.ts:106` `writeExistingFileGuard`, `:107` `notepadWriteGuard`, `:108` `questionLabelTruncator`, `:109` `claudeCodeHooks`, `:110` `nonInteractiveEnv`, `:111` `bashFileReadGuard`, `:112` `commentChecker`, `:113` `directoryAgentsInjector`, `:114` `directoryReadmeInjector`, `:115` `rulesInjector`, `:116` `tasksTodowriteDisabler`, `:117` `webfetchRedirectGuard`, `:118` `fsyncSkipWarning`, `:119` `prometheusMdOnly`, `:120` `sisyphusJuniorNotepad`, `:121` `atlasHook`, `:122` `compactionTodoPreserver`, `:123` `teamToolGating`.

**Consequence: a Rust port with a cached VFS and a long-lived LSP connection turns a 60 ms per-tool-call penalty into microseconds. The Rust port can be materially faster than the TypeScript original on the hottest path in the system** — the opposite of the usual "porting to Rust is slower" expectation, and a concrete argument that the port pays for itself.

---

## 6b. Can omo tolerate LATE / DEGRADED plugin binding? — YES, and it self-heals

Answering `pragmatic-architect`'s question directly, because their proposed startup fix (load plugins *after* the HTTP listener binds, concurrently) depends on it. **The answer is yes — omo's setup never throws, never blocks, and never awaits the server.**

**Setup path is I/O-free w.r.t. the server:**
- `initLiveServerRoute({ serverUrl, ... })` — `src/shared/live-server-route.ts:65-86` — merely *stores* the URL, sets `probeTimestamp: 0`, and logs `hasServerUrl` (`:84`). No network call.
- `warmLiveServerProbe()` — `:89-91` — is `void probe(lastRegistration)`, i.e. **explicitly fire-and-forget, not awaited**.
- Auth is env-var based and resolved **per request**, not at setup: `getServerBasicAuthHeader()` at `src/shared/opencode-server-auth.ts:9-18` reads `OPENCODE_SERVER_PASSWORD` and returns `undefined` when unset.
- Called from `create-plugin-module.ts:150` (`initLiveServerRoute`) and `:152` (`warmLiveServerProbe`) — neither is awaited.

**Every server touchpoint degrades safely rather than throwing:**

| Site | Behaviour on missing / dead server |
|---|---|
| `probe()` `:96` | early-return if `!serverUrl` |
| `probe()` `:106-110` | `AbortController` bounded by `PROBE_ABORT_MS = 1_500` (`:10`) |
| `probe()` `:134-138` | `catch` → `available = false`, `probeTimestamp = Date.now()`, **swallowed** |
| `probe()` `:121-129` | 401/403 → warn once, "live wake routing disabled" |
| `probeSessionAffinity()` `:149` | early-return `undefined` if `!serverUrl` |
| `probeSessionAffinity()` `:177-179` | `catch` → `undefined` — **transient failures KEEP the live route** |
| `probeSessionAffinity()` `:171-175` | only a definite **404** demotes to the in-process client |

The intent is documented in-source at `:143-147`: *"Only a definite 404 demotes the route: transient failures keep the live route so serve-topology runner-split protection is not lost."* (issue #5569).

**It is self-healing.** `PROBE_TTL_MS = 60_000` (`:9`). `resolveAvailability()` at `:224-237` re-probes whenever the cached result is older than the TTL (`:218`, `:230-233`). And the consumer `tryResolveDispatchClientSync()` at `:252-271` returns `undefined` — meaning *"unknown, fall back in-process"* — when the probe is stale, rather than a hard negative. So it fails **safe toward in-process, never toward dropping a wake.**

**⇒ Conclusion: `pragmatic-architect`'s concurrent-after-bind plugin load is SAFE for omo, and is a net win** — it makes the very first probe succeed instead of depending on the 60 s TTL re-probe.

**One caveat to state honestly:** `warmLiveServerProbe()` is one-shot at setup, so with late binding there is a window (up to `PROBE_TTL_MS` = 60 s, in practice one dispatch interval) where team-mode live delivery takes the **in-process** fallback instead of the HTTP route. Nothing is lost — the mailbox still persists to disk (`features/team-mode/team-mailbox/`) — so delivery degrades to the slower path and self-corrects. Worth asserting in a test rather than assuming.

---

## 6c. ✅ END-TO-END VALIDATION — the architecture works, and the blocker list is now exact

> **⚠️ CORRECTION (self-audited).** An earlier revision of this section claimed the stdin transport was *deterministically* broken and that my fix "verified" it. **That was overstated.** The unmodified host loads real omo and the fixture correctly. The hang is an **intermittent, load-dependent lost-wakeup race**, not a hard failure. See §6c.4 for the corrected mechanism and what it changes.

`pragmatic-architect` reported 30 s plugin-load timeouts and hypothesised that **bun never fires `"readable"` on `process.stdin`**, hanging `host/plugin-host.ts:85-113 readExact()`. I investigated and **their stated mechanism is wrong**, though the symptom is real. **I also initially misdiagnosed it myself** — both errors are recorded below rather than quietly overwritten.

### 6c.1 The stated mechanism is false — `"readable"` does fire
Measured directly, piping a payload and instrumenting the event:

```
[bun/plain]        AFTER-WAIT readable=true      [deno/plain]       AFTER-WAIT readable=true
[bun/setEncoding]  AFTER-WAIT readable=true      [deno/setEncoding] AFTER-WAIT readable=true
{"firstRead":"null","readableFired":true,"elapsedMs":403}   ← bun
{"firstRead":"null","readableFired":true,"elapsedMs":403}   ← deno
```

- `"readable"` **fires on both bun and deno**.
- **`setEncoding` is irrelevant** — byte-identical behaviour with and without it. That question is closed.
- The telling detail: **`read(1)` returns `null` while `"readable"` fires and data is available.**

Side finding that confirms my earlier claim: `node --experimental-strip-types` → `node: bad option`. **`subprocess/runtime.rs:28-31`'s node fallback is dead on this machine** (v20.20.2; the flag needs ≥22.6).

### 6c.2 The real mechanism — a lost-wakeup race
`plugin-host.ts:96-103`:
```ts
const chunk = reader.read(remaining);
if (chunk !== null) { ... } else {
  await new Promise<void>(resolve => reader.once("readable", resolve));   // ← attached AFTER read() returned null
}
```
`readMessage()` then loops calling `readExact(1)` **byte-by-byte** for the header. If `"readable"` fires *before* the `once()` listener is attached, the wakeup is **lost**: the data is already buffered, no new data will arrive to re-emit the event, and the `once()` never resolves. **Intermittent and load-dependent** — a cold bun under parallel load widens the window, which is exactly why it looked deterministic in a tight test loop.

Isolated A/B (both patterns, warm): pattern A (current shape) `got 1B after 2ms`; pattern B (listener attached before read) `got 1B after 3ms` — **both pass warm**. That is the signature of a race, not a missing event.

### 6c.3 The 2×2 that settles it — my first "TIMEOUT" was my own harness
```
ORIGINAL (unmodified) host + REAL omo      -> -32600  (fast)
ORIGINAL (unmodified) host + echo fixture  -> hooks=["chat.headers","tool.execute.before"]
FIXED host + echo fixture   (3 attempts)   -> 3/3 pass
FIXED host + REAL omo        (2 attempts)   -> 2/2 -32600
```
**The pristine host works.** My initial "ORIGINAL host + fixture → TIMEOUT 8000ms" was a cold-start artifact of my own `repro.mjs` harness, which I then misattributed to the host. The fixture also passes 3/3 on the *fixed* host, so the "fixture anomaly" I flagged as an open item **does not exist** — it was the same cold-start artifact.

Also note my first fix attempt (`for await` *inside* `readExact`) genuinely did break things — it drains the stream per call — but that was a bug I introduced, not evidence about the original.

### 6c.4 What actually stands
1. **The transport race is real but intermittent**, and `pragmatic-architect`'s 30 s timeouts on the *builtin auth* plugins are genuine instances of it. Their inference that **"the host has likely never completed a handshake with any plugin" is too strong** — omo and the fixture both load cleanly on a warm unmodified host.
2. **A deterministic fix is still the right call.** Replacing the `once()`-after-`read()` handshake with **one long-lived async iterator feeding a shared buffer** removes the race by construction — there is no listener to lose. That part of my §6c recommendation stands and is the correct implementation, just not for the reason I first gave.
3. **The blocker list is unchanged at 6** — the race was never one of my six; my six were always the API-surface blockers. Three are fixed and verified (§6c.5), three remain open.

### 6c.5 Three surgical fixes → real omo loads over the wire
Applied to `/tmp/ipcbench/host-full.ts`:
1. `readExact` → single shared async iterator + shared buffer (removes the race deterministically)
2. `mod.default ?? mod` → also accept `mod.server` / `mod.default.server`, mirroring `readV1Plugin` at `packages/opencode/src/plugin/index.ts:37`
3. handshake result gains `tools: Object.keys(hooks.tool ?? {})`

Result:
```
HANDSHAKE OK
  hooks: 12  [chat.params, chat.headers, command.execute.before, chat.message,
              experimental.chat.messages.transform, experimental.chat.system.transform,
              tool.definition, tool.execute.before, tool.execute.after,
              experimental.session.compacting, experimental.compaction.autocontinue, dispose]
  tools: 26   (incl. all 11 team_*, call_omo_agent, background_cancel, background_output)
  chat.headers over the wire -> {"output":{"headers":{}}}
```

**`hooks: 12`, not 14** — `plugin-host.ts:422` filters out `auth`, `event`, `config`. Blocker 4, now measured: the two silently dropped hooks are **`event` and `config`** — precisely the ralph/ulw-loop state machine and the config mutation hook.

**`chat.headers` returns `{headers:{}}` — unchanged.** Blocker 3, now **observable on the wire**: `chat-headers.ts:56-80` calls `client.session.message(...)` for the internal-initiator marker, the noop client returns `{}`, so the Copilot header is never injected. The hook *runs* and produces *nothing*.

### 6c.6 🔑 Simplification: the Rust `HookEvent` enum is NOT on the critical path
`subprocess/client.rs:215-226 invoke_hook(hook: &str, ...)` sends the hook **name as a string**, and `plugin-host.ts:456` dispatches via `pluginHooks[params.hook]` — a string-keyed lookup. `hook_name_to_event` (`subprocess/mod.rs:25-42`) is therefore **not required for the IPC path**; new hooks need **no new `HookEvent` variant**. Three items off the fix list.

### 6c.7 Definitive blocker list
| # | Blocker | Location | Status |
|---|---|---|---|
| 1 | `PluginModule` export shape rejected | `plugin-host.ts:404` | **FIXED + verified** |
| 2 | tool registry discarded | `plugin-host.ts:441-445` | **FIXED + verified** |
| 3 | noop SDK client → features silently dead | `plugin-host.ts:242-260,367` | open — **must fail loudly, never silently** |
| 4 | `event` + `config` filtered out | `plugin-host.ts:422` | open — `event` needed as fire-and-forget notification |
| 5 | no `tool.execute` RPC; `ToolContext.metadata()/.ask()/.abort` unbridged | absent | open — tools register but cannot be invoked |
| 6 | node fallback dead (`--experimental-strip-types` needs ≥22.6) | `subprocess/runtime.rs:28-31` | open — drop node or gate on version |
| — | stdin `once("readable")` lost-wakeup race (**intermittent**) | `plugin-host.ts:96-103` | fix recommended; deterministic fix verified, but **not deterministic-bug-blocking** — see §6c.2 |

Blockers 3-5 are what stand between "omo registers 26 tools" and "team mode / call_omo actually work".

---

## 7. Corroboration from other team members

- **`pragmatic-architect`** root-caused the 30 s plugin-load timeout to **bun never firing `"readable"` on `process.stdin`**, hanging `readExact` (`plugin-host.ts:85-113`) before `initialize` is dispatched — and observed that the deno path completes in 0.18 s. **Their diagnosis is correct and I reproduced it** (§6c). Their other measurements stand: plugin load up to 30 s serial, `dist/index.js` 84 MB with no source maps, ~50 npm deps resolved from `~/.config/opencode/node_modules`, and the 12 Rust crates covering ~10% of TS behaviour with no HTTP server. Their "two halves" framing matches §3. Their open question — can omo tolerate late/degraded binding? — is answered **yes, self-healing** (§6b). Their `crates/opencode-server/src/server.rs:481-489` finding (plugin load inside the state await, before the listener binds at `:489`) is consistent with their 70-90 s bind time.
- **`feasibility-skeptic`** raised the per-session JS-engine cost attack. §3(a) answers it with the arithmetic: a per-session isolate is 78 MB × 100 = 7.8 GB, which is fatal — but the out-of-process host is **O(1)**, which is why (c) wins rather than (a).

---

## 8. CLAIMS I COULD NOT VERIFY

1. **`proxy-local` RSS — unmeasured.** Its probe never returned (it opens a listener and a 15 s poll loop that blocks process exit). The omo figures (87/106/111 MB) are solid; the proxy-local estimate (~35-50 MB) is **inferred from its import list** (`proxy-local.ts:1-7` — `node:http, node:net, node:fs, node:os, node:crypto` only, no zod/effect/MCP), not observed. Needs a clean measurement before the memory budget is signed off.
2. **Multi-session plugin state sharing — the biggest open risk to my own recommendation.** I ran omo with a single synthetic `PluginInput`. Whether `InstanceState` (`packages/opencode/src/plugin/index.ts:56`) is per-project or per-session in a real 100-session deployment is unresolved, and it determines whether one shared bun host is safe. Needs a real multi-session test.
3. **omo under `deno` and `node`.** `runtime.rs:28-31` gives node `--experimental-strip-types`, requiring **Node ≥22.6**; this machine has **v20.20.2**, so that fallback is broken here. I validated **bun only**.
4. **Per-guard attribution for the 60 ms.** I proved it is I/O-bound and located the 17-guard fan-out, but did not isolate which guard dominates. `commentChecker`'s LSP spawn (`comment-checker/cli.ts:1`) is the prime suspect, unconfirmed. My `child_process` monkey-patch trace returned no output — bun hoists ESM imports, so the patch did not intercept.
5. **`messages.transform` payload scalability.** Measured at 200 messages / 1.75 MB → 6.87 ms. At a 2,000-message pre-compaction context (~17 MB) this extrapolates to ~60-70 ms **per LLM request**, needing a compact binary codec or a fetch-by-reference protocol. Untested.
6. **v2 API usage by other plugins.** I confirmed *omo* does not use it. I did not verify whether any of Wael's other plugins do.
7. **`dist/` vs `src/` as the shipped artifact.** I tested `dist/index.js` (via the `~/.opencode/plugins` shim, which points at it). Note a latent risk: `omo-opencode/package.json` pins `@opencode-ai/plugin` **1.15.13** while the server repo is at **1.18.34** (`packages/plugin/package.json`). That version skew is unexamined and may itself be a break.
8. **`omo-codex` (566 MB) scope.** Assumed out of scope for the opencode plugin path. If any of Wael's usage routes through it, the 87 MB figure is wrong by an order of magnitude.
9. **TUI plugin surface.** omo writes a `tui.json` entry (`create-plugin-module.ts:141-149` → `ensureTuiPluginEntry`) and ships `src/tui.ts`; `proxy-local` ships a separate TUI plugin (`tui-switch-proxy.tsx`, `SolidPlugin` from `@opentui/solid`). I mapped the **server-side** plugin API only. The TUI plugin surface (`packages/plugin/src/tui.ts`, `TuiPlugin`/`TuiPluginApi`, `src/plugin/tui/runtime.ts:593`) is a **separate contract in a separate process** and is unexamined — relevant to task 10's TUI-severance work.
