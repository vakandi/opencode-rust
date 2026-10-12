# TS ANATOMY — structural map of the opencode TypeScript server
**Member:** ts-anatomy · **Round 1 findings** · **Subject:** `/Users/vakandi/Documents/opencode-server-rust`
(`anomalyco/opencode` server-only, **v1.18.34**, 15 packages, 146,127 measured LOC in `packages/*/src`)

**Contents:** §0 headline · §1 session lifecycle · §2 HTTP API surface · §3 tool system · §4 permission system ·
§5 agent+skill · §6 config/project/worktree · §7 storage · §8 plugin contract (server side) ·
§9 TS→Rust correspondence · CLAIMS I COULD NOT VERIFY · Appendix A: TUI-decoupling mini-plan (task 10)

---

## EXECUTIVE SUMMARY — the six findings that should change how the team argues

1. **The tree is not the codebase the consumer targets.** `opencode_client.py:4` self-identifies as v1.18.18;
   this is v1.18.34 and the server was rewritten onto Effect-TS `HttpApi`. **Three** API generations coexist. The
   **live** router is `packages/opencode/src/server/routes/instance/httpapi/**` (mounted `server.ts:276-284`) —
   *not* `packages/server`. A plan that reads only `packages/server` ports the wrong API. (HIGH)

2. **All SQL in the process serializes through one synchronous handle.** `Semaphore.make(1)`
   (`core/src/database/sqlite.node.ts:110`) over `node:sqlite` `DatabaseSync`, whose `statement.all()` (`:63`)
   is a synchronous C call on the JS main thread. No worker, no pool, one global DB file
   (`core/src/database/database.ts:43-55`), `PRAGMA cache_size = -64000` = 64 MB page cache on that one
   connection (`:30`). This is the #1 structural constraint on the 8 GB / 100-session question.

3. **Persistence is event-sourced and every durable event is a `BEGIN IMMEDIATE` transaction.**
   `Session.updatePart`/`updateMessage` **never touch SQLite** — they only publish events
   (`session/session.ts:629-643`; `structuredClone(part)` per update at `:639`). Writes happen in a projector
   (`core/src/session/projector.ts:310-328`: SELECT + INSERT..ON CONFLICT + usage). Every session event is
   `durable: {aggregate:"sessionID", version:1}` (`schema/src/v1/session.ts:501-504`), so each routes through
   `commitDurableEvent` = **one `db.transaction(..., {behavior:"immediate"})`, ~6 statements**
   (`core/src/event.ts:237-363`). ⇒ **~35-55 SQL statements per assistant turn.** And the `event` table has
   **no retention**: rows are removed only by `ON DELETE CASCADE` on *session* delete. It is append-only for
   the life of the DB file. (`message.part.delta` is the one exception — live-only, no DB.)

4. **The consumer has a live bug in its completion detection.** `session/status.ts:39-48` **deletes** the map
   entry on idle rather than storing `{"type":"idle"}`, and status is `InstanceState` keyed by directory
   (`:26-28`). `opencode_client.py:240-241` returns `"idle"` *by absence*, and `wait_for_response` calls
   `get_session_status` with **no directory argument** (`:543`, `:557`) ⇒ falls back to `process.cwd()`, not the
   session's directory. **A running session can report idle.** Independent of the rewrite.

5. **There is no permission-based filter for built-in tools.** `ToolRegistry.tools()`
   (`tool/registry.ts:291-340`) filters only websearch availability, the `apply_patch` vs `edit/write`
   model-family switch (`:297-300`), and code-mode visibility. A `deny` on `bash` does **not** remove it from
   the model's tool list — enforcement is each tool's own opt-in `ctx.ask`, **with no safety net**. Tool
   dispatch itself lives inside AI SDK (`session/llm.ts:318` passes `tools: prepared.tools` to `streamText`).

6. **The Rust reference is not a port of three decisive things** (§9.4/§9.5): event-sourced persistence
   (Rust is direct-write CRUD over 6 tables, `sqlx`+`SqlitePool` — which *fixes* finding 2 but loses
   replay/idempotency/audit); **multi-directory instances** (no `InstanceStore`/`InstanceState`/
   `WorkspaceRoutingMiddleware` anywhere in the 19-crate list ⇒ **`x-opencode-directory` has no port**, and the
   consumer depends on it); and in-process plugins (Rust = JSON-RPC subprocess to external bun/deno/node, with
   **no embedded JS engine in the resolved dependency graph** — definitive negative).

---

## ⚠️ 0. THE HEADLINE FINDING — this is NOT the codebase the consumer targets

The consumer at `/Users/vakandi/EliaAI/subworkers/server/app/utils/opencode_client.py:4` documents itself as
`VERIFIED endpoints (server v1.18.18)`. This source tree is **v1.18.34**, and its server was **rewritten onto
Effect-TS `HttpApi`**. Both generations are present:

- **Legacy v1 API** — still shipped as *generated SDK types only*, not as a live router:
  `packages/sdk/js/src/gen/sdk.gen.ts:461` (`url: "/session/status"`),
  `packages/sdk/js/src/gen/types.gen.ts:2118`.
- **Live router** — `packages/opencode/src/server/routes/instance/httpapi/**` (Effect `HttpApiBuilder`),
  mounted by `packages/opencode/src/server/routes/instance/httpapi/server.ts:276-284`.
- **A third, parallel "v2" API** — `packages/server/src/routes.ts:39-63` + `packages/server/src/handlers/session.ts:19`
  with completely different operation ids (`session.list`, `session.create`, `session.prompt`, `session.history`,
  `session.events`, `session.active`) and a different session store (`SessionV2`, `@opencode-ai/core/session`).
  Mounted separately as `serverRoutes` (`server.ts:177-181`).

**Consequence for the team:** any port plan that reads only `packages/server` will port the wrong API.
The endpoints the Python consumer actually calls are served by the **`instanceApiRoutes`** tree in
`packages/opencode`, not by `packages/server`. Confidence: **HIGH** (verified by path constants + handler wiring).

### 0.1 The three state generations coexist in the DB
`packages/core/src/database/migration/` contains 40 migrations dated `20260127` → `20260622`, including
`20260622170816_reset_v2_session_state.ts`. The schema carries **both** a legacy event-logged V1 model and a V2
event-sourced model:

| table | holds | defined at |
|---|---|---|
| `session` | session row | `packages/core/src/session/sql.ts:22` |
| `message` | V1 message, **whole body in one JSON `data` blob** | `sql.ts:68` |
| `part` | V1 part, **whole body in one JSON `data` blob** | `sql.ts:82` |
| `todo` | todos | `sql.ts:100` |
| `session_message` | V2 event-sourced messages | `sql.ts:119` |
| `session_input` | durable prompt inbox (admitted/promoted seq) | `sql.ts:140` |
| `session_context_epoch` | compaction baselines | `sql.ts:168` |
| `event` / `event_sequence` | **append-only event log** | `packages/core/src/event/sql.ts:5,12` |

---

## 1. SESSION LIFECYCLE — the spine, hop by hop

### 1.1 `x-opencode-directory` → workdir (this is the load-bearing undocumented contract)

**Hop A — route registration.** `packages/opencode/src/server/routes/instance/httpapi/groups/session.ts:452-454`
attaches three middlewares to the whole session group, in this nesting order:
`InstanceContextMiddleware` → `WorkspaceRoutingMiddleware` → `Authorization`.

**Hop B — directory default.** `.../middleware/workspace-routing.ts:86-88`
```ts
function defaultDirectory(request, url): string {
  return url.searchParams.get("directory") || request.headers["x-opencode-directory"] || process.cwd()
}
```
Precedence: **`?directory=` query > `x-opencode-directory` header > `process.cwd()`**.
Note: no URI-decode at this point; the value is passed raw and decoded one hop later.

**Hop C — session override wins over the header.** `workspace-routing.ts:160-186`, specifically line **182**:
```ts
return RequestPlan.Local({
  directory: session?.directory || defaultDirectory(request, url),
  workspaceID: envWorkspaceID ?? workspaceID,
})
```
`sessionID` is extracted from the URL by `getWorkspaceRouteSessionID` (`workspace-routing.ts:222`, impl in
`packages/opencode/src/server/shared/workspace-routing.ts`), the session row is read (`:223-231`,
`NotFoundError` swallowed → `undefined`), and **if the session exists, its stored `directory` overrides the
header**. This is why the Python client *must* send a consistent header on every call, and why a mismatched
header on `/session/{id}/message` silently talks to the wrong instance rather than 404-ing. Confidence: **HIGH**.

**Hop D — workspace indirection.** `workspace-routing.ts:167-179`. If `OPENCODE_WORKSPACE_ID` is set, or a
`?workspace=` param resolves to a registered workspace with a `remote` target, the request is **proxied** to
another server (`proxyRemote`, `:113-146`) instead of executing locally. For the subworker case
(no env var, no `?workspace=`) this branch is dead and `RequestPlan.Local` is always taken.

**Hop E — provide `WorkspaceRouteContext`.** `workspace-routing.ts:207-208`.

**Hop F — resolve the per-directory Instance.** `.../middleware/instance-context.ts:23-35`
```ts
const route = yield* WorkspaceRouteContext
const ctx = yield* store.load({ directory: decode(route.directory) })   // decodeURIComponent at :15-21
Effect.provideService(InstanceRef, ctx)
Effect.provideService(WorkspaceRef, route.workspaceID)
```
Every downstream service reads the workdir from `InstanceRef` (`packages/opencode/src/effect/instance-ref.ts`).

**Hop G — InstanceStore cache.** `packages/opencode/src/project/instance-store.ts:43`
```ts
const cache = new Map<string, Entry>()
```
`load()` (`:108-124`) is `Effect.uninterruptibleMask` + a `Deferred`; concurrent requests for the same directory
share one boot fiber. **The map has no eviction, no size bound, and no TTL.** Confidence: **HIGH**.

**Hop H — project identity + worktree.** `instance-store.ts:45-63` → `Project.fromDirectory`
(`packages/opencode/src/project/project.ts:213-235`):
```ts
const data = yield* projectV2.resolve(AbsolutePath.make(directory))
const worktree = data.id === ProjectV2.ID.make("global") && !data.vcs ? "/" : data.directory
```
Result is upserted into `ProjectTable` (`project.ts:220-231`). The resulting triple is
`InstanceContext = { directory, worktree, project }` (`packages/opencode/src/project/instance-context.ts:5-9`).
For a non-git directory, `worktree === "/"` — and `containsPath` (`:16-22`) explicitly special-cases that so the
`external_directory` permission is not neutered.

### 1.2 POST /session → session creation
`groups/session.ts:203-214` declares the endpoint (path `root = "/session"` at `:29`; `SessionPaths.create` at `:87`).
Payload is `[HttpApiSchema.NoContent, Session.CreateInput]` — **body optional**.
Handler: `handlers/session.ts:155-157` `create()` → `SessionShare.Service.create(ctx.payload)`.
Raw variant `createRaw` at `handlers/session.ts:159-176`: empty body → `create({})`, else
`Schema.decodeUnknownEffect(Session.CreateInput)` and **any decode failure becomes `HttpApiError.BadRequest`**.
`Session.create` itself: `packages/opencode/src/session/session.ts:667-689` → `createNext` (`:499`).

**Consumer note:** `opencode_client.py:261` sends `{"directory": ..., "agent": ...}`. `Session.CreateInput`'s
`directory` field is **not** what selects the workdir — only the header does (Hop B/C). The Python comment at
`:264-265` ("OpenCode ignores body.directory") is **correct and confirmed**.

### 1.3 POST /session/{id}/message → prompt → loop
Endpoint `groups/session.ts:316-328` (`SessionPaths.prompt` `:95` = `${root}/:sessionID/message` — note
**the same path as GET messages**, dispatched by method). Payload `PromptPayload` (`:70`) = `SessionPrompt.PromptInput` minus `sessionID`.

Handler `handlers/session.ts:295-309`:
```ts
yield* requireSession(ctx.params.sessionID)
const message = yield* promptSvc.prompt({ ...ctx.payload, sessionID }).pipe(mapError → BadRequest)
return HttpServerResponse.stream(Stream.make(JSON.stringify(message)).pipe(Stream.encodeText),
                                 { contentType: "application/json" })
```
**This is a single-element stream, not SSE.** It emits one JSON chunk containing the *final assembled assistant
message* after the whole turn completes. The Python client's `resp.json()` is therefore correct, and the 30 s
`send_message` timeout in `opencode_client.py:342` is a real risk: the request is held open for the entire turn.

### 1.4 `SessionPrompt.prompt` → `loop` → `runLoop`
`packages/opencode/src/session/prompt.ts:1052-1071`:
1. `sessions.get` (`:1055`), `revert.cleanup` (`:1056`)
2. `createUserMessage` (`:1057`, impl `:635`) — persists the user message
3. `sessions.touch` (`:1058`)
4. **per-prompt tool permissions**: `input.tools` map is flattened into rules `{permission: t, action: enabled?"allow":"deny", pattern:"*"}` and **overwrites `session.permission`** (`:1060-1067`)
5. `loop({sessionID})` (`:1070`)

`loop` (`:1343-1347`) = `state.ensureRunning(sessionID, lastAssistant(sessionID), runLoop(sessionID))`.

`ensureRunning` — `packages/opencode/src/session/run-state.ts:88-94` → `Runner.make` per session (`:59-66`) with
`onIdle`/`onBusy`/`onInterrupt`. `runners: Map<SessionID, Runner>` is **InstanceState** (`:35-50`).

### 1.5 `runLoop` — the actual agent loop
`packages/opencode/src/session/prompt.ts:1081-1341`. Per iteration:

| line | action |
|---|---|
| 1089 | `status.set(sessionID, {type:"busy"})` |
| **1092** | **`msgs = yield* MessageV2.filterCompactedEffect(sessionID)`** — full history re-read (see §7.3) |
| 1096 | `MessageV2.latest(msgs)` |
| 1106-1130 | exit condition: `lastAssistant.finish` set, not in `["tool-calls","unknown"]`, no pending tool calls, `parentID === lastUser.id` |
| 1133-1139 | step 1: fork `title()` generation (uses agent `"title"`) |
| 1141 | `getModel(...)` |
| 1144-1147 | `subtask` task → `handleSubtask` (`:255`), `continue` |
| 1149-1159 | `compaction` task → `compaction.process`, may `break` |
| 1161-1168 | token overflow → `compaction.create({auto:true})`, `continue` |
| 1170-1177 | `agents.get(lastUser.agent)`; if falsy → publish `Session.Event.Error` + throw `Agent not found` |
| 1178-1179 | `maxSteps = agent.steps ?? Infinity` |
| 1180-1184 | `SessionReminders.apply` |
| 1186-1201 | build + persist assistant message (`MessageID.ascending()`, `path: {cwd: ctx.directory, root: ctx.worktree}`) |
| 1213-1219 | `processor.create({assistantMessage, sessionID, model})` → `handle` |
| 1226-1241 | **`SessionTools.resolve({agent, session, model, processor: handle, bypassAgentCheck, messages, promptOps})`** |
| 1243-1250 | `json_schema` format → inject `StructuredOutput` tool |
| 1252-1253 | step 1: fork `summary.summarize` |
| 1255 | `plugin.trigger("experimental.chat.messages.transform", {}, {messages: msgs})` |
| 1257-1269 | build `system[]` = env + instructions + mcp + skills |
| 1272-1286 | **`handle.process({user, agent, permission, sessionID, system, messages, tools, model, toolChoice})`** |
| 1295-1329 | outcome → `"break"` / `"continue"`; `content-filter` finish becomes `ContentFilterError` |
| 1338 | `compaction.prune({sessionID})` forked, ignore-fail |
| 1339 | `return yield* lastAssistant(sessionID)` |

### 1.6 Provider request + streaming
`handle.process` → `packages/opencode/src/session/processor.ts:641-697`:
- `status.set(busy)` (`:653`), `const stream = llm.stream(streamInput)` (`:654`)
- `stream.pipe(Stream.tap(handleEvent), Stream.takeUntil(() => ctx.needsCompaction), Stream.runDrain)` (`:656-660`)
- wrappers: `Effect.onInterrupt` → `halt(AbortError)` (`:662-669`), `Effect.retry(SessionRetry.policy(...))` which also publishes `{type:"retry"}` status (`:674-688`), `Effect.catch(halt)` (`:689`), `Effect.ensuring(cleanup())` (`:690`)

`LLM.run` — `packages/opencode/src/session/llm.ts:85-355`:
- resolves `language/cfg/item/info` in parallel (`:95-103`)
- `LLMRequestPrep.prepare({...provider, auth, plugin, flags, isWorkflow})` (`:106-113`)
- **two runtimes**: if `flags.experimentalNativeLlm` and `@opencode-ai/llm` supports the provider →
  `LLMNativeRuntime.stream(...)` returning `{type:"native", stream}` (`:226-253`); otherwise **fallback to
  AI SDK `streamText(...)`** (`:278-354`). The default path is AI SDK.
- AI SDK call: `streamText({ activeTools: Object.keys(prepared.tools).filter(x => x !== "invalid"), tools: prepared.tools, toolChoice, maxOutputTokens, abortSignal, headers, maxRetries: input.retries ?? 0, messages, model: wrapLanguageModel(language, {middleware:[transformParams]}) })` (`:280-353`)
- **Tool dispatch is delegated to AI SDK**, not to opencode. `prepared.tools[name].execute` is the callback.

### 1.7 Stream events → parts (persistence is event-sourced, NOT direct)
`handleEvent` — `packages/opencode/src/session/processor.ts:278-551`. Cases and their persistence path:

| event | line | calls |
|---|---|---|
| `reasoning-start` | 280-292 | `session.updatePart` → **durable** |
| `reasoning-delta` | 294-306 | `session.updatePartDelta` → **live only, no DB** |
| `reasoning-end` | 308-313 | `finishReasoning` → durable |
| `tool-input-start/delta/end` | 315-329 | `ensureToolCall` → durable |
| `tool-call` | 331-381 | `ensureToolCall` + `updateToolCall`; **doom-loop guard** at `:356-380` asks `permission "doom_loop"` when the last 3 parts are the same tool+input |
| `tool-result` | 383-414 | image normalize + `truncate.output`, `completeToolCall` |
| `tool-error` | 416-419 | `failToolCall` |
| `provider-error` | 421-422 | `throw` |
| `step-start` | 424-433 | `snapshot.track()` + `updatePart(step-start)` |
| `step-finish` | 435-497 | snapshot patch, usage accumulation, `updatePart(patch)` |
| `text-start` | 500-511 | `updatePart({text:""})` → durable |
| `text-delta` | 513-524 | `ctx.currentText.text += value.text` then `updatePartDelta` → **live only** |
| `text-end` | 526-546 | `plugin.trigger("experimental.text.complete")` then `updatePart(full text)` → durable |
| `finish` | 548-549 | no-op |

`cleanup()` `:553-611` — snapshot patch → `patch` part, close open text/reasoning, wait ≤250 ms per tool Deferred
(`:587`), then mark every still-open tool call `status:"error", error:"Tool execution aborted",
metadata.interrupted=true` (`:591-607`), set `time.completed`, `updateMessage`.

**The key architectural fact:** `Session.updateMessage` (`session/session.ts:629-633`) and `Session.updatePart`
(`:635-643`) **never touch SQLite**. They only `events.publish(...)`. Note `updatePart` does
`structuredClone(part)` on every call (`session.ts:639`).
Writes happen in a **projector**: `packages/core/src/session/projector.ts`, registered via `events.project(...)`.
`PartUpdated` handler = `projector.ts:310-328`:
```ts
const row = yield* db.select().from(PartTable).where(eq(PartTable.id, id)).get()   // :316  READ
yield* db.insert(PartTable).values({...}).onConflictDoUpdate({target: PartTable.id, set:{data}}).run()  // :317-322 WRITE
const previous = row && usage(row.data); const next = usage(event.data.part)
if (previous) yield* applyUsage(db, row.session_id, previous, -1)   // :325
if (next)     yield* applyUsage(db, sessionID, next)                 // :326
```
**2+ statements per part update, plus 1-2 more for usage.** Confidence: **HIGH**.

### 1.8 The durable-event tax (this is the biggest single architectural fact)
`packages/schema/src/v1/session.ts:501-504` — the `options` spread into every session event:
```ts
const options = { durable: { aggregate: "sessionID", version: 1 } } as const
```
Applied to `Created` (`:572`), `Updated` (`:580`), `Deleted` (`:588`), `MessageUpdated` (`:596`), and (by
inspection of the same object literal) `MessageRemoved`, `PartUpdated`, `PartRemoved`.
`message.part.delta` (`PartDelta`, `v1/session.ts:632-641`) has **no** `...options` → live-only, no DB. Verified.

So for `definition.durable`, `packages/core/src/event.ts:378-379` routes into `commitDurableEvent`, which at
`:239-352` runs **one `db.transaction(..., { behavior: "immediate" })` per event**:
1. `SELECT seq, owner_id FROM event_sequence WHERE aggregate_id=?` (`:243-248`)
2. `Schema.encodeUnknownSync(definition.data)(event.data)` (`:250`) — full JSON encode of the part
3. `SELECT aggregate_id, seq FROM event WHERE id=?` dup check (`:303-308`)
4. **all projectors run inline inside the txn** (`:320-322`) → the `part` table read+write above
5. `INSERT…ON CONFLICT` into `event_sequence` (`:324-335`)
6. `INSERT INTO event` (`:336-348`)

**Per assistant turn** (one text block, no tools) that is roughly: `text-start`, `step-start`, `text-end`,
`step-finish`, message-updated ×2-3, cleanup message-update ≈ **6-8 durable events ≈ 35-55 SQL statements**,
all inside `BEGIN IMMEDIATE` transactions, all serialized through §7.1's single-permit semaphore on a
synchronous driver. Confidence: **HIGH** on the mechanism, **MEDIUM** on the exact per-turn count (depends on
provider event cadence and `Truncate`/usage branches).

### 1.9 Completion / idle detection
Two independent mechanisms:

**(a) In-loop exit** — `prompt.ts:1111-1130` (see §1.5). Breaks when the last assistant message has a terminal
`finish` and no unresolved tool calls.

**(b) Status map** — `packages/opencode/src/session/status.ts`:
- state is `InstanceState.make(() => Effect.succeed(new Map<SessionID, Info>()))` (`:26-28`) — **per-directory**
- `set` (`:39-48`): publishes `Event.Status`; **if `status.type === "idle"` it publishes `Event.Idle` and
  `data.delete(sessionID)`** — i.e. **idle sessions are REMOVED from the map, not stored as `{"type":"idle"}`**
- `get` (`:30-33`) defaults to `{type:"idle"}` when absent
- `list` (`:35-37`) returns a **copy** of the map

`run-state.ts:59-66`: `Runner.make({onIdle: delete runner + status.set(idle), onBusy: status.set(busy)})`.
Retry publishes `{type:"retry", attempt, message, action, next}` (`processor.ts:678-686`).
`cancel` (`run-state.ts:77-86`) also force-sets idle when no runner exists; `cancelBackgroundJobs` (`:111-143`)
walks the `BackgroundJob` graph transitively via `metadata.sessionId` / `metadata.parentSessionId`.

**Consequences for the Python consumer (verified against source, HIGH confidence):**
- `GET /session/status` (`handlers/session.ts:77-79` = `Object.fromEntries(statusSvc.list())`) returns **only
  busy/retry sessions**. Idle sessions are simply absent.
- `opencode_client.py:240-241` `status_map.get(session_id, {"type":"idle"})` therefore returns `"idle"` **by
  absence** — it works, but only because the server omits them.
- `wait_for_response` Phase 1 (`:542-553`) waits for `"busy"`; Phase 2 (`:556-565`) waits for `"idle"`. Phase 2
  can succeed **immediately** if the session was never registered busy (e.g. it finished before the first poll,
  or the wrong directory header was used → different InstanceState → empty map). This is a real race.
- **Because status is `InstanceState` keyed by directory, a wrong/missing `x-opencode-directory` makes a running
  session report `idle`.** `opencode_client.py:230` `get_status_map` is called with **no directory argument** in
  `get_session_status` (called from `wait_for_response` `:543` and `:557`), so it falls back to `?directory=` →
  header → `process.cwd()` = the server's CWD. Confidence: **HIGH** — this is a latent bug in the consumer.

### 1.10 Abort
`POST /session/{id}/abort` → `groups/session.ts:253-264` → `handlers/session.ts:232-235`
`promptSvc.cancel(sessionID)` → `prompt.ts:152` → `run-state.ts:77-86` → `runner.cancel()`.
Returns `true` unconditionally (so the Python client's idempotency note at `:286` is accurate, but the value
carries no information).

---

## 2. HTTP API SURFACE THE SUBWORKER SERVER DEPENDS ON

All confirmed present. Path constants in `groups/session.ts:78-105`, `groups/global.ts:69`,
`groups/instance.ts:43-44`, `groups/provider.ts:12`.

| # | Client call | Method | Path | TS location | Notes |
|---|---|---|---|---|---|
| 1 | `health()` `:169-174` | GET | `/global/health` | `groups/global.ts:69,79-88`; handler `handlers/global.ts:66-68` | Returns `{healthy: true, version: InstallationVersion}`. `healthy` is a `Schema.Literal(true)` (`global.ts:13`) — **always true, no real health probe**. |
| 2 | `list_sessions()` `:184-202` | GET | `/session/` | `groups/session.ts:111-120` (`root="/session"`); handler `handlers/session.ts:64-75` | **⚠️ client sends a trailing slash, route is `/session`.** No `"/session/"` literal exists anywhere in the tree. Effect `HttpRouter` path matching is not verified here → **UNVERIFIED whether the trailing slash 404s or is normalized**. Query: `ListQuery` = `{directory?, workspace?, scope?, path?, roots?, start?, search?, limit?}` (`groups/session.ts:30-38`). **Default limit is `undefined` here** (unlike `packages/server`'s 50) — the handler passes `ctx.query.limit` straight through (`handlers/session.ts:73`). Response is a bare `Array<Session.Info>`, not `{data, cursor}`. |
| 3 | `create_session()` `:243-271` | POST | `/session` | `groups/session.ts:203-214`; handler `handlers/session.ts:155-176` | Body optional. `directory` in body is **inert**; header is authoritative (§1.1). `model` in body → `BadRequest` per the Python docstring `:255-257` — **plausible** given `Session.CreateInput`, not directly verified. |
| 4 | `get_session()` `:204-215` | GET | `/session/:sessionID` | `groups/session.ts:132-143`; handler `handlers/session.ts:85-87` | Errors: `BadRequest`, `ApiNotFoundError`. |
| 5 | `get_status_map()` `:217-231` | GET | `/session/status` | `groups/session.ts:121-131`; handler `handlers/session.ts:77-79` | Response = `Record<string, SessionStatus.Info>`; **idle omitted** (§1.9). |
| 6 | `send_message()` `:334-381` | POST | `/session/:sessionID/message` | `groups/session.ts:316-328`; handler `handlers/session.ts:295-309` | Body `{parts:[{type:"text",text}], agent?, model?:{modelID,providerID}, variant?}`. Returns a **one-chunk stream** of the final `SessionV1.WithParts`, held open for the whole turn. |
| 7 | `list_messages()` `:392-416` | GET | `/session/:sessionID/message` | `groups/session.ts:179-190`; handler `handlers/session.ts:106-145` | Query `{limit?, before?}`. `before` without `limit` → `BadRequest` (`:110`). `limit` absent **or 0** → `session.messages({sessionID})` = **full unpaginated history** (`:119-121`); the Python client defaults to `limit=50` (`:396`) so it is paged. Cursor is emitted as `Link`/`X-Next-Cursor` headers (`:138-144`), **not** in the body — the Python client ignores these and so can only walk backwards from the newest page. |
| 8 | `delete_session()` `:273-279` | DELETE | `/session/:sessionID` | `groups/session.ts:215-226`; handler `handlers/session.ts:178-181` | Returns literal `true`. |
| 9 | `abort_session()` `:281-311` | POST | `/session/:sessionID/abort` | `groups/session.ts:253-264`; handler `handlers/session.ts:232-235` | Returns `true`. Query `WorkspaceRoutingQuery` = `{directory?, workspace?}` (`workspace-routing.ts:27`). |
| 10 | `dispose_instance()` `:313-332` | POST | `/instance/dispose` | `groups/instance.ts:44,60-70`; handler `handlers/instance.ts:24-27` | **Not** a direct store call: `markInstanceForDisposal(ctx)` stashes `{ctx, store, bridge}` in a `WeakMap` keyed by the original `Request` (`.../httpapi/lifecycle.ts:14,21-30`), and `disposeMiddleware` (`:41-53`, wired at `server.ts:102` and `:321`) runs `store.dispose(ctx)` **after the response is produced**, failures logged not thrown. So it returns `true` before teardown finishes. |
| 11 | `list_models()` `:383-390` | GET | `/provider` | `groups/provider.ts:12,36-46`; handler `handlers/provider.ts` | Returns `Provider.ListResult` (`all`/`connected`/`default`). |

### 2.1 Fragile / undocumented surface — ranked

1. **`GET /session/status` semantics.** Omission-not-`idle` is not documented anywhere and is load-bearing for
   the consumer's completion detection. Any change to `status.set` (`status.ts:42-46`) silently breaks it.
2. **Directory-header precedence vs session row.** `workspace-routing.ts:182`. A stale/mismatched header does not
   error; it silently targets the session's own directory.
3. **Status map is per-directory.** `status.ts:26-28` via `InstanceState`. Sessions are invisible from any other
   directory's header. The Python `wait_for_response` already sends no directory (`:543`, `:557`).
4. **`GET /session/` trailing slash.** Only in the client; no matching literal in the server.
5. **`POST /session/{id}/message` is a long-poll.** 30 s client timeout (`:342`) vs a full agent turn.
6. **Cursor pagination only via headers.** `handlers/session.ts:138-144`.
7. **`/global/health` is a liveness stub**, not readiness. It answers before any directory instance exists.
8. **`/instance/dispose` is fire-and-forget.** Teardown completes after the 200.
9. **`?directory=` query beats the header** (`workspace-routing.ts:87`) — the Python client sends both in
   `abort_session` (`:299-300`) and `dispose_instance` (`:325-326`); consistent, but redundant.

---

## 3. THE TOOL SYSTEM

### 3.1 Registry and the built-in set
`packages/opencode/src/tool/registry.ts`. Interface `:77-87`. Tools are constructed **inside `InstanceState`**
(`:121-254`) so the whole registry is per-directory.

Custom tools, two sources:
- **filesystem**: `Glob.scanSync("{tool,tools}/*.{js,ts}", {cwd: dir, absolute: true, dot: true, symlink: true})`
  for each `config.directories()` dir (`:183-186`), then `await config.waitForDependencies()` (`:187`), then
  dynamic `import(pathToFileURL(match).href)` (`:192`). Namespaced `namespace` or `namespace_id` (`:195`).
- **plugins**: `for (const p of plugins) for (const [id, def] of Object.entries(p.tool ?? {}))` (`:199-204`).

Built-ins initialized at `:209-227`, ordered at `:231-249`:
`invalid`, (`question` if `flags.client ∈ {app,cli,desktop}` or `enableQuestionTool`), `shell`, `read`, `glob`,
`grep`, `edit`, `write`, `task`, `fetch`(webfetch), `todo`, `search`(websearch), `skill`, `patch`(apply_patch),
(`execute` if code-mode), (`lsp` if `experimentalLspTool`), (`plan` if `experimentalPlanMode && client==="cli"`).

### 3.2 Filtering — **there is no permission-based filter for built-in tools**
`ToolRegistry.tools()` (`:291-340`) applies exactly three filters:
- `websearch` availability: `webSearchEnabled(providerID, {exa, parallel})` (`:293-295`, impl `:58-65` — true for
  `opencode`, `opencode-go`, or the exa/parallel flags)
- **`apply_patch` vs `edit`/`write` model-family switch** (`:297-300`):
  ```ts
  const usePatch = input.modelID.includes("gpt-") && !input.modelID.includes("oss") && !input.modelID.includes("gpt-4")
  if (tool.id === ApplyPatchTool.id) return usePatch
  if (tool.id === EditTool.id || tool.id === WriteTool.id) return !usePatch
  ```
- code-mode `execute` visibility (`:305-308`)

Then per tool, `plugin.trigger("tool.definition", {toolID}, output)` (`:318`) can rewrite `description` /
`parameters` / `jsonSchema`, and for `task` the agent catalogue is appended via `describeTask` (`:265-278`, which
*does* filter: `Permission.evaluate("task", item.name, agent.permission).action !== "deny"` at `:268`).

**So a `deny` on `bash` does NOT remove `bash` from the tool list the model sees.** It is enforced at execution
time by `ctx.ask` inside the tool (§4.2). Confidence: **HIGH** — I read the whole of `registry.ts`.

The only *visibility* filtering that exists is `Permission.visibleTools` (`packages/opencode/src/permission/index.ts:216-219`,
built on `disabled()` `:204-214`), and it is used **only for MCP tools** (`registry.ts:286`, inside
`describeCodeMode`). `disabled()` maps `edit|write|apply_patch → "edit"` and the three `*_mcp_resource*` tools →
`"read"` before matching (`:205-209`).

### 3.3 The per-tool wrapper
`packages/opencode/src/session/tools.ts:92-134`. For each registry tool:
```ts
tools[item.id] = tool({
  description: item.description,
  inputSchema: jsonSchema(ProviderTransform.schema(input.model, ToolJsonSchema.fromTool(item))),
  execute(args, options) {
    return run.promise(Effect.gen(function* () {
      const ctx = context(args, options)                                  // :105
      yield* plugin.trigger("tool.execute.before", {tool, sessionID, callID}, {args})   // :106-110
      const result = yield* item.execute(args, ctx)                       // :111
      /* re-stamp attachments with fresh PartID/sessionID/messageID */    // :112-120
      yield* plugin.trigger("tool.execute.after", {...}, output)          // :121-125
      if (options.abortSignal?.aborted) yield* input.processor.completeToolCall(options.toolCallId, output)  // :126-128
      return output
    }))
  },
})
```
`context()` (`:59-90`) builds `Tool.Context = {sessionID, abort, messageID, callID, extra, agent, messages,
metadata(updater), ask(permissionRequest)}`. `ask` merges the ruleset at `tools.ts:87`:
```ts
ruleset: Permission.merge(input.agent.permission, input.session.permission ?? [])
```
and `Effect.orDie`s the result — **a `DeniedError` becomes a defect, not a typed tool error** (`tools.ts:89`).

MCP tools are appended separately at `:390-490`, with an explicit `ctx.ask({permission: key, patterns:["*"],
always:["*"]})` at `:408`. Three MCP *resource* pseudo-tools (`list_mcp_resources`,
`list_mcp_resource_templates`, `read_mcp_resource`) are added at `:136-386` only when some client advertises
`resources` capability (`:136-138`); each has its own `ctx.ask({permission:"read", ...})`
(`:180`, `:263`, `:343`).

### 3.4 Tool implementations — permission call sites
Permission **name** used at each `ctx.ask` (grep over `packages/opencode/src/tool/`):

| tool | file:line | permission | patterns |
|---|---|---|---|
| `read` | `read.ts:255` | `read` | file path |
| `glob` | `glob.ts:28` | `glob` | pattern |
| `grep` | `grep.ts:39` | `grep` | pattern |
| `edit` (create + replace) | `edit.ts:102`, `edit.ts:145` | `edit` | `path.relative(instance.worktree, filePath)`, `always:["*"]` |
| `write` | `write.ts:54` | `edit` | path |
| `apply_patch` | `apply_patch.ts:206` | `edit` | path |
| `shell` (bash) | `shell.ts:283` | `ShellID.ToolID` | `Array.from(scan.patterns)` from **tree-sitter parse** (`:257-260`) |
| `shell` external dirs | `shell.ts:270` | `external_directory` | globs of dirs found in the command |
| `webfetch` | `webfetch.ts:39` | `webfetch` | url |
| `websearch` | `websearch.ts:119` | `websearch` | query |
| `skill` | `skill.ts:27` | `skill` | skill name |
| `todowrite` | `todo.ts:24` | `todowrite` | — |
| `lsp` | `lsp.ts:56` | `lsp` | — |
| `task` (delegation) | `task.ts:120` | `task` | `[params.subagent_type]` |
| code-mode `execute` | `code-mode.ts:147` | `entry.key` | `["*"]` |
| external-directory helper | `external-directory.ts:35` | `external_directory` | — |
| `question` / `plan` | `question.ts:24`, `plan.ts:30` | *different mechanism* — `question.ask(...)`, not `ctx.ask` | — |

`ShellTool.ask` (`shell.ts:262-289`) is the only tool that parses the command with **tree-sitter**
(`parser().then(p => (ps ? p.ps : p.bash).parse(command))`, `:258`) to extract filesystem patterns, so a bash
permission rule can be pattern-scoped to the paths a command touches.

---

## 4. THE PERMISSION SYSTEM

### 4.1 Declaration
Two shapes, normalized to one:
- **JSON/JSONC config** → `Permission.fromConfig(info)` (`permission/index.ts:186-198`): `{key: "allow"|"deny"|"ask"}`
  becomes `{permission:key, action, pattern:"*"}`; `{key: {pattern: action}}` becomes one rule per pattern, with
  `expand()` (`:178-184`) rewriting `~/`, `~`, `$HOME/`, `$HOME`.
- **Agent frontmatter / agent config** → merged in `agent/agent.ts:293`
  `item.permission = Permission.merge(item.permission, Permission.fromConfig(value.permission ?? {}))`.

A `Rule` is `{permission, pattern, action}` where `action ∈ "allow"|"deny"|"ask"`.
`Permission.merge(...rulesets)` is a flat concat (`index.ts:200-202`) — **order is the only precedence**.

### 4.2 Evaluation — last match wins
`packages/opencode/src/permission/index.ts:28-38`:
```ts
export function evaluate(permission, pattern, ...rulesets): Rule {
  return rulesets.flat()
    .findLast(r => Wildcard.match(permission, r.permission) && Wildcard.match(pattern, r.pattern))
    ?? { action: "ask", permission, pattern: "*" }
}
```
Both the permission **name** and the **pattern** are wildcard-matched; default is `ask`.

### 4.3 Runtime gate — `Permission.ask`
`permission/index.ts:67-107`. State is `InstanceState` (`:46-65`) holding `{pending: Map<ID, {info, deferred}>, approved: Rule[]}`.

```
for each pattern in request.patterns:                       // :72
  rule = evaluate(request.permission, pattern, ruleset, approved)   // :73
  if rule.action === "deny"  -> fail DeniedError(ruleset filtered)  // :75-79   (immediate, synchronous)
  if rule.action === "allow" -> continue                            // :80
  needsAsk = true                                                    // :81
if !needsAsk -> return                                                // :84
id = request.id ?? PermissionV1.ID.ascending()                        // :86
pending.set(id, {info, deferred}); events.publish(Event.Asked, info)  // :99-100
await Deferred.await(deferred)                                        // :101-106  <-- BLOCKS THE FIBER
```
**This is the enforcement point for every permission-gated action in the system.** It is a fiber-blocking
`Deferred.await` on an in-memory map, keyed by an in-memory `Deferred`. There is no timeout inside `ask`; a
request that is never replied to holds the fiber and the `pending` entry forever (mitigated only by the
`InstanceState` finalizer at `:54-61` which fails all pendings on dispose).

### 4.4 Reply
`permission/index.ts:109-167`:
- `reject` (`:121-139`): fails the target Deferred with `CorrectedError` if a message was given, else
  `RejectedError`; then **cascades reject to every other pending request in the same `sessionID`** (`:129-138`)
- `once` (`:143`): succeed, no rule change
- otherwise (`:145-151`): push `{permission, pattern, action:"allow"}` for each `info.always` pattern into
  `approved`, then **auto-resolve every pending request in the same session whose patterns now all evaluate
  `allow`** (`:153-166`)

HTTP surface: `POST /session/:sessionID/permissions/:permissionID`
(`groups/session.ts:395-408`, marked `deprecated: true` at `:406`; handler `handlers/session.ts:362-378`).

### 4.5 Where the check happens on every tool call — summary
There is **no single chokepoint**. The chain per call is:

1. `SessionTools.resolve` wraps each tool but adds **no** permission logic (`session/tools.ts:99-133`)
2. AI SDK calls `execute(args, options)` (`session/llm.ts:318` → `:280-353`)
3. the tool's own body calls `ctx.ask(...)` at the points listed in §3.4
4. `ctx.ask` = `permission.ask({...req, sessionID, tool:{messageID,callID}, ruleset: merge(agent, session)})` + `Effect.orDie` (`session/tools.ts:81-89`)
5. `Permission.ask` evaluates and either fails, returns, or blocks (`permission/index.ts:67-107`)

Implication: **a tool that forgets to call `ctx.ask` is unguarded.** There is no registry-level safety net.
Confidence: **HIGH**.

### 4.6 Built-in defaults
`agent/agent.ts:119-136`, applied to every agent then overridden:
```
"*": "allow", doom_loop: "ask", external_directory: {"*":"ask", <whitelisted>: "allow"},
question: "deny", plan_enter: "deny", plan_exit: "deny",
read: {"*":"allow", "*.env":"ask", "*.env.*":"ask", "*.env.example":"allow"}
```
Whitelisted dirs (`:108-113`) = `Truncate.GLOB`, `Global.Path.tmp/*`, all skill dirs, all reference dirs.
Built-in agents: `build` (`:141-155`), `plan` (`:156-181`, `edit:{"*":"deny"}` except `.opencode/plans/*.md`),
`general` (`:182-195`, `todowrite:"deny"`), `explore` (`:196-218`, `"*":"deny"` + an explicit allow-list),
`compaction`/`title`/`summary` (`:219-264`, all `hidden:true`, `"*":"deny"`).
`Truncate.GLOB` is force-allowed unless explicitly denied (`:296-310`).

---

## 5. AGENT + SKILL SYSTEM

### 5.1 Agent discovery — markdown + frontmatter
`packages/opencode/src/config/agent.ts`:
- `load(dir)` (`:11-32`): `Glob.scan("{agent,agents}/**/*.md", {cwd: dir, absolute: true, dot: true, symlink: true})`
  → `ConfigMarkdown.parse(item)` (`:19`, failures silently `undefined` → skipped)
  → name via `configEntryNameFromPath(rel, ["agent/","agents/"])` (`:22`) → `{name, ...frontmatter, prompt: md.content.trim()}` (`:24-28`)
  → `ConfigParse.schema(ConfigAgentV1.Info, config, item)` (`:29`)
- `loadMode(dir)` (`:34-58`): same over `{mode,modes}/*.md`, forces `mode:"primary"`, uses
  `Schema.decodeUnknownExit(..., {errors:"all"})` and **drops the entry on failure** (`:51-56`)

Frontmatter parsing: `packages/opencode/src/config/markdown.ts:26-30` throws `FrontmatterError` naming the file
on YAML failure; a `fallbackSanitization` permissiveness path exists (`:17-18`).

Both loaders are called once **per config directory** during instance boot:
`packages/opencode/src/config/config.ts:474-475`
```ts
result.agent = mergeDeep(result.agent ?? {}, await ConfigAgent.load(dir))
result.agent = mergeDeep(result.agent ?? {}, await ConfigAgent.loadMode(dir))
```
`loadMode` runs *after* `load`, so `modes/*.md` wins on collision.

### 5.2 Agent assembly and validation
`packages/opencode/src/agent/agent.ts:98-353`, inside `InstanceState`.
- `cfg = yield* config.get()` (`:100`), `skillDirs = yield* skill.dirs()` (`:101`)
- defaults built `:119-136`; user permissions `:138`; built-in agents `:140-265`
- user agents applied `:267-294`: `disable` → delete (`:268-271`); unknown key → create with `mode:"all"`,
  `native:false` (`:272-280`); then per-field overwrite incl. `steps`, `options` (`mergeDeep`), and
  **`item.permission = Permission.merge(item.permission, fromConfig(value.permission))`** (`:293`) — appended last, so it wins
- **validation is minimal**: `Agent.get(name)` is `agents[agent]` — returns `undefined`, no error (`agent.ts:312-314`,
  service wrapper `:356-358`). The only validation is in `defaultInfo()` (`:328-340`): throws if the configured
  default agent is missing, is a `subagent`, or is `hidden`.
- **callers** raise the error: `prompt.ts:1170-1177` (`Agent not found: "<name>"` + available non-hidden list),
  `prompt.ts:461-467` (shell), `prompt.ts:313-317` (subtask)
- `list()` (`:316-326`) sorts default-agent first, then name asc
- `Agent.Info` schema at `agent.ts:35-56`

### 5.3 `@mention` / file references
Regex in `packages/opencode/src/config/markdown.ts:5-6`:
```ts
export const FILE_REGEX = /(?<![\w`])@(\.?[^\s`,.]*(?:\.[^\s`,.]+)*)/g
export const SHELL_REGEX = /!`([^`]+)`/g
```
Resolved in `SessionPrompt.resolvePromptParts` (`prompt.ts:157-191`): for each `FILE_REGEX` match it tries
`agents.get(name)` first (agent attach, `:176`), else expands `~/` (`:170`) and `fsys.stat` (`:174`), and if it
resolves to a file, replaces the `@token` text part with a `file` part. `seen: Set<string>` dedupes (`:161`).
Shell expansion `` !`cmd` `` is resolved separately in the `command` path (`prompt.ts:1397` uses
`ConfigMarkdown.shell(template)`).

### 5.4 Subagent delegation
Tool `task` — `packages/opencode/src/tool/task.ts:81+`:
- args (`:46-63`): `subagent_type`, `description`, `prompt`, optional `task_id` (resume), optional `background`
- `background=true` requires `OPENCODE_EXPERIMENTAL_BACKGROUND_SUBAGENTS=true` (`:99-101`)
- `depth >= (cfg.subagent_depth ?? 1)` → throws (`:111-115`)
- `ctx.ask({permission:"task", patterns:[params.subagent_type], metadata:{subagent_type}})` (`:120-129`)
- `agents.get(params.subagent_type)`; failure → `Unknown agent type: ...` (`:131-134`)
- permissions derived via `deriveSubagentSessionPermission` (`packages/opencode/src/agent/subagent-permissions.ts`,
  imported at `task.ts:10`), passing `parentSessionPermission: parent.permission ?? []` and `subagent: next` (`task.ts:140-141`)
- result parts are re-emitted to the parent as a `SubtaskPart`; the orchestrating side is
  `SessionPrompt.handleSubtask` (`prompt.ts:255-450`), which creates the assistant message and calls `taskTool`
  with a `taskAbort` `AbortController` (`:323-324`)
- `MessageV2.latest` classifies `parts.filter(p => p.type === "compaction" || p.type === "subtask")` as
  loop tasks (`message-v2.ts:596-600`), dispatched at `prompt.ts:1142-1159`

The `task` tool description embeds the catalogue produced by `describeTask` (`registry.ts:265-278`) — i.e. the
subagent list the model sees is itself permission-filtered.

### 5.5 Skills
`packages/opencode/src/skill/index.ts` (354 lines) + `skill/discovery.ts` (140 lines).
Patterns (`:23-25`):
```
EXTERNAL_SKILL_PATTERN  = "skills/**/SKILL.md"
OPENCODE_SKILL_PATTERN  = "{skill,skills}/**/SKILL.md"
SKILL_PATTERN           = "**/SKILL.md"
```
- `InstanceState` slot `discovered` (`:259-272`) → `discoverSkills(config, discovery, fsys, global,
  flags.disableExternalSkills, flags.disableClaudeCodeSkills, ctx.directory, ctx.worktree)`
- `Discovery.pull(url)` (`discovery.ts:49-131`) downloads to a staging dir, requires `SKILL.md` to exist
  (`:104`, `:126`), else returns `null`; dirs filtered (`:131`)
- Second `InstanceState` slot `state` (`:273-287`) registers the **built-in `customize-opencode` skill first**
  (`:278-283`) so a user-disk skill of the same name overrides it, then `loadSkills(...)`
- `get`/`require`/`all`/`dirs`/`available` (`:289-315`). **`available(agent)` filters by
  `Permission.evaluate("skill", skill.name, agent.permission).action !== "deny"`** (`:314`) — this is one of the
  few places a deny actually hides a capability from the model.
- Injection into the prompt: `SystemPrompt.skills(agent)` is called at `prompt.ts:1258` and appended to the
  `system[]` array (`:1264-1269`)
- Rendering: `fmt(list, {verbose})` (`:321-346`) — verbose emits `<available_skills><skill>…` XML,
  non-verbose emits a `## Available Skills` markdown bullet list
- The `skill` tool (`tool/skill.ts`) gates on `ctx.ask({permission:"skill"})` at `skill.ts:27`

---

## 6. CONFIG + PROJECT + WORKTREE LAYER

### 6.1 Config discovery
`packages/opencode/src/config/paths.ts:23-41`:
```ts
export const directories = (directory, worktree) => unique([
  Global.Path.config,                                       // ~/.config/opencode
  ...(!Flag.OPENCODE_DISABLE_PROJECT_CONFIG
      ? afs.up({ targets: [".opencode"], start: directory, stop: worktree }) : []),
  ...afs.up({ targets: [".opencode"], start: Global.Path.home, stop: Global.Path.home }),
  ...(Flag.OPENCODE_CONFIG_DIR ? [Flag.OPENCODE_CONFIG_DIR] : []),
])
```
So: global dir, then `.opencode` dirs walking **up from the instance directory, stopping at the worktree root**,
then `~/.opencode`, then an explicit override. `ConfigPaths.files(name, directory, worktree)` (`:10-21`) does the
same walk for `opencode.jsonc` / `opencode.json`, **reversed** (`:20`) so nearest-first.

### 6.2 Merge order
`packages/opencode/src/config/config.ts`:
- global: `config.json` (`:272`), then `opencode.json` (`:273`), then `opencode.jsonc` (`:274`) — later wins; a
  legacy `config/` dir is migrated to `config.json` (`:276-285`)
- `mergeConfig(target, source)` = remeda `mergeDeep` (`:43`)
- per-directory loop `:437-479`: for dirs ending in `.opencode` (or equal to `OPENCODE_CONFIG_DIR`) load
  `opencode.json` then `opencode.jsonc` (`:439-448`); `ensureGitignore(dir)` (`:451`); then **fork a detached
  `npm install @opencode-ai/plugin@<version>` for EVERY config directory** (`:453-470`, `Effect.forkDetach`);
  then `command` → `agent` → `mode` → `plugin` merges (`:473-479`)
- more per-directory sources at `:532` (`opencode.json`/`.jsonc`) and `:551` (built-in agents) and `:561`
  (`Flag.OPENCODE_PERMISSION` JSON)
- **directory-local overrides last**: `:577` `result.permission = mergeDeep(perms, result.permission ?? {})`
- config state is `InstanceState` (`:614-619`); `Config.get()` at `:620`, `Config.directories()` at `:624`,
  `waitForDependencies()` at `:632`, `invalidate()` at `:652`

**Startup side effects per distinct directory:** one file-walk, N × `npm install` forks, agent/markdown glob +
YAML parse, skill glob + YAML parse, plugin import, MCP connect, LSP spawn, watcher setup. At 100 sessions across
100 distinct directories this is 100× that work with no sharing — there is no cross-instance cache.

### 6.3 Project identity
`packages/opencode/src/project/project.ts:213-235`. `projectV2.resolve(absolutePath)` returns `{id, previous?, vcs?,
directory}`; the id is upserted into `ProjectTable` and `migrateProjectId` renames old ids (`:221-231`).
`worktree = (id === "global" && !vcs) ? "/" : data.directory` (`:217`).
`ProjectTable` is at `packages/core/src/project/sql.ts`; project directories are also tracked
(`migration/20260602182828_add_project_directories.ts`, `20260612174303_project_dir_strategy.ts`).

### 6.4 Worktree / workdir isolation
Two distinct notions, both present in `InstanceContext`:
- **`directory`** = the request's workdir. All tool `cwd`, `path.cwd` on assistant messages (`prompt.ts:1193`),
  and the tool-cache key.
- **`worktree`** = the project/git root. Used for the permission baseline: every `edit` ask uses
  `path.relative(instance.worktree, filePath)` as the pattern (`edit.ts:103`, `:146`); `external_directory`
  containment uses `containsPath` (`instance-context.ts:16-22`).

The **Worktree service** (`packages/opencode/src/worktree/index.ts`, 623 lines) manages *additional* git
worktrees: `candidate` (`:175-198`, slug + collision retry), `makeWorktreeInfo` (`:199-280`, runs
`git worktree add`), `create`/`createFromInfo` (`:281-292`), `list` (`:333`), `remove` (`:388-431`, runs
`git worktree remove --force` and parses `warning: failed to remove …` for `RemoveFailedError`). Events:
`WorktreeEvent` (`:21`).
In the subworker flow this is **not** used — sessions are isolated purely by `directory` via `InstanceStore` +
`InstanceState`.

### 6.5 Disposal semantics (the only reclamation path)
`packages/opencode/src/server/routes/instance/httpapi/lifecycle.ts`:
`markInstanceForDisposal` (`:21-30`) stashes in a `WeakMap<Request, MarkedInstance>` (`:14`);
`disposeMiddleware` (`:41-53`) awaits `store.dispose(ctx)` after the response, logging failures.
`InstanceStore.disposeContext` (`instance-store.ts:94-98`) → `runDisposers(ctx.directory)` → every
`InstanceState` slot's registered disposer → `ScopedCache.invalidate(cache, directory)`
(`effect/instance-state.ts:39`). Then `cache.delete(directory)` (`instance-store.ts:104`) and a
`server.instance.disposed` GlobalBus event (`:79-92`).

**The Python client's `/instance/dispose` comment at `opencode_client.py:316-319` ("watchers/LSP/MCP handles
accumulate and are never evicted") is confirmed correct**: `InstanceStore`'s `Map` (`:43`) and every
`InstanceState` `ScopedCache` (`instance-state.ts:31-37`, `capacity: Number.POSITIVE_INFINITY`) are unbounded.
Nothing evicts; only an explicit dispose call does.

---

## 7. THE STORAGE LAYER

### 7.1 Driver, pragmas, and the serialization point
- Driver: **`node:sqlite` `DatabaseSync`** (`packages/core/src/database/sqlite.node.ts:1,151-157`), selected via
  the `#sqlite` import condition (`packages/core/package.json:25-30`; `bun` condition → `sqlite.bun.ts`).
  `enableForeignKeyConstraints: true`, `open: true`, `config.timeout`.
- `readonly(config)` = single `DatabaseSync`. Drizzle on top (`sqlite.node.ts:167-170`), Effect `SqlClient` shim
  (`sqlite.node.ts:48-137`).
- **🔴 `const semaphore = yield* Semaphore.make(1)` at `sqlite.node.ts:110`.** `acquirer =
  semaphore.withPermits(1)(...)` (`:113`) and `transactionAcquirer` takes the same permit
  (`:114-123`). **Every SQL statement in the process serializes through one permit.**
- **🔴 `statement.all(...)` (`sqlite.node.ts:63`) is a synchronous C call on the JS main thread.** No worker,
  no `node:worker_threads`, no async driver. Every query blocks the event loop for its duration.
- WAL is set once in the native layer (`sqlite.node.ts:159`). The rest of the pragmas are in
  `packages/core/src/database/database.ts:27-32`:
  ```
  PRAGMA journal_mode = WAL
  PRAGMA synchronous = NORMAL
  PRAGMA busy_timeout = 5000
  PRAGMA cache_size = -64000      // 64 MB page cache, ONE global connection
  PRAGMA foreign_keys = ON
  PRAGMA wal_checkpoint(PASSIVE)
  ```
  **No `mmap_size`, no `temp_store`, no `wal_autocheckpoint` tuning.**
- DB path: `packages/core/src/database/database.ts:43-55` — `Flag.OPENCODE_DB` (`:memory:` or absolute honoured)
  else `Global.Path.data/opencode.db` for `latest|beta|prod` channels, else `opencode-<channel>.db`.
  **One database file for the entire server**, all directories, all sessions. Confidence: **HIGH**.
- Migrations: `DatabaseMigration.apply(db)` (`database.ts:33`), 40 files in
  `packages/core/src/database/migration/`, run at layer build.

### 7.2 Tables and what holds session/message/part data
See the table in §0.1. Key points:
- `message.data` and `part.data` are **`text({mode:"json"})`** (`session/sql.ts:77,92`) — the entire typed
  message/part body as one JSON string. No content columns, so every read is `JSON.parse` of a full row.
- Indexes: `message(session_id, time_created, id)` (`sql.ts:79`), `part(message_id, id)` (`:95`),
  `part(session_id)` (`:96`), `session(project_id|workspace_id|parent_id)` (`:62-64`).
- `event(id)` PK, `event(aggregate_id, seq)` UNIQUE, `event(aggregate_id, type, seq)`, FK to
  `event_sequence(aggregate_id)` `ON DELETE CASCADE` (`packages/core/src/event/sql.ts:12-25`).

### 7.3 History reads — the N²-shaped hot path
`MessageV2.filterCompactedEffect(sessionID)` (`packages/opencode/src/session/message-v2.ts:578-580`) →
`stream(sessionID)` (`:473-494`):
```ts
const size = 50
while (true) {
  const next = yield* page({ sessionID, limit: size, before })
  if (next.items.length === 0) break
  for (let i = next.items.length-1; i>=0; i--) result.push(next.items[i])
  if (!next.more || !next.cursor) break
  before = next.cursor
}
```
`page()` (`:429-470`): one `SELECT … FROM message WHERE session_id=? [AND older] ORDER BY time_created DESC, id DESC
LIMIT limit+1` (`:439-445`), then `hydrate` (`:463`).
`hydrate` (`:98-119`) is **not** N+1 — it batches: one `SELECT … FROM part WHERE message_id IN (…)` (`:103-109`).
So a full-history read of N messages costs **⌈N/50⌉ × 2 queries**, but it **materializes every message and every
part JSON blob of the session into a JS array**.

**This is called at the top of every loop iteration** (`prompt.ts:1092`). Therefore: *steps per turn × full session
history reads per step*, each a full deserialization of the conversation, all through the 1-permit synchronous
semaphore. For a 60-message session with 400 parts, one loop iteration ≈ 2 message queries + 2 part queries
pulling ~400 JSON blobs. Confidence: **HIGH** on the mechanism and the call site.

`filterCompacted` (`:525-577`) then reorders for model consumption and the pure-JS `latest()` (`:586-600`) walks
the whole array again.

### 7.4 Pagination, retention, pruning
- **Message pagination**: opaque base64url cursor `{id, time}` (`message-v2.ts:71-78`, `:468`); `older(row)`
  keyset predicate at `:95-97` (`time_created < t OR (= t AND id < i)`). Surfaced as `Link`/`X-Next-Cursor`
  headers (`handlers/session.ts:138-144`).
- **Message retention**: `SessionCompaction.prune` (`packages/opencode/src/session/compaction.ts:273-303+`) walks
  messages newest→oldest, skips the last 2 turns (`:291`), stops at the first summary (`:292`), and marks
  completed tool parts for output-truncation once the cumulative estimate exceeds `PRUNE_PROTECT`, skipping
  `PRUNE_PROTECTED_TOOLS` (`:297`). Gated on `cfg.compaction?.prune` (`:275`). It **truncates tool outputs; it does
  not delete rows.**
- **Revert** does delete: `SessionEvent.RevertEvent.Committed` deletes `session_message` and `session_input` rows
  past a boundary seq (`packages/core/src/session/projector.ts:414-441`).
- **🔴 No retention on `event` / `event_sequence`.** I grepped the whole tree for `delete(EventTable`, `prune`,
  `vacuum`, `EventTable` — the only writes to `event` are the insert at `core/src/event.ts:337` and reads at
  `:78-86` (replay) and `:263-268`, `:303-308` (dup/replay checks). `event` rows are removed **only** by
  `ON DELETE CASCADE` when the session is deleted (`session.deleted` → `db.delete(SessionTable)` at
  `projector.ts:257-259`).
  **The event log is append-only for the life of the database file.** `compaction.prune` is unrelated to it.
  Confidence: **HIGH**.
- `ToolOutputStore` has its own retention: `RETENTION = Duration.days(7)` (`packages/core/src/tool-output-store.ts:15`),
  scanned once globally (`:199`).
- `Flag.OPENCODE_DISABLE_PRUNE` exists (`packages/core/src/flag/flag.ts:25`).

### 7.5 What is expensive at 100 concurrent sessions (ranked, all source-grounded)

| # | Item | file:line | Nature |
|---|---|---|---|
| 1 | **1-permit semaphore over a synchronous `node:sqlite` handle** | `sqlite.node.ts:110`, `:63` | hard serialization of all SQL; blocks the event loop |
| 2 | **Full-history re-read per loop step** | `prompt.ts:1092` → `message-v2.ts:578,473` | O(steps × history) deserialization |
| 3 | **`BEGIN IMMEDIATE` txn + ~6 statements per durable part/message update** | `core/src/event.ts:239-352`, `projector.ts:310-328` | 35-55 stmts/turn; each a write txn |
| 4 | **`event` table grows forever**, full part JSON per row | `core/src/event/sql.ts:12`, no prune found | unbounded disk + index growth on `event_aggregate_seq_idx` |
| 5 | **`PRAGMA cache_size = -64000`** | `database.ts:30` | 64 MB page cache, one connection, held for process life |
| 6 | **Unbounded `InstanceState` ScopedCache per directory** | `effect/instance-state.ts:31-37` | per-directory: status map, permission pending+approved, registry, runners; never evicted |
| 7 | **Unbounded `InstanceStore` Map** | `project/instance-store.ts:43` | one entry per directory ever touched |
| 8 | **`Queue.unbounded()` per SSE connection** | `handlers/event.ts:31` | every event offered to every client **before** the directory filter (`:35-38`); a slow client grows without bound. *The Python client does not open SSE — verified, it polls `/session/status`.* |
| 9 | **`structuredClone(part)` per part update** | `session/session.ts:639` | full deep clone on every streaming update |
| 10 | **Plugin hooks awaited sequentially**, per-directory plugin load | `plugin/index.ts:284-297`, `:134` | each `tool.execute.before/after` awaits every plugin serially; plugins loaded once per directory |
| 11 | **`npm install` fork per config directory per instance boot** | `config/config.ts:453-470` | process spawn storm on cold directories |
| 12 | **`GlobalBus` is a plain `EventEmitter`, `setMaxListeners` never called** | `bus/global.ts:11-22` | >10 listeners ⇒ Node warning; every event dispatched to all listeners regardless of directory |

---

## 8. PLUGIN API SURFACE — SERVER SIDE (contract only; deep map is plugin-host's job)

### 8.1 What `packages/plugin` exports (the types a plugin is written against)
`packages/plugin/src/index.ts` (335 lines):
- `Plugin` = `(input: PluginInput, options?: PluginOptions) => Promise<Hooks>` (`:68`)
- `PluginModule = {id?, server: Plugin, tui?: never}` (`:70-74`)
- `PluginInput` (`:51-64`): `{client, project, directory, worktree, experimental_workspace:{register(type, adapter)}, serverUrl, $: BunShell}`
- `WorkspaceAdapter` / `WorkspaceTarget` / `WorkspaceInfo` (`:29-49`)
- `AuthHook` + `AuthOAuthResult` (`:76-185`)
- `ProviderHook` (`:187-192`)
- `Hooks` interface — **the full hook list** (`:194-334`)
- sub-exports: `./tool.js` (`ToolDefinition`, re-exported at `:15`), `./shell.js` (`BunShell`, `:13`),
  `./tui.js` (634 lines, TUI-only)
- `Config = Omit<SDKConfig,"plugin"> & {plugin?: Array<string|[string, PluginOptions]>}` (`:66`)

**Server-side hook list (from `Hooks`, `packages/plugin/src/index.ts:194-334`) — 21 hooks:**
`dispose`, `event`, `config`, `tool` (map of defs, `:210-212`), `auth` (`:213`), `provider` (`:220`),
`chat.message` (`:225`), `chat.params` (`:236`), `chat.headers` (`:247`), `permission.ask` (`:254`),
`command.execute.before` (`:255`), `tool.execute.before` (`:262`), `shell.env` (`:268`),
`tool.execute.after` (`:273`), `experimental.chat.messages.transform` (`:283`),
`experimental.chat.system.transform` (`:292`), `experimental.provider.small_model` (`:299`),
`experimental.session.compacting` (`:308`), `experimental.compaction.autocontinue` (`:319`),
`experimental.text.complete` (`:328`), `tool.definition` (`:332`).

There is also a **V2 plugin API** under `packages/plugin/src/v2/{effect,promise}/*` (agent, aisdk, catalog,
command, context, event, filesystem, integration, location, npm, path, plugin, reference, registration, skill) —
a parallel Effect/Promise-split surface. Confidence that this is the actively-served path: **LOW** (not traced).

### 8.2 What `packages/opencode/src/plugin` provides TO plugins
`packages/opencode/src/plugin/index.ts` (318 lines):
- `PluginInput` construction `:152-168`: `client = createOpencodeClient({baseUrl: Server.url ?? "http://localhost:4096",
  directory: ctx.directory, headers: ServerAuth.headers(), fetch: in-process Server.Default().app.fetch when no URL})`
  (`:146-151`) — **plugins get an in-process fetch into the same server when no listener is up**
  (`server.ts:56-65` builds `HttpApiApp.webHandler()`)
  `$: typeof Bun === "undefined" ? undefined : Bun.$` (`:167`) — **`$` is `undefined` under Node**
- Internal plugins first: `internalPlugins(flags)` (`:170-179`), skipped when `flags.disableDefaultPlugins`
- External plugins from `cfg.plugin_origins` (`:181`), `waitForDependencies()` if any (`:184`),
  then `PluginLoader.loadExternal({items, kind:"server", report})` (`:186-218`) with staged error reporting:
  `install` / `compatibility` / `entry` / `load` (`:193-215`), each publishing `Session.Event.Error`
- **Plugins are applied sequentially on purpose** — comment at `:222-223`: "Keep plugin execution sequential so
  hook registration and execution order remains deterministic"
- `config` hook fired for all plugins after load (`:245-253`)
- `event` hook subscribed through `events.listen`, **filtered on `event.location?.directory !== ctx.directory`**
  (`:255-262`); dispatched with `void hook.event?.(...)` — **fire-and-forget, not awaited**
- `dispose` hooks registered as scope finalizers (`:265-277`)
- The whole `hooks` array lives in `InstanceState` (`:134`) ⇒ **plugins are loaded and held once per directory**
- `trigger(name, input, output)` (`:284-297`):
  ```ts
  if (!name) return output
  for (const hook of s.hooks) {
    const fn = hook[name]; if (!fn) continue
    yield* Effect.promise(async () => fn(input, output))   // ← AWAITED, SEQUENTIALLY
  }
  return output
  ```
  `output` is mutated in place, so hooks compose by side effect on a shared object.
- `list()` `:299-302`, `init()` `:304-306` (forces cache population)
- **Loader**: `packages/opencode/src/plugin/loader.ts` (237 lines) — `mod = await import(row.entry)` (`:139`),
  parallel attempts `:210`

### 8.3 Server-side trigger call sites (who calls what)
| hook | caller | file:line |
|---|---|---|
| `config` | post-load fan-out | `plugin/index.ts:247` |
| `event` | `events.listen` (directory-filtered, unawaited) | `plugin/index.ts:259` |
| `dispose` | scope finalizer | `plugin/index.ts:269` |
| `tool.definition` | registry per tool | `tool/registry.ts:318` |
| `tool.execute.before` / `.after` | wrapper (builtin) | `session/tools.ts:107`, `:121` |
| `tool.execute.before` / `.after` | MCP resource tools | `session/tools.ts:176`,`:209`,`:259`,`:292`,`:339`,`:374` |
| `tool.execute.before` / `.after` | MCP tools | `session/tools.ts:402`,`:420` |
| `experimental.chat.messages.transform` | per loop step | `prompt.ts:1255` |
| `experimental.text.complete` | on `text-end` | `processor.ts:530` |
| `shell.env` | shell impl | `prompt.ts:554` |
| `chat.params` / `chat.headers` / `permission.ask` | `packages/opencode/src/session/llm/request.ts` / `permission/` | **not traced — UNVERIFIED** |
| `chat.message` | prompt creation | **not traced — UNVERIFIED** |

---

## 9. TS -> RUST CORRESPONDENCE

### 9.1 Size reconciliation

| | TS (`packages/*/src`) | Rust (`crates/`) |
|---|---|---|
| units | 15 packages | 19 crates (all declared as members, `Cargo.toml:3-23`) |
| files | 1,043 `.ts` | **208 `.rs`** (13 more under `target/` are build artifacts — exclude) |
| LOC | 146,127 measured (204,808 claimed for full repo incl. tests/sdk/ui) | **89,698** |
| server-relevant LOC | ~110k (excl. `ui` 15,963 + `tui` 5,883) | ~62,700 (excl. `opencode-tui` 21,418 + `opencode-cli` 5,551) |

Rust per-crate: `tui` 21,418 · `provider` 17,489 · `session` 14,115 · `tool` 8,897 · `server` 6,832 ·
`cli` 5,551 · `config` 3,730 · `mcp` 2,376 · `plugin` 1,951 · `agent` 1,792 · `storage` 1,263 · `lsp` 975 ·
`util` 900 · `permission` 807 · `types` 359 · `command` 343 · `grep` 330 · `watcher` 303 · `core` 267.

Git HEAD `618c4b9`. Workspace: `resolver = "2"`, edition 2021, version 0.1.0, MIT (`Cargo.toml:1-29`),
50 workspace deps (`:31-81`). **Only 4 TODO comments workspace-wide and ZERO `todo!()`/`unimplemented!()`.**

### 9.2 Crate-by-crate correspondence

| Rust crate | LOC | TS subsystem it maps to | TS anchors | Faithful? |
|---|---|---|---|---|
| `opencode-core` | 267 | `bus/global.ts` + `id/` + `core/src/event.ts` bus half | `bus/global.ts:11-22`, `event.ts:606-613` | **PARTIAL.** Rust `Bus` (`bus.rs:29`, publish `:47`, subscribe `:70`, define_event `:134`) is a clean pub/sub. TS's `listeners[]` array + `projectors` map + `notify` inline sequential dispatch (`event.ts:406-417`) has **no equivalent**. |
| `opencode-types` | 359 | `packages/schema` (3,387) + `protocol` (1,582) | `schema/src/v1/session.ts`, `session-status-event.ts` | **PARTIAL / LOSSY.** 359 LOC of serde structs vs 4,969 LOC of Effect Schema. TS has codecs (`Schema.encodeUnknownSync` at `event.ts:250`), branded IDs (`SessionID.make`, `AbsolutePath.make`), and discriminated unions (`v1/session.ts:492 Info = Schema.Union([User, Assistant])`). Rust has plain serde. Expect hand-written conversion at every boundary. |
| `opencode-util` | 900 | `util/`, `core/src/util/{wildcard,glob}`, `format/` | `permission/index.ts:32` uses `Wildcard.match` | **FAITHFUL** for wildcard/color/timeout/lock/token/format/git/abort + `Filesystem` (`filesystem.rs:5`, `contains` `:53`, `find_up` `:60`, `glob_up` `:86`). |
| `opencode-grep` | 330 | `Ripgrep` node + `tool/grep.ts` | `grep.ts:39` | **FAITHFUL.** Shells out to ripgrep like TS. Note `Cargo.toml:9-15` has **no `nucleo-matcher`/`syntect`** — matching is entirely ripgrep's job, same as TS. |
| `opencode-permission` | 807 | `permission/index.ts` (223) + `core/src/v1/permission` | `index.ts:28-38` evaluate, `:67-107` ask, `:109-167` reply, `:200-219` merge/disabled/visibleTools | **MOSTLY FAITHFUL, one deliberate divergence.** Rust has `evaluate` (`ruleset.rs:84`), `merge` (`:80`), `from_config` (`:53`), `default_ruleset` (`:151`), `build_agent_ruleset` (`:211`), `PermissionEngine::ask` (`engine.rs:97`) / `respond` (`:147`) / `clear_session` (`:186`) / `is_approved` (`:85`), and `disabled` (`ruleset.rs:100`). **Divergence:** TS has no timeout inside `ask` and blocks a fiber on an in-memory `Deferred`; Rust `ask` is `async` (`engine.rs:97`) — better, but the in-memory pending-map model is otherwise identical. |
| `opencode-config` | 3,730 | `config/` (18 files, ~1,200) + `core/src/v1/config/*` | `config/config.ts:437-479` merge loop, `config/paths.ts:23-41` dir walk | **FAITHFUL on shape, UNVERIFIED on the side effects.** `ConfigLoader::load_all` (`loader.rs:143`), `load_all_with_remote` (`:226`), `merge` (`schema.rs:1180`), `load_global` (`:63`), `load_project` (`:87`), jsonc via `jsonc-parser 0.29`. **🔴 UNVERIFIED whether the Rust port keeps TS's per-config-directory `npm install` fork (`config.ts:453-470`) — that is a per-directory process spawn and its absence/presence changes cold-start cost by orders of magnitude.** |
| `opencode-plugin` | 1,951 | `plugin/index.ts` (318) + `plugin/loader.ts` (237) + `packages/plugin` (1,612) | `plugin/index.ts:284-297` trigger, `:134` InstanceState hook array, `loader.ts:139` `await import()` | **ARCHITECTURALLY DIVERGENT — deliberate.** TS = in-process `await import()` of TS/JS modules, hooks awaited **sequentially** per call (`index.ts:294`). Rust = **JSON-RPC subprocess** to an external bun/deno/node (`subprocess/protocol.rs:14 RpcRequest`, `client.rs:122 PluginSubprocess`, `:143 spawn`, `runtime.rs:7 JsRuntime{Bun,Deno,Node}`, resolved by `which::which()` at `:54`). Hook surface: `Hook` (`lib.rs:131`), `HookEvent` (`:43`), `HookContext` (`:87`), `PluginSystem::trigger` (`:187`) / `trigger_collect` (`:316`) / `trigger_event` (`:330`) / `trigger_session_event` (`:335`), priorities (`:158 with_priority`). **This is plugin-host's decision point: IPC costs a process per plugin + serialization per hook call, but requires zero JS engine and no `pub(crate)` visibility into server internals.** |
| `opencode-command` | 343 | `command/` | — | **FAITHFUL, small.** `CommandRegistry` (`lib.rs:59`), `execute_with_hooks` (`:183`). |
| `opencode-storage` | 1,263 | `core/src/database/*` (1,700+) + `core/src/session/sql.ts` (176) + `core/src/event/sql.ts` (25) + 40 migrations | `session/sql.ts:22,68,82,100`, `event/sql.ts:5,12`, `database.ts:27-32` | **🔴 THE BIGGEST GAP. FAITHFUL ON SQLITE MECHANICS, MISSING THE EVENT-SOURCING LAYER ENTIRELY.**<br>**Mechanics — better than TS:** `sqlx 0.8` async + `SqlitePool` (`database.rs:24,68`) with `begin`/`transaction` (`:72,:79`) vs TS's `Semaphore.make(1)` over synchronous `node:sqlite` (`sqlite.node.ts:110,:63`). This alone removes the #1 bottleneck in §7.5.<br>**Schema — only 6 tables** (`schema.rs:7 CREATE_SESSIONS_TABLE`, `:50 CREATE_MESSAGES_TABLE`, `:77 CREATE_PARTS_TABLE`, `:118 CREATE_TODOS_TABLE`, `:135 CREATE_PERMISSIONS_TABLE`, `:145 CREATE_SESSION_SHARES_TABLE`, `:158 CREATE_INDEXES`, aggregated `:180 ALL_MIGRATIONS`).<br>**MISSING:** (a) no `event` / `event_sequence` tables — `ALL_MIGRATIONS` is a flat `&[&str]` replayed every startup in a bare loop (`database.rs:103`), with **no versioned migration framework** vs TS's 40 dated migrations; (b) no `session_message` (V2), no `session_input` (durable prompt inbox), no `session_context_epoch`, no `project`, no `workspace`, no `credential`, no `account`, no `share`-v2; (c) **no projector layer at all** — `opencode-core/src/session/projector.ts` (455 LOC) and `core/src/event.ts`'s `commitDurableEvent` (`:237-363`) have **no counterpart**. `PartRepository::upsert` (`repository.rs:880`) writes directly, i.e. the Rust port **drops per-aggregate sequence numbers, replay/idempotency checks, and the `BEGIN IMMEDIATE`-per-event discipline**.<br>Consequence: the Rust reference is a **different, simpler persistence design**, not a port. Whether that is a simplification or a regression is the single biggest architectural decision in this project. |
| `opencode-provider` | 17,489 | `provider/` (5 files) + `packages/llm` (9,533) + `session/llm/*` (910) | `session/llm.ts:85-355`, `session/llm/ai-sdk.ts`, `native-runtime.ts`, `provider/transform.ts` | **FAITHFUL AND LARGER.** 21 provider impls vs TS's ~15 (`llm.ts` picks `language` via `provider.getLanguage`). `Provider` trait (`provider.rs:33`), `ProviderRegistry` (`:282`), `ProviderError` (`:45`), `parse_api_call_error` (`:187`), `parse_stream_error` (`:240`), unified `StreamEvent` (`stream.rs:7`) / `StreamResult` (`:125`), `StreamUsage` (`:114`), `ToolResultOutput` (`:102`), `ToolErrorKind` (`:94`). SSE: `parse_openai_sse` (`:177`), `parse_anthropic_sse` (`:270`) + 11 private per-provider parsers — mirrors TS's per-provider `ProviderTransform`. **Responses API** (`responses.rs`, ~70 pub items, `OpenAIResponsesLanguageModel:981`) is far beyond anything in the TS file I read. `CustomFetchProxy` trait (`custom_fetch.rs:35`, `register_custom_fetch_proxy:50`) = the `auth.fetch` plugin hook. Retry (`retry.rs:145 with_retry`, `:182 with_retry_and_hook`) maps TS `SessionRetry.policy` (`processor.ts:675`). **Rust has no equivalent of TS's dual-runtime seam** (`session/llm.ts:226` `experimentalNativeLlm` vs AI SDK) — Rust IS the native path. |
| `opencode-session` | 14,115 | `session/` (23 files: prompt 1,631 + processor 732 + llm 404 + llm/* 910 + compaction 608 + system 154 + …) | `session/prompt.ts`, `processor.ts`, `llm.ts`, `compaction.ts`, `message-v2.ts`, `retry.ts`, `revert.ts`, `instruction.ts`, `summary.ts`, `system.ts` | **STRUCTURALLY FAITHFUL, module-for-module.** 14 modules at `lib.rs:3-16` map 1:1 onto the TS `session/` dir. `prompt.rs` ~3,400 LOC vs TS 3,677 across prompt+processor+llm+llm/*. Named analogues: `SessionPrompt` (`prompt.rs:137`) ↔ `SessionPrompt.Service`; `LlmProcessor` (`llm.rs:202`) ↔ `SessionProcessor`; `StreamProcessor` (`llm.rs:156`); `collect_stream` (`:1432`) ↔ `Stream.runDrain` (`processor.ts:659`); `filter_compacted` (`message_v2.rs:554`) ↔ `MessageV2.filterCompacted` (`message-v2.ts:525`); `to_model_messages` (`message_v2.rs:834`) ↔ `MessageV2.toModelMessagesEffect` (`prompt.ts:1262`); `run_compaction` (`compaction.rs:904`) + `PruneToolPart` (`:134`) ↔ `SessionCompaction` + `prune` (`compaction.ts:273`); `resolve_prompt_parts` (`prompt.rs:2615`) ↔ `prompt.ts:157`; `resolve_tools_with_permissions` (`llm.rs:1397`) ↔ `registry.tools()` (`session/tools.ts:92`); `max_steps_for_agent` (`prompt.rs:3147`) ↔ `maxSteps` (`prompt.ts:1178`); `SessionStateManager` (`session.rs:200`) ↔ `SessionRunState` + `SessionStatus` merged. **🔴 The `bypassAgentCheck` param threaded through TS `SessionTools.resolve`/`Tool.Context` (`session/tools.ts:46,64`) has no visible Rust counterpart — UNVERIFIED.** |
| `opencode-tool` | 8,897 | `tool/` (19 files, 5,198) + `tool/shell/*` | `tool/registry.ts` (455), `session/tools.ts` (590), `tool/{edit,read,write,shell,glob,grep,...}` | **FAITHFUL FOR THE CORE, DIVERGENT IN THE TOOL SET.** `Tool` trait (`tool.rs:689`), `ToolContext` (`:282`), `ToolResult` (`:258`), `ToolError` (`:707`), `ToolRegistry` (`registry.rs:11`) + `create_default_registry` (`:220`), **14 callback type aliases** (`tool.rs:61-202`: Ask, Question, SwitchAgent, CreateSubsession, PromptSubsession, FileTimeRead, PublishBus, UpdatePart, UpdateMessage, LspTouchFile, TodoUpdate, TodoGet, GetLastModel, CreateSyntheticMessage) ↔ TS's ad-hoc `EffectBridge` calls. `with_file_lock` (`tool.rs:25`) has **no TS equivalent** (TS edit relies on `FileSystem.Event.Edited` + `Watcher.Event.Updated`, `edit.ts:117-121`). `BashArity::prefix` (`permission/arity.rs`) ↔ `permission/arity.ts` (163 LOC). **tree-sitter-bash 0.24 + tree-sitter-bash 0.23** (`Cargo.toml`) ⇒ Rust *does* port the tree-sitter command scan (`shell.ts:258`). `edit/replacers.rs` ports the fuzzy-match strategies (9 `Replacer` impls, `Replacer` trait `:53`, `CompositeReplacer:583`) — TS has no equivalent module. `truncation.rs:7-9 MAX_LINES/MAX_BYTES/CLEANUP_RETENTION_DAYS` ↔ `tool/truncate.ts` (156) + `ToolOutputStore` `RETENTION` (`core/src/tool-output-store.ts:15`). **DIVERGENCE — tool sets differ both ways:** Rust ADDS `BatchTool` (`batch.rs:38`), `MultiEditTool` (`multiedit.rs:7`), `CodeSearchTool` (`codesearch.rs:59`), `LsTool` (`ls.rs:38`), `LspTool`+`PlanEnter/ExitTool`; TS ADDS `code-mode` (`tool/code-mode.ts` 310 LOC + `packages/codemode` 6,878 LOC) and `shell/id.ts`+`shell/prompt.ts` (312 LOC). **Rust gates `lsp` behind a cargo feature** (`Cargo.toml:8-10`, `default = []`, `lsp = ["opencode-lsp","lsp-types"]`) while `lib.rs:11` declares `pub mod lsp_tool` unconditionally — possible build break, UNVERIFIED. |
| `opencode-agent` | 1,792 | `agent/agent.ts` (453) + `config/agent.ts` (59) + `config/markdown.ts` (36) + `agent/subagent-permissions.ts` | `agent.ts:98-353`, `config/agent.ts:11-32` (markdown+frontmatter), `agent.ts:267-294` (merge), `:328-340` (defaultAgent validation) | **FAITHFUL.** `AgentRegistry` (`agent.rs:456`) with `from_config:472`, `from_optional_config:478`, **`from_project_dir:485`** (markdown+frontmatter discovery), `get:606`, `list:618`, `list_primary:638`, `list_subagents:658`, `default_agent:668`, `generate:687`. `BuiltinAgent` enum (`:32`, `all() -> [BuiltinAgent;6]` at `:53`) ↔ TS's 7 built-ins (`build, plan, general, explore, compaction, title, summary` — 7, not 6 ⇒ **Rust may be missing one; UNVERIFIED which**). `PermissionNext::evaluate` (`:108`) + `tool_permission_decision:444` + `is_tool_allowed:448` ⇒ **Rust DOES gate tool visibility by permission, which TS does not do for built-ins** (`registry.ts:291-303`). `AgentExecutor` (`executor.rs:30`) with `execute:135` / `execute_streaming:266` and `with_disabled_tools:80` / `with_persisted_subsessions:88` ⇒ **Rust's executor is a cleaner split than TS's `handleSubtask` inside `prompt.ts:255-450`.** 🔴 `AgentError` is declared twice as `pub enum` (`agent.rs:165` AND `executor.rs:13`) and both are glob re-exported (`lib.rs:7-8`) — probable name collision, UNVERIFIED whether it compiles. |
| `opencode-mcp` | 2,376 | `mcp/` + `core/src/v1/config/mcp` | `session/tools.ts:390-490`, `mcp/catalog` | **FAITHFUL, arguably better.** `McpTransport` trait (`transport.rs:16`) with `StdioTransport:26`, `HttpTransport:162`, `SseTransport:274` (via `reqwest-eventsource`, `:300,:308`). `McpClientRegistry` (`client.rs:648`), `MCP_TOOLS_CHANGED_EVENT` (`:138`), `McpStatus` (`:24`), full JSON-RPC protocol types (`protocol.rs:4-192`), OAuth (`oauth.rs:47 McpOAuthManager`, `auth.rs:13 OAuthTokens`, `:183 is_token_expired`). Depends on **axum** (`Cargo.toml:29`) for a local OAuth callback server — TS does this too (`auth.ts`, `server/routes/instance/httpapi/groups/mcp.ts`). |
| `opencode-server` | 6,832 | `packages/opencode/src/server/**` (28 files, 412 LOC excl. routes) **+ `packages/server` (1,682)** | `server/routes/instance/httpapi/**` (63 files, ~4.4k), `server/routes/instance/httpapi/server.ts:276-284`, `packages/server/src/routes.ts` | **🔴 FAITHFUL ON ROUTES, MISSING THE INSTANCE/HEADER LAYER — which is the entire §1.1 story.** Positives: `router()` (`routes.rs:43`), **106 `.route()` calls** across 13 nests (`/session`, `/provider`, `/config`, `/mcp`, `/file`, `/find`, `/permission`, `/project`, `/pty`, `/question`, `/tui`, `/global`, `/experimental`, `/plugin`) + top-level `/`, `/health`, `/event`, `/path`, `/vcs`, `/command`, `/agent`, `/skill`, `/lsp`, `/formatter`, `/auth/{id}`, `/doc`, `/log` (`routes.rs:44-71`). That covers every endpoint in §2 **except** `/instance/dispose` and `/path`'s worktree shape. axum 0.8 + ws (`Cargo.toml:26`), tower/tower-http, `portable-pty 0.8` for PTY, SSE at `/event` (`:47`) and `/{id}/stream` (`:130`). 66 DTOs (`:138-4211`) including `GlobalHealthResponse:3962`, `CreateSessionRequest:370`, `SendMessageRequest:710`, `SessionStatusInfo:357`.<br>**MISSING / UNVERIFIED:** (a) **no `x-opencode-directory` handling was found in the survey** — the Rust `ServerState` (`server.rs:112`) is a single global state struct, and there is **no crate for `InstanceStore` / `WorkspaceRoutingMiddleware` / `InstanceState`** anywhere in the 19-crate list. The entire multi-directory instance model (§1.1 hops B-H, §6.5) appears **absent**. (b) no remote-workspace proxying (TS `workspace-routing.ts:113-146`) — no `sync`/`control-plane`/`workspace` crate. (c) no `worktree` *service* crate — `server/src/worktree.rs` is 192 LOC (list/create/remove/prune) vs TS `worktree/index.ts` 623. (d) no `GlobalBus`-equivalent routing by directory. |
| `opencode-lsp` | 975 | `lsp/lsp.ts` + `tool/lsp.ts` | `tool/lsp.ts:56` | **FAITHFUL.** `LspClient` (`lib.rs:149`) with 19 methods (`start:166`, `goto_definition:395`, `completion:433`, `references:466`, `hover:498`, `document_symbol:526`, `rename:652`, call hierarchy `:682-744`), `LspClientRegistry:804` + `touch_file:852` ↔ TS's `Instruction`-triggered `LspTouchFile` callback. `detect_language:895`. |
| `opencode-watcher` | 303 | `Watcher` node (`Watcher.Event.Updated`, `edit.ts:120`) | `edit.ts:117-121` | **FAITHFUL.** `FileWatcher` (`:75`), `watch:109`, `unwatch:175`, `subscribe:104`, plus a **global singleton** `get_watcher:209` / `init_watcher:219`. Note: the singleton is *global*, not per-directory — so it does **not** reproduce TS's per-directory watcher duplication, which may be an improvement or a bug depending on intent. |
| `opencode-cli` | 5,551 | `cli/` | — | **OUT OF SCOPE** (binary composition root, 0 `pub` items). |
| `opencode-tui` | 21,418 | `packages/tui` (5,883) | — | **OUT OF SCOPE**, and 3.6x larger than the TS TUI — irrelevant to a server-only port but a large fraction of the Rust LOC. |

### 9.3 Crates with NO TS counterpart (i.e. Rust-only surface)

None material. Every one of the 19 crates maps to something. Conversely see 9.4.

### 9.4 TS subsystems with NO Rust counterpart — the gap list

| TS subsystem | LOC | Rust status |
|---|---|---|
| `packages/core/src/event.ts` durable commit + `session/projector.ts` | 638 + 455 | **ABSENT.** No `event`/`event_sequence` tables, no per-aggregate seq, no inline projectors, no replay idempotency. Rust writes directly via `PartRepository::upsert`. |
| `packages/opencode/src/project/*` + `core/src/project/sql.ts` + migrations `…_project_directories`, `…_project_dir_strategy` | ~600 | **ABSENT** (no project crate; `ProjectInfo` DTO exists at `server/routes.rs:3263` but no identity resolution). |
| `…/middleware/workspace-routing.ts` + `project/instance-store.ts` + `effect/instance-state.ts` + `server/…/lifecycle.ts` | 250+213+69+54 = 586 | **ABSENT.** No instance/workspace crate. ⇒ **The `x-opencode-directory` contract has no port.** |
| `packages/opencode/src/sync/*` + `core/src/control-plane/*` + `core/src/workspace*` + `server/.../middleware/proxy.ts` | ~1,500+ | **ABSENT** (no sync crate, no workspace crate, no proxy middleware). |
| `packages/codemode` + `tool/code-mode.ts` | 6,878 + 310 | **ABSENT** (no code-mode tool; Rust has `CodeSearchTool`/`BatchTool` instead). |
| `packages/protocol` + `packages/schema` (typed errors, `sessionsCursor`, `errors`) | 1,582 + 3,387 | **THIN.** Rust `ApiError` (`server/error.rs:9`) + 66 DTOs. Effect Schema codecs ⇒ hand-written serde. |
| `packages/core/src/{account,credential,share,permission}/*.sql.ts` + `data-migration.sql.ts` | — | **PARTIAL.** Rust has `session_shares` + `permissions`; **no account, credential, or data-migration tables.** |
| `packages/http-recorder` | 1,540 | **ABSENT.** |
| `packages/effect-drizzle-sqlite` / `effect-sqlite-node` | 3,238 + 168 | **N/A** — replaced by sqlx. |
| `InstanceState` unbounded-ScopedCache semantics (`effect/instance-state.ts:31-37`) | 69 | **N/A** — no equivalent construct; Rust has plain per-call state. Removes the §7.5 items #6/#7 by construction. |
| Plugin `@mention` file-reference resolution specifics | `prompt.ts:157-191`, `config/markdown.ts:5` | **PARTIAL.** Rust has `resolve_prompt_parts` (`prompt.rs:2615`) + `extract_file_references` (`prompt.rs:2676`) ⇒ ported. Agent-mention branch (TS `prompt.ts:176`) UNVERIFIED. |

### 9.5 Faithfulness verdict

**Partial, and divergent in three architecturally decisive ways.** The Rust tree is a competent,
near-complete re-implementation of the *session loop*, *tool system*, *permission model*, *provider layer*,
*agent registry*, *config loader* and *MCP client* — roughly 62.7k server-relevant LOC against ~110k of TS
server code, with module-for-module name correspondence in `opencode-session` and `opencode-tool`.

But it is **not a port** of three things:

1. **Persistence.** Rust is direct-write repository CRUD over 6 tables with a replay-every-startup DDL list.
   TS is event-sourced with per-aggregate sequences, inline projectors, and an append-only `event` log with
   **no pruning** (`/tmp/ts-anatomy-map.md` §7.5 #4). The Rust design is simpler and — given §7.1's
   `Semaphore.make(1)` over synchronous `node:sqlite` — almost certainly *better performing*. What is lost is
   replay/idempotency/audit. **This is a design decision, not an oversight, and the team must make it
   explicitly rather than discover it.**
2. **Multi-directory instances.** No `InstanceStore`, no `InstanceState`, no `WorkspaceRoutingMiddleware`, no
   `x-opencode-directory`. The Rust `ServerState` (`server/src/server.rs:112`) looks single-tenant.
   **If the subworker scheduler's contract depends on the header (it does — `opencode_client.py:101,270`), then
   the Rust reference as it stands cannot serve that contract.** UNVERIFIED whether `ServerState` has an
   undocumented per-directory map; the crate list contains no candidate module.
3. **Plugin host.** TS = in-process TS `import()` with sequential awaited hooks. Rust = JSON-RPC subprocess to
   an external bun/deno/node, with **no embedded JS engine anywhere in the resolved dependency graph**
   (definitive negative across workspace manifest + 19 crate manifests + the 125 KB `Cargo.lock`).
   Trade-off: IPC pays process-spawn + serialization per hook invocation (`tool.execute.before`/`after` fire on
   **every** tool call), but it needs no engine, no `pub(crate)` coupling into server internals, and it is the
   only approach in this comparison that survives the "oh-my-openagent is 27k TS files" constraint at all.

## CLAIMS I COULD NOT VERIFY

1. **Whether `GET /session/` (trailing slash) resolves.** `opencode_client.py:201` sends `/session/`; the route
   constant is `"/session"` (`groups/session.ts:29`). No `"/session/"` literal exists in the server tree. Effect
   `HttpRouter`'s trailing-slash normalization was not inspected (node_modules not searched). **This is a
   live-fragility item, not a confirmed break.**
2. **`model` in `POST /session` body → `BadRequest`.** Asserted by `opencode_client.py:255-257`; I did not read
   `Session.CreateInput`'s exact fields.
3. **Which message events besides `MessageUpdated` carry `...options` (durable).** I verified `Created`,
   `Updated`, `Deleted`, `MessageUpdated` explicitly and `PartUpdated` by inference from the projector being
   registered on it. `MessageRemoved` / `PartRemoved` not individually confirmed — though their projectors exist.
4. **Exact durable-event count per assistant turn.** Mechanism is confirmed; the number depends on provider SSE
   cadence and which branches fire. Quoted as "roughly 6-8 events / 35-55 statements", labelled MEDIUM.
5. **`chat.params`, `chat.headers`, `chat.message`, `permission.ask`, `command.execute.before`,
   `experimental.chat.system.transform`, `experimental.session.compacting`,
   `experimental.compaction.autocontinue`, `experimental.provider.small_model` trigger call sites.** Declared in
   `packages/plugin/src/index.ts` but I did not locate every server-side caller. Two hook dispatchers exist:
   the `Hook` triggers above and a second path through
   `LLMRequestPrep.prepare` (`session/llm.ts:106-113`) — I did not read `session/llm/request.ts` in full.
6. **Whether `packages/plugin/src/v2/*` is a live second plugin API or dead code.** Present, 16 files, not traced.
7. **Which LLM runtime is actually active in production.** `flags.experimentalNativeLlm` (`session/llm.ts:226`)
   selects `@opencode-ai/llm`; AI SDK is the documented default (`:271-277`). Which one the subworker server
   actually runs was not determined.
8. ~~The full `/tmp/OpenCodeRust/crates` inventory.~~ **RESOLVED** — 19 crates, 208 `.rs`, 89,698 LOC; see §9.
   Remaining Rust-side gaps carried into §9.4: no project crate, no instance/workspace crate, no sync crate,
   no codemode crate, no event-sourcing layer, and the duplicate `AgentError` at
   `opencode-agent/src/agent.rs:165` vs `executor.rs:13`.
9. **`packages/ui`, `packages/tui`, `packages/codemode` runtime relevance to the server.** Read only their LOC.
   `codemode` is gated behind `flags.experimentalCodeMode` (`registry.ts:118`, `session/tools.ts:388`).
10. **`Global.Path.*` concrete values.** Read as symbols only (`packages/core/src/global`); I cite them by name,
    not by resolved path, except `Global.Path.data` for the DB (inferred from `database/path.ts` usage).
11. **Whether MCP clients are per-directory or global.** `MCP.Service` is in the global `app` LayerNode group
    (`server.ts:250`) but `mcp.clients()` is consumed from `InstanceState`-scoped callers. Not resolved.
12. **Runtime behavior of `Effect.orDie` on `ctx.ask` denial** (`session/tools.ts:89`) — whether the AI SDK
    surfaces the defect as a tool error or kills the turn was not traced end to end.

### 9.6 Rust-side claims I could not verify (inherited from the delegated survey)
- Whether `/tmp/OpenCodeRust` **compiles**. `cargo check` was never run. A `target/debug/build/libsqlite3-sys-*/out/bindgen.rs`
  exists, so a build reached sqlx's bindgen stage, but success is unproven. This adjudicates the `AgentError`
  collision and whether `opencode-tool/src/lsp_tool.rs` builds under `default = []`.
- Whether `ServerState` (`opencode-server/src/server.rs:112`) has any per-directory map. No candidate module
  appears in the crate list, but I did not read `server.rs` line by line.
- Exact `sqlx` usage file:line in `opencode-session` and `opencode-cli` (manifest dep + file counts only).
- `opencode-command/commands/` and `opencode-server/static/` non-`.rs` contents.
- Whether `BuiltinAgent::all() -> [BuiltinAgent; 6]` (`opencode-agent/src/agent.rs:53`) omits one of TS's 7
  built-ins (`build, plan, general, explore, compaction, title, summary`).

---

## APPENDIX A — TUI-decoupling mini-plan (task 10)

Inventory + exact shim/call-site list + verification. **No code changes made.**

**Headline: this is a ~196-LOC inline job plus a wholesale CLI/TUI subtree delete. It is much smaller than
the task description implies, because the server's `/tui/*` HTTP group does not import `@opencode-ai/tui` at
all.** Read §2 before sizing it.

---

### 1. Complete `@opencode-ai/tui` import inventory

Exhaustive: `grep -rn --include='*.ts' --include='*.tsx' -E '@opencode-ai/tui' packages/`.
**36 import sites across 27 files.** Only **one** package declares the dependency:
`packages/opencode/package.json:95` (`"@opencode-ai/tui": "workspace:*"`). `packages/core`,
`packages/server`, `packages/schema`, `packages/llm`, `packages/plugin`, `packages/protocol` are **already clean**.

### 1.1 Group A — server-critical shims (MUST be inlined)

| # | File | Line | Statement | Inline target | LOC |
|---|---|---|---|---|---|
| A1 | `packages/opencode/src/util/record.ts` | 1 | `export * from "@opencode-ai/tui/util/record"` | `packages/tui/src/util/record.ts` | **3** |
| A2 | `packages/opencode/src/util/error.ts` | 1 | `export * from "@opencode-ai/tui/util/error"` | `packages/tui/src/util/error.ts` | **193** |

`packages/tui/src/util/record.ts` is the whole file:
```ts
export function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value)
}
```
`packages/tui/src/util/error.ts` exports exactly 4 symbols: `cliErrorMessage` (`:5`), `errorFormat` (`:108`),
`errorMessage` (`:136`), `errorData` (`:158`). It is **pure functions, zero imports from tui internals** — inline
is a clean copy, no rewrite.

**Consumers of A1+A2 outside the CLI — 18 server files**, all of which break if the shims are not inlined first:
`src/config/config.ts` (`:17` imports `isRecord`), `src/control-plane/workspace.ts`, `src/format/index.ts`,
`src/installation/index.ts`, `src/plugin/index.ts`, `src/plugin/install.ts`, `src/plugin/openai/ws-pool.ts`,
`src/plugin/openai/ws.ts`, `src/plugin/shared.ts`, `src/provider/provider.ts`, `src/session/llm/ai-sdk.ts`,
`src/session/llm/native-request.ts`, `src/session/llm/native-runtime.ts`, `src/session/message-v2.ts`,
`src/session/processor.ts`, `src/session/retry.ts`, `src/session/tools.ts`, `src/skill/index.ts`,
`src/worktree/index.ts`.

**Inline destination:** `packages/opencode/src/util/record.ts` and `packages/opencode/src/util/error.ts`,
replacing the re-export line with the copied body. Keep the `@/util/error` import specifiers unchanged so all
18 call sites need **zero edits**. This is the whole reason to inline rather than re-point.

### 1.2 Group B — CLI-only, DELETE (no inline needed)

Every one of these is reachable only from `src/cli/**`, which a server-only port drops entirely.

| # | File:line | Import |
|---|---|---|
| B1 | `src/cli/cmd/tui.ts:7` | `errorMessage` from `/util/error` |
| B2 | `src/cli/cmd/tui.ts:12` | `type EventSource` from `/context/sdk` |
| B3 | `src/cli/cmd/tui.ts:16` | `win32InstallCtrlCGuard` from `/terminal-win32` |
| B4 | `src/cli/cmd/attach.ts:3` | `errorMessage` from `/util/error` |
| B5 | `src/cli/cmd/run/runtime.lifecycle.ts:15,16` | `openEditor`, `registerOpencodeKeymap` |
| B6 | `src/cli/cmd/run/footer.ts:32` | `OpencodeKeymapProvider` |
| B7 | `src/cli/cmd/run/footer.prompt.tsx:11,26` | `normalizePromptContent`, `OPENCODE_BASE_MODE`/`useBindings` |
| B8 | `src/cli/cmd/run/footer.view.tsx:13,14,37` | spinner register/frames, 5 keymap symbols |
| B9 | `src/cli/cmd/run/footer.subagent.tsx:4,6` | `registerOpencodeSpinner`, `SPINNER_FRAMES` |
| B10 | `src/cli/cmd/run/theme.ts:674` | dynamic `import("@opencode-ai/tui/context/theme")` |
| B11 | `src/cli/cmd/run/runtime.boot.ts:9,10` | `resolve` from `/config`, `TuiConfig` |
| B12 | `src/cli/tui/layer.ts:1` | `run as runTui`, `type TuiInput` |
| B13 | `src/cli/logo.ts:1` | `export * from "@opencode-ai/tui/logo"` (11 LOC) |
| B14 | `src/cli/cmd/prompt-display.ts:1` | `export * from "@opencode-ai/tui/prompt/display"` |
| B15 | `src/util/locale.ts:1,2` | `/util/locale` (86 LOC) — **only importers are `src/cli/cmd/session.ts:8` and `src/cli/cmd/run/{splash,tool,runtime.lifecycle,subagent-data,runtime.queue,session-data,turn-summary}.ts`** ⇒ delete |
| B16 | `src/plugin/tui/runtime.ts:18,19,32,39,42,45` | `errorData`/`errorMessage`, `isRecord`, `hasTheme`/`upsertTheme`, `HostPluginApi`/`HostSlots`, `createCommandShim`, `createPluginRuntime` |
| B17 | `src/plugin/tui/internal.ts:1` | `createBuiltinPlugins`, `type BuiltinTuiPlugin` |

`src/config/tui.ts` (276 LOC), `src/config/tui-migrate.ts` (132), `src/config/tui-host-attention.ts` (21) are
imported **only** by `src/plugin/tui/runtime.ts:16,20` (B16) and `src/cli/cmd/run/runtime.boot.ts:10` (B11).
⇒ **delete the whole `src/config/tui*.ts` trio, do NOT inline `TuiConfig`.** See §2.1 for the verification that
nothing server-side needs them.

### 1.3 Group C — build-time script, not runtime

| # | File:line | Import | Note |
|---|---|---|---|
| C1 | `packages/opencode/script/schema.ts:5` | `TuiConfig` from `/config` | Config-JSON-schema **generator script**, not the server. Either stub `TuiConfig` to `Schema.Unknown`, or accept that `script/` is not part of the server artifact. |

### 1.4 Group D — dead code, DELETE outright

| # | File | Finding |
|---|---|---|
| D1 | `packages/opencode/parsers-config.ts` (1 line: `export { default } from "@opencode-ai/tui/parsers-config"`) | **Nothing imports it.** Repo-wide grep for `parsers-config` returns only: the file itself, `packages/tui/package.json:38` (tui's own export map), and `packages/tui/src/routes/session/index.tsx:58` which imports tui's copy by **relative path**. Also `packages/opencode`'s exports map is `{"./*": "./src/*.ts"}` (`packages/opencode/package.json`), and this file is at the **package root, not under `src/`** ⇒ it is not even publicly exported. **Dead. Delete.** |

### 1.5 Group E — test-only, does not block a server-only port

15 sites in `packages/opencode/test/**`: `test/fixture/tui-environment.tsx:7`, `test/fixture/tui-sdk.ts:2`,
`test/fixture/tui-plugin.ts:3`, `test/fixture/tui-runtime.ts:3,5`, `test/cli/cmd/tui/attention.test.ts:3,4`,
`test/cli/tui/editor-context-zed.test.ts:6`, `test/cli/tui/editor-context.test.tsx:6,10`,
`test/cli/tui/plugin-loader.test.ts:15`, `test/cli/run/footer.view.test.tsx:8`,
`test/cli/run/runtime.boot.test.ts:3`.
Every one is under `test/cli/` or `test/fixture/tui-*`. `test/cli/**` dies with the CLI. The three
`test/fixture/tui-*` fixtures are only consumed by `test/cli/**` — **UNVERIFIED**: I did not trace their
reverse importers. Action: delete `test/cli/**` and any `test/fixture/tui-*` that still resolves.

---

### 2. 🔴 THE FINDING THAT MAKES THIS SMALL: the server's TUI HTTP surface has zero tui imports

`src/server/routes/instance/httpapi/groups/tui.ts` (208 LOC) + `handlers/tui.ts` (131 LOC) declare
**18 routes under `/tui/*`** (`groups/tui.ts:11` `const root = "/tui"`, `TuiPaths` at `:36-44`):
`appendPrompt`, `openHelp`, `openSessions`, `openThemes`, `openModels`, `submitPrompt`, `clearPrompt`,
`executeCommand`, …

**Neither file imports `TuiConfig`, `theme`, or `keybind`.** Verified three ways:
`grep -n 'TuiConfig\|theme\|keybind' groups/tui.ts handlers/tui.ts` returns **only the string literals
`"open-themes"`** (`:40`) and its OpenAPI summary/description (`:94-95`); their full import lists are
`@/server/tui-event` + `@/server/shared/tui-control` + `effect` + local middleware only — **zero
`@opencode-ai/tui`, zero `theme`, zero `keybind`**. The group is a **pure control channel**: it publishes
`TuiEvent` bus events instructing an *already-connected* TUI client to open a dialog. It carries no theme or
keybind types over the wire.

Side effect of deleting the group: `src/server/shared/tui-control.ts` (28 LOC) becomes dead — its only
consumers are `handlers/tui.ts:6`. But **`src/server/tui-event.ts` must be KEPT** — `src/mcp/index.ts:28`
imports `TuiEvent` from it, and it is a 1-line re-export of `@opencode-ai/schema/tui-event` (schema package,
not tui package), so it does not count against the TUI budget.

Two consequences:

- **2.1 — `TuiConfig` can be deleted, not inlined.** The service is
  `export class Service extends Context.Service<Service, Interface>()("@opencode/TuiConfig")` with
  `export const node = LayerNode.make(...)` at `src/config/tui.ts:47,262`.
  **Verified: `TuiConfig` does not appear in `src/server/routes/instance/httpapi/server.ts`.** It is not in the
  `app` LayerNode group (`server.ts:212-269`) nor in any handler wiring. Its only consumers are B11 and B16,
  both CLI-only. ⇒ delete `tui.ts` + `tui-migrate.ts` + `tui-host-attention.ts` (429 LOC) with no server impact.
- **2.2 — the port does NOT need to serve a theme/keybind HTTP surface at all.** Nothing in
  `/tmp/ts-anatomy-map.md` §2 (the 11 endpoints `opencode_client.py` calls) touches `/tui/*`. And
  `opencode_client.py` opens **no SSE connection** (verified — it polls `GET /session/status`). So the whole
  `/tui/*` group, plus `handlers/tui.ts`, can be **deleted** from a server-only port with zero consumer impact.
  What must still serve instead: **nothing.** If a human operator later wants a web UI, it is a different project.

---

### 3. Exact shim list to inline (final, minimal)

| Action | Source to copy | Destination | LOC | Edits needed at call sites |
|---|---|---|---|---|
| Inline | `packages/tui/src/util/record.ts` (whole file) | `packages/opencode/src/util/record.ts` | 3 | **none** |
| Inline | `packages/tui/src/util/error.ts` (whole file, 4 exports) | `packages/opencode/src/util/error.ts` | 193 | **none** |
| Delete | `packages/opencode/parsers-config.ts` | — | 1 | none (dead) |
| Delete | `src/cli/**`, `src/cli/tui/layer.ts`, `src/logo.ts`, `src/cli/cmd/prompt-display.ts`, `src/util/locale.ts` | — | — | CLI entrypoint removal |
| Delete | `src/plugin/tui/**` | — | — | none server-side |
| Delete | `src/config/tui.ts`, `src/config/tui-migrate.ts`, `src/config/tui-host-attention.ts` | — | 429 | none (§2.1) |
| Delete | `src/server/routes/instance/httpapi/groups/tui.ts` + `handlers/tui.ts`, remove `tuiHandlers` from the `instanceApiRoutes` handler list (`server.ts:170`) | — | 339 | none (§2.2) |
| Delete, **partial** | `src/server/shared/tui-control.ts` (28 LOC) — becomes dead | — | 28 | `nextTuiRequest`/`submitTuiResponse` used only by `handlers/tui.ts:6` |
| **KEEP** | `src/server/tui-event.ts` (1 LOC) | — | 1 | ⚠️ **also imported by `src/mcp/index.ts:28`** — it is a 1-line re-export of `@opencode-ai/schema/tui-event`, i.e. *schema*, not tui-package. Do not delete. |
| Delete dep | `packages/opencode/package.json:95` `"@opencode-ai/tui": "workspace:*"` | — | — | — |
| Delete pkg | `packages/tui/` entirely (5,883 LOC) + its `turbo` entry | — | — | — |
| Stub or drop | `script/schema.ts:5` `TuiConfig` import | — | 1 | script-only |

**Net effect on the server artifact: +196 LOC inlined, −~7,000 LOC of TUI package removed, −0 call-site edits
outside the CLI.** The reason call-site edits are zero is that Group A shims keep the `@/util/error` /
`@/util/record` specifiers; only the re-export line inside each shim changes.

⚠️ **Sequencing note (lead's call, not mine):** A2 must be inlined **before** the CLI is deleted, because 15 of
the Group B call sites also import `errorMessage`. Inlining first makes them all resolve locally; deleting the
CLI then needs no edits. Doing it in the other order produces ~15 spurious type errors.

---

### 4. Verification — server builds, typechecks, and boots with zero `@opencode-ai/tui` imports

Run from `/Users/vakandi/Documents/opencode-server-rust`. Scripts verified present in
`package.json` and `packages/opencode/package.json`.

**V0 — the gate (must be zero, and is currently 36):**
```bash
grep -rn --include='*.ts' --include='*.tsx' -E '@opencode-ai/tui' packages/ \
  | grep -v node_modules | grep -v '/test/'
# expect: 0 lines. Also assert the dep is gone:
grep -n '"@opencode-ai/tui"' packages/*/package.json   # expect: no output
```

**V1 — typecheck:**
```bash
bun turbo typecheck              # root script -> tsgo --noEmit per package
# per-package, faster feedback:
bun run --cwd packages/opencode typecheck
bun run --cwd packages/core typecheck
bun run --cwd packages/server typecheck
```

**V2 — build:**
```bash
bun run --cwd packages/opencode build     # -> script/build.ts
```

**V3 — server tests (the real regression gate):**
```bash
bun run --cwd packages/opencode test                  # bun test --timeout 30000 --only-failures
bun run --cwd packages/opencode test:httpapi           # 3x httpapi-exercise.ts, --fail-on-missing --fail-on-skip
```
`test:httpapi` is the important one: it exercises the live `HttpApi` tree and will catch a
mis-wired `instanceApiRoutes` after `tuiHandlers` is removed from `server.ts:170`.
Per `turbo.json`, `opencode#test` `dependsOn: ["^build"]`, so V2 must pass first.

**V4 — boot without the CLI/TUI:**
```bash
bun run --cwd packages/opencode src/index.ts serve --port 5655
curl -s localhost:5655/global/health                  # {"healthy":true,"version":"..."}
curl -s -H 'x-opencode-directory: /Users/vakandi/EliaAI/subworkers' \
     -X POST localhost:5655/session -d '{}' | head -c 400
curl -s -o /dev/null -w '%{http_code}\n' -X POST localhost:5655/tui/open-themes   # expect 404 after delete
```
That second `curl` is the positive proof the `x-opencode-directory` → instance chain still boots after the
inlining (§1 hops B–H of the anatomy map). The `404` on `/tui/open-themes` is the intended new behaviour.

**V5 — no runtime TUI import leaks in the server graph:**
```bash
bun run --cwd packages/opencode dev 2>&1 | grep -i 'tui'   # expect no module-resolution errors
# and confirm the served OpenAPI still contains the 11 consumer endpoints:
curl -s localhost:5655/doc | grep -oE '"/(global/health|session[^"]*|provider|instance/dispose)"' | sort -u
```

---

### 5. Risks and things I could not verify

1. **UNVERIFIED — reverse importers of `test/fixture/tui-{environment,sdk,plugin,runtime}.*`.** I grepped forward
   from the tui imports; I did not prove those four fixtures are unreachable from non-CLI tests. If one is used
   by `test/server/**`, it needs rewriting, not deleting.
2. **UNVERIFIED — whether anything outside `packages/opencode` imports `@opencode-ai/tui`'s `util/*` subpaths.**
   My grep covered all of `packages/`, so I am confident for the workspace, but a consumer outside
   `packages/` (or a published `@opencode-ai/plugin`) could be relying on the re-exports. `packages/plugin`'s
   own `package.json` does not list `@opencode-ai/tui`, so this is low risk.
3. **UNVERIFIED — `bin/opencode` behavior after CLI removal.** `packages/opencode/package.json` declares
   `bin: {opencode: "./bin/opencode"}`. I did not read `bin/opencode`; it presumably dispatches into
   `src/cli/`. The `serve` subcommand in V4 must survive, so the bin shim needs to route `serve` directly to
   the server entrypoint rather than through the CLI arg parser. **This is the one real behavioural risk in the
   whole change** — verify before deleting anything under `src/cli/`.
4. **`script/schema.ts`** (C1) regenerates the published config JSON schema. If that schema is a published
   artifact, stubbing `TuiConfig` to `Schema.Unknown` changes it. Decide: keep a minimal local `TuiConfig`
   schema, or drop `script/` from the server artifact.
5. **`src/config/config.ts` only needs `isRecord`** (verified at `:17`), not `TuiConfig`. So the server's config
   loader survives Group B/C deletion untouched. This is the single most load-bearing fact in §2.
6. **Not verified: that the build currently passes.** I ran no build, typecheck, or test. V1-V3 are the
   *acceptance* criteria for the change, not a claim about today's state. Establish the baseline first — if
   `bun turbo typecheck` is already red, V1 cannot distinguish my breakage from pre-existing breakage.

---

### 6. One-paragraph summary for the plan

Severing TUI is **not** the risky, sprawling refactor the task framing suggests. Only **two** shims
(`util/record.ts` = 3 LOC, `util/error.ts` = 193 LOC) are genuinely server-critical, and because they are
one-line re-exports, inlining their bodies requires **zero call-site edits** across the 18 server files that
consume them. Everything else — 15 CLI call sites, the 429-LOC `config/tui*.ts` trio, the 1,951-LOC
`opencode-plugin`-shaped `src/plugin/tui/**`, and the 339-LOC `/tui/*` HTTP group — is reachable **only** from
`src/cli/**`, and the `/tui/*` group imports `@opencode-ai/tui` **not at all** (it is a bus-event control
channel with zero theme/keybind types). Two files are outright dead: `packages/opencode/parsers-config.ts`
(imported by nobody, and not even covered by the package's `{"./*": "./src/*.ts"}` exports map) and the three
`test/fixture/tui-*` files (pending reverse-importer verification). Net: **+196 LOC inlined, ~7,000 LOC of
`packages/tui` (5,883) plus ~800 LOC of in-repo TUI glue deleted, 0 call-site edits, one behavioural risk
(`bin/opencode` `serve` routing).** Do the inlining before the CLI deletion or you eat ~15 spurious type errors.