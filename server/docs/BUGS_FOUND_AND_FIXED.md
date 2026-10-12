# Bugs found and fixed — working notes

Written while bringing the Rust OpenCode server to parity with the TypeScript original and to
100 concurrent sessions in 6 GB. Every entry is a defect that was **reproduced**, then fixed,
then re-verified. File/line references are from this tree.

Rule that governed all of it: **measure, don't guess.** Twice the obvious fix was wrong and
only a measurement caught it — see §14 and §15.

---

## PART 1 — The livestream carried nothing (the core defect)

### 1. The update hook discarded the agent's entire output
**Symptom** — `GET /event` streamed two useless events (`session.status`, `session.updated`)
and **zero** part data. No text, no reasoning, no tool activity, ever.

**Cause** — `routes.rs`, the `update_task` loop fed by the engine's snapshot hook. The hook
receives a **full `Session` snapshot** (every message, every part, every delta) and stored it,
then broadcast a three-field object:

```rust
update_state.broadcast(&serde_json::json!({
    "type": "session.updated",
    "sessionID": snapshot_id,
    "source": "prompt.stream",
}).to_string());
```

The payload it had just been handed was thrown away.

**Fix** — new `crates/opencode-server/src/events.rs`: `diff_parts()` / `PartIndex` emit only
what changed, shaped like the reference server (`message.part.updated`, `message.part.delta`).

**Verified** — live SSE now carries
`message.part.updated  part.type=text  text='reply with one word: pong'`.

---

## 2. My own hot-path fix made the 100-session goal worse

Wiring the snapshot channel for part 1 introduced a per-tick `Session` **clone** (~20×/sec per
streaming session) into a 1024-deep channel **per session**. At 100 concurrent sessions that
is ~2,000 whole-session clones/second — and a long session turns one clone into a large copy.

**Fix** — the hook is `Arc<dyn Fn(&Session) + Send + Sync>`: **synchronous**. So it can fold
the snapshot in place and emit straight onto the bus. Added `StreamTracker` (keeps only the
**last message's** parts, since streaming only appends to it). Removed the clone, the queue,
the spawned task and the drop counter entirely.

**Verified** — `tracker_emits_nothing_when_folded_twice`,
`tracker_state_is_bounded_by_the_last_message_not_the_session` (500 historical messages must
not add events).

---

## 3. `PartInfo` DTO is lossy and inconsistent three ways

`PartInfo` (`routes.rs`) exposed only `text|file|tool_call|tool_result`, and the same resource
was serialised three different ways:

- `get_message` → raw `PartType` enum → camelCase `stepStart`, `toolCall`
- `list_messages` → `PartInfo` via `part_type_name` → `step_start`, `tool_call`
- reference server → kebab `step-start`, and `tool` (not `toolCall`)

**Fix** — one adapter, `part_to_ts_json()`, mapping all 13 `PartType` variants to the reference
shape (TS folds a call and its result into **one** `tool` part with a `state.status`).

---

## 4. `sessionID` on a part was populated from `message_id`

**Symptom** — every part claimed to belong to a session named after a message.

**Cause** — `events.rs`, a convoluted no-op:
`"sessionID": part.message_id.as_deref().and_then(|_| part.message_id.clone())`.

Nothing asserted this field, so it shipped silently. **Fix** — thread the real session id
through; added `s1_part_session_id_is_the_session_not_the_message`, which fails on the old
code with a precise message.

---

## 5. Message list was flat, not `{info, parts}`

`GET /session/{id}/message` returned a flat `Vec<MessageInfo>` with snake_case
(`session_id`, `created_at`) while its own sibling `get_message` returned
`{"info":…,"parts":…}` with camelCase. A client reading `msg["info"]["role"]` always got `{}`.

**Fix** — `messages_to_ts_json()`; both endpoints now use it.

---

## 6. `POST /session` silently dropped `agent`

`CreateSessionRequest` had no `agent` field and no `deny_unknown_fields`, so it was discarded.
**Fix** — added, persisted under the same `session.metadata["agent"]` key the prompt path uses.

## 7. `GET /session/status` had no `type` field

Clients read `status["type"] == "busy"`. The struct had `status|idle|busy`. **Fix** — added
`#[serde(rename = "type")] run_type` carrying `idle|busy|retry`; existing fields kept.

## 8. `POST /session/{id}/message` rejected the reference payload

`content: String` was **required** and there was no `parts`, so every real client got
`422 missing field 'content'`. **Fix** — `content` optional, `parts[]` accepted, `model`
accepts string *or* `{providerID, modelID}`, and empty/malformed input returns **400 naming the
offending index** rather than 422.

---

## PART 2 — Why no model provider worked at all

`POST /prompt {"model":"opencode/space-bunny-free"}` → **`Provider not found: opencode`**.
This was **four stacked bugs**, and the first one silently disabled everything.

### 9. The model catalogue was read from the wrong path (the real blocker)

`load_models_dev_cache()` read **only** `dirs::cache_dir()/opencode/models.json`
= `~/Library/Caches/...` on macOS. The catalogue actually lives at
`~/.cache/opencode/models.json` (XDG — where the TypeScript server writes it).

So the loader returned an **empty** `ModelsData` → `state.providers` was empty → bootstrap
logged *"No providers registered … falling back to direct env registration"* → **only**
env-key providers survived (`openrouter`, via `OPENROUTER_API_KEY`). Every keyless provider
vanished with no error.

**Fix** — probe platform cache, `$XDG_CACHE_HOME`, `~/.cache`, temp; **warn loudly** when none
is found instead of silently returning empty.

**How it was found** — existing `tracing::debug!` counters in `bootstrap_registry` showed
`registered: 0, no-concrete: 0, fallback: 1`. That "0 providers attempted" line is what proved
it was the catalogue, not the provider logic.

### 10. Provider activation required a credential, and `autoload` was dead code

Activation only merged a provider when an env var key existed. The custom loaders compute an
`autoload` flag for exactly the keyless case — and **nothing ever read it**.

**Fix** — activate on `autoload` too, and apply the loader's paid-model blacklist directly
(`ProviderPatch` cannot carry a blacklist).

### 11. `create_concrete_provider("opencode")` bailed out without a key

Both the key and the base URL used `?`, returning `None` and dropping the provider.
Note the loader already injected the sentinel `options["apiKey"] = "public"` — the design
already intended keyless zen.

**Fix** — default to an empty key and `DEFAULT_ZEN_BASE_URL` (`https://opencode.ai/zen/v1`).

### 12. Tools were serialised bare → 400 on every OpenAI-compatible provider

`ChatRequest.tools` is a flat `ToolDefinition { name, description, parameters }`, so serde
emitted `"tools":[{"name":…}]`. OpenAI-compatible endpoints require
`{"type":"function","function":{…}}` and reject the **entire request**.

**Proven by direct curl** — bare tools → `400 invalid_request_error`; wrapped → `200`.

**Fix** — wrap in `OpenAIProvider::build_request_body`. This affected *every* OpenAI-compatible
provider, not just zen.

**Result** — all three livestream requirements verified live with a real model and no
credential: AI text, AI tools (`bash` really executed; the model quoted its output), and AI
reasoning.

---

## PART 3 — Concurrency and the 100-session goal

### 13. 195 × `UNIQUE constraint failed: messages.id`

`sync_sessions_to_storage` deletes and re-inserts **every message of every session**, from
**22 routes**, with no transaction and no lock. Concurrent requests interleaved their
inserts and collided on the primary key.

**Fix (correctness)** — `ServerState.sync_gate: tokio::sync::Mutex<()>` serialises the sync.

### 14. The mutex fixed corruption but caused a timeout storm — measured

| run | created | notes |
|---|---|---|
| before | 100/100 | 195 UNIQUE violations |
| after mutex | **16/100** | 84 timed out |

Serialising 100 full rewrites into a queue is not a fix, it is a different outage.
**This was caught by re-running the load test, not by reasoning.**

**Fix (root cause)** — `sync_session_to_storage(session_id)` persists **one** session and
shares a `write_session()` helper. Cost drops from O(all messages) to O(one session).
Converted 21 of 22 call sites to it. The two that needed judgement:
`fork_session` must persist the **fork**, not the source `id` (which was also already moved).

**Verified** — 14/100 → **100/100**, peak 115 MiB = **1.9 % of the 6 GB budget**.

### 15. `SessionManager.events` grew without bound

Pushed at 12 sites; `drain_events` had **zero callers**; and `Created`/`Updated` each embed a
**full `Session` clone including all messages**.

**RED** — `event_buffer_is_bounded`: *"event buffer grew to 10000 entries after 10k operations"*.
**Fix** — `MAX_BUFFERED_EVENTS = 1024` + `push_event()` that drops the oldest half once full
(amortised O(1) rather than shifting on every push). All 12 sites rewritten.

### 16. Progressive degradation (partly fixed, honest status)

After several successive load runs, a **single** `POST /session` on an *idle* server took >20 s
and SQLite reported *database is locked*, with 747 sessions accumulated. Cause: the full-rewrite
sync still holds `sync_gate` while rewriting everything, so it grows without bound and blocks
later work. **All 22 call sites now use per-session persistence**, so the unbounded rewrite is
no longer on any request path; a single `POST /session` went **>20 s → 9 ms** and the clean
100-session run passed (100/100 created, peak 115 MiB = 1.9 % of budget).

If a full rewrite is ever genuinely needed (a bulk import, or reconciling rows orphaned by a
crash), call `sync_sessions_to_storage` deliberately from a background task — never from a
request handler.

---

## PART 4 — Build and tooling traps (cost me real time)

### 17. `cargo build -p opencode-server` does **not** rebuild the binary
The executable comes from **`opencode-cli`**. `-p opencode-server` builds only the library, so
I verified a live endpoint against an **8-minute-stale binary** and briefly believed my change
had not worked. Always `cargo build -p opencode-cli` (or `--workspace`) and **check the binary
mtime** before trusting a live result.

### 18. Never pipe cargo through `head`
`cargo build … | head -30` SIGPIPEs the compiler mid-write; the binary is never written and the
next invocation reports "Finished" from a stale fingerprint. Redirect to a file instead.

### 19. SSE lines are not JSON
An SSE log interleaves `event: message` with `data: {...}`, and the data line is invalid JSON
until the prefix is stripped. `jq .` errors with *Invalid numeric literal*; `jq -R 'fromjson?'`
silently returns **nothing**. Correct form:
`jq -R 'select(startswith("data: ")) | ltrimstr("data: ") | fromjson? // empty'`.
`mktemp` on macOS also needs `XXXXXX` last.

### 20. Wrong assumptions I had to correct
- "The engine is never invoked from HTTP" — **false**; `prompt_with_update_hook` was already
  called, the hook just discarded its payload.
- "`PartInfo` drops reasoning" — **false**; reasoning serialises, it was merely unlabelled.
Both were caught by review and confirmed by reading source before acting.

---

## PART 5 — Environment problems (not code)

- **The OpenRouter key in the environment is dead.** `/api/v1/models` returns `200` with *no
  key at all* (it is public), but `/api/v1/key` and `/credits` return `401 User not found`.
  A well-formed key with a dead account behind it.
- **opencode zen needs no credential.** Verified directly; it returns `reasoning_content`.
- **The local proxy is dead** — `proxy-local` on `127.0.0.1:18889` reports `liveTunnels: 0`, so
  with `HTTP_PROXY` set every outbound call fails with *error sending request*. Direct egress
  works. Start the server with the proxy vars unset.
- **Disk at 98 %.** 18 GB of it is the **TypeScript** opencode database (`event` table, 1.43 M
  rows) — unrelated to this project, but it is an emergency on the host machine.
- **Two pre-existing test failures** in `opencode-session`: `instruction::tests::test_find_up_*`
  fail on macOS because `find_up` canonicalises (`/private/var/...`) while the test compares
  against the raw temp path (`/var/...`). Proven present at pristine HEAD; unrelated to this work.

---

## Verifying the fixes

```bash
cargo build -p opencode-cli                                     # the executable lives here
cargo test -p opencode-server --test sse_livestream             # 16
cargo test -p opencode-server --test wire_contract              #  8
cargo test -p opencode-session event_buffer_is_bounded          #  1
cargo test -p opencode-session prompt_with_update_hook          # engine canary

# 100 concurrent sessions, 6 GB budget
env -u HTTP_PROXY -u HTTPS_PROXY -u http_proxy -u https_proxy \
  ./target/debug/opencode serve --port 4097 --hostname 127.0.0.1 &
python3 scripts/loadtest.py --n 100 --hold 40 --budget-mb 6144 --model opencode/space-bunny-free

# run a session from a file and render its livestream
./scripts/run-prompt-md.sh PROMPT.md http://127.0.0.1:4097
```

`scripts/loadtest.py` reports created/accepted/failures, peak RSS and budget used.
**Run it from a clean database** — a stale one accumulates sessions and will make a healthy
server look wedged (§16).

## Still open

1. ~~`stream_message` still uses the full-rewrite sync~~ — **done.** All 22 call sites now use
   per-session persistence; `persist_sessions_if_enabled` has no remaining callers. Verified
   end-to-end: a session created with `agent`, prompted from a file, and its assistant answer
   persisted with zero sync errors.
2. Tool-**result** parts carry no tool name: Rust models a call and its result as two sibling
   parts, and the adapter collapses them, so the result side has no `tool` field. The call side
   is correct; only the paired result is anonymous.
3. Under the free zen model, ~21/100 sessions were still `busy` at the end of a 40 s window.
   Upstream latency, not a crash and not memory — but not yet explained, and worth a provider
   timeout/audit.
4. `SessionManager::events` is now bounded (1024) but still holds full `Session` clones. A
   `Created`/`Updated` event for a large session is expensive to clone; if this shows up in a
   profile, store ids and re-read on demand instead.

---

## PART 6 — oh-my-openagent (omo) parity

omo is the plugin that supplies most of the agent behaviour. It is bundled as a compiled
bundle and loaded through `plugin-host.ts` as a subprocess plugin, so every omo capability has
to survive four hops: **capture → schema conversion → registration → invocation**.

Measured surface of the loaded plugin: **31 `tool.*` hook names offered, 26 registered**
(the difference is omo's own `disabled_tools` filtering), inside a total omo surface of
**14 tools / 23 features / 56 hooks**.

### 21. omo was rejected at load with `-32600`

omo exports `{ id, server }`, not a bare function. The host checked
`typeof mod.default === "function"` and rejected it, so **every omo feature was silently
absent** — no omo agents, no omo tools, no omo hooks.

Fixed in `crates/opencode-plugin/host/plugin-host.ts` by also accepting `entry.server`.
Verified: zero `-32600`, and omo's agents are listed.

### 22. omo's tool definitions were dropped, and their schemas could not have survived

Two separate defects stacked here.

**Dropped.** `hooks.tool` is an *object map* of tool definitions, but the host's hook loop only
accepted `typeof === "function"`. It therefore skipped the entire tool registry.

**Unserialisable even once forwarded.** omo's `tool()` helper is the identity function
(`function tool(input) { return input }`, `dist/index.js:115769`), so a tool definition holds
live **zod** objects in `args` and a real **function** in `execute`. `JSON.stringify` destroys
both — a forwarded definition would have arrived with a name and nothing else.

The way out was already inside omo: `normalizeToolArgSchemas` attaches a `_zod.toJSONSchema()`
method to every argument schema (`dist/index.js:159931`), which reaches the tools that end up
in `hooks.tool`. So the host converts **inside bun, where zod is live**, via
`jsonSchemaFromArgs`. Never try to serialise a zod schema across a process boundary.

### 23. Captured tools were stored nowhere and registered nowhere

`InitializeResult` deserialised the tools and then dropped them on the floor. Added the field,
storage and an accessor on `PluginSubprocess`.

The injection point is **not** obvious and the naive version is a silent no-op:
`create_default_registry()` builds a **fresh** registry on every prompt (used both for the
schema list sent to the provider and for execution), so registering into any long-lived
registry is discarded before use. Tools are therefore recorded in a process-global store at
load time and re-registered into each freshly built registry.

### 24. The real bottleneck: `tool.definition` fired once per tool

`ToolRegistry::list_schemas()` fired the `tool.definition` hook **once per tool**, awaiting a
JSON-RPC round-trip each time. Each call can burn the full 30 s client timeout, and the cost
**scales with tool count** — so adding omo's 26 tools made prompts dramatically slower:

- 20 native tools → 4 timeouts
- +26 omo tools → 10 timeouts (≈300 s of pure stall per prompt)

`list_schemas` also held the `tools` read-lock across those RPCs, which would block
`register()` for the duration of every hook call.

Fixed by tracking plugin-authored tools in a `HashSet` and skipping the round-trip for them —
their schema is already authoritative, converted by the host from omo's own definition. This is
safe because omo's `tool.definition` handler only rewrites the description of `todowrite`,
which is a **native** tool and still receives the hook.

Measured: prompt persistence **200 s → 15 s**; `tool.definition` timeouts **4–10 → 0**.

### 25. Two "blockers" I reported were wrong — corrected

Recording these because the wrong conclusions were nearly acted on.

- **"`session_prompt` mutates a clone and never writes it back."** False. `routes.rs:1739`
  already calls `sessions.update(session)` before persisting. I missed it by grepping for
  `update_session` instead of `sessions.update(`. Acting on the false lead produced a
  redundant `SessionManager::replace` that duplicated the existing `update`; that duplication
  has been removed.
- **"A plugin-load race strips omo's tools."** False. The captured provider payload shows
  `incoming=44 resolved=44` with `session_list` present. The model claiming *"I don't have
  access to a session_list tool"* was the free model misreporting its own toolset.

**Never trust a model's description of its own tools — log the provider request payload.**

### 26. Verified state of the omo tool chain

Proven at every layer, with the evidence for each:

- **Capture** — unit/integration test; definition and description survive `initialize`.
- **Schema conversion** — live; 26 tools registered, real JSON Schema produced.
- **Delivery** — live; provider payload `incoming=44 resolved=44`, includes `session_list`.
- **Invocation** — `crates/opencode-plugin/tests/plugin_tool_exec.rs` spawns a real bun host
  against a purpose-built plugin and asserts `tool.execute` returns `ECHO:hello`.
  Confirmed the test does **not** silently skip (no skip line, bun present, assertions panic
  on failure).

The only thing never observed is the free model *spontaneously choosing* an omo tool, which is
model behaviour rather than a plumbing gap.

### omo parity matrix

**Tools — captured, converted, registered, invocable (26):**
`background_cancel`, `background_output`, `call_omo_agent`, `glob`, `grep`,
`interactive_bash`, `look_at`, `session_info`, `session_list`, `session_read`,
`session_search`, `skill`, `skill_mcp`, `task`, and 11 `team_*`
(`team_create`, `team_delete`, `team_list`, `team_send_message`, `team_status`,
`team_task_create`, `team_task_get`, `team_task_list`, `team_task_update`,
`team_shutdown_request`, `team_approve_shutdown`, `team_reject_shutdown`).

Capture/registration is plumbing. Whether each tool then *behaves* correctly is separate and
mostly **unverified** — a tool can be delivered and invoked and still be useless if the
dependency underneath it is a stub.

**Not started / blocked:**
- `event` and `config` hooks are filtered out at load. `ulw-loop`'s state machine needs `event`
  as a genuine channel, and `config` is a **mutation** hook, so modelling it as a
  "config loaded" variant is semantically wrong.
- The SDK client exposed to plugins is effectively a **noop**, so any feature that calls back
  into the server (team-mode live delivery needs `promptAsync` to reach a peer) fails silently.
- **One plugin host subprocess per directory** — observed 3 live, and orphans reparented to
  `launchd` after their parent server was killed. This is the 106 MB × N memory problem.
- Team mode (11 tools) is the highest-value remaining feature: it is what the live subworker
  agents depend on to message each other. Its tools are now delivered, but end-to-end delivery
  to a peer transcript is unproven.
- `experimental.chat.messages.transform` throws `-32603`
  (`undefined is not an object (evaluating 'lastMessage.info.role')`) because the payload we
  build for that hook lacks the message shape omo expects. Non-fatal, but unfixed.

### Testing traps that cost real time

- The prompt endpoint is `POST /session/{id}/prompt` with `{"message": "..."}`.
  `POST /session/{id}/message` is `send_message` and **never builds a tool registry** —
  concluding from it that the injection point was dead sent me down a false path.
- The server process **dies when the shell call that started it returns**. Start it, probe it
  and grep its log inside a single call.
- `bun run` of the server writes the host to `~/Library/Caches/opencode/plugin-host.ts`; verify
  that file is current before trusting any runtime result.
- Importing omo's bundle directly in bun hangs for minutes. Only the spawned host loads it fast.
- Direct host probes must read the fd unbuffered — mixing Python buffered reads with `select()`
  produced a bogus "120 s timeout" while the server loaded the same plugin in ~190 ms.
- `PROMPT=200` only means the request was accepted. `session_prompt` returns
  `{"status":"started"}` before the work runs; confirm via persisted messages.

### 27. Plugin hosts leaked as orphans after the server died

Killing the server left `bun run plugin-host.ts` processes running, reparented to `launchd`
(`ppid=1`), holding **227 MiB** across three processes with ages of 26/22/1 minutes.
`pkill` on the server cannot reap them, because they are no longer its children.

Cause: on stdin EOF the host's read loop `break`s and `main()` merely *resolves*. omo installs
timers and a background manager that keep bun's event loop alive, so the process never exits.
Worse, a host whose parent died *while it was inside* `initialize` or a hook never regains
control of the loop and so never observes EOF at all.

Fixed in `crates/opencode-plugin/host/plugin-host.ts` with two parts: an explicit
`process.exit(0)` once the read loop ends, and `process.stdin.on("end"/"close", …)` handlers
that force exit independently of what the loop is doing. The first part alone reduced orphans
from 3 to 1; the second took it to 0.

Measured: 3 hosts alive while the server runs → **0 orphans** after killing it. Regression gate
green after the change.

This bounds the leak but does **not** fix the one-host-per-directory scaling — that still needs
a single shared host or native in-process plugins.

### 28. `experimental.chat.messages.transform` threw on every prompt

Every prompt logged:

```
WARN opencode_plugin: hook `experimental.chat.messages.transform` failed:
plugin RPC error (-32603): undefined is not an object (evaluating 'lastMessage.info.role')
```

Cause: a **shape mismatch**. omo reads `output.messages.at(-1).info.role` (plus
`info.sessionID`, `info.model`), i.e. the opencode wire shape
`{ info: { role }, parts }` with a lowercase role. `SessionMessage` serialises **flat** —
`role` at the top level, PascalCase — so `info` was `undefined`.

Because the hook is wrapped in a `catch`, this was non-fatal: prompts worked, but omo's
message-transform behaviour (context injectors, reminder and prefill logic) was **silently
inactive** — the worst failure mode, since nothing looked broken.

Fixed in `crates/opencode-session/src/prompt.rs` with a symmetric pair:
`messages_to_wire` projects messages into the shape plugins expect, and `messages_from_wire`
restores them. The inverse deliberately returns `None` if any entry is unrecognisable, so a
malformed payload leaves the original messages untouched rather than replacing them with a
partial list — a bug in the conversion cannot corrupt a prompt.

Verified live: `-32603` **1 → 0**, hook errors **0**, and the assistant still replies correctly
(`HOOKFIX_OK`, 1082 bytes persisted). Regression gate green. The only remaining mention of the
hook name is the benign plugin-load line.

### 29. Investigated and dismissed: "one plugin host per directory"

This was carried as an open risk ("106 MB × N"). It is **not** real.

Measured: 3 hosts after boot, 3 after sessions spanning 4 directories, 3 after 34 sessions
across those 4 directories. The count is **constant**.

The three hosts are three **plugins**, not three directories — `load_all` spawns one
subprocess per spec, and the spec list is fixed: two builtin auth shims (`codex-auth.ts`,
51 lines, provider `openai`; `copilot-auth.ts`, 67 lines, providers `github-copilot` /
`github-copilot-enterprise`) plus omo from `config.plugin`. Neither builtin contributes any
tool.

Lazy-loading the builtins was considered and **rejected**: `server.rs` iterates the auth
bridges at startup and calls `bridge.load()` per provider to pick up already-stored
credentials, so deferring the spawn would stop the server seeing stored Codex/Copilot
credentials at boot.

No action taken. The ceiling is bounded by the configured plugin count rather than by traffic,
and the 100-session loadtest measured ~442 MiB total (~7% of the 6 GB budget). Trading a
working auth path for ~150 MiB against a budget that is 93% free is a bad trade.

### 30. Corrected: the plugin SDK client is real, not a noop

This was carried as a blocker ("SDK client is a noop ⇒ features silently dead"). It is
**false**.

`createPluginClient` (`crates/opencode-plugin/host/plugin-host.ts:305`) resolves the real SDK in
three escalating steps — direct `import("@opencode-ai/sdk")`, then walking up from the plugin
directory, then `createRequire` — and only falls back to `createNoopClientProxy(["client"])`
if all three fail. The SDK is present at `~/.opencode/node_modules/@opencode-ai/sdk`, and omo
lives under `~/.opencode/plugins/`, so the upward walk succeeds.

Verified by temporarily instrumenting the noop branch: all three plugins load
(`builtin-codex-auth`, `builtin-copilot-auth`, `oh-my-opencode`) and the fallback is taken
**zero** times. The instrumentation was then removed and the gate re-verified.

The server also serves the endpoints that client needs (`/{id}/prompt`, `/{id}/prompt_async`,
`/{id}/message`). So team-mode live delivery is not blocked by a stub client — what remains
unproven is a real team run delivering to a peer transcript.

### 31. `config` mutation hook was never delivered

omo exposes a `config` hook whose handler mutates the config object in place. Three separate
gaps meant it never ran:

- the host filtered `config` out of the registered hook names;
- `hook_name_to_event` had no mapping for it, even though `HookEvent::ConfigLoaded` already
  existed and was never fired;
- the loader had no `ConfigLoaded` branch, so the hook would have been invoked with an empty
  payload.

Fixed with four small changes: stop filtering `config` in the host, map it in
`hook_name_to_event`, add a `ConfigLoaded` branch that copies the config's **own keys** into
`output` (the host passes `output` as the plugin's first argument, so nesting it under a
`config` property would have handed omo the wrong shape), and fire it from `server.rs` via
`apply_config_hook` between `load_config` and the provider bootstrap.

I had first recorded this as needing a reordering of server startup. That was wrong: the real
order is already plugins (`:150`) → config (`:155`) → registry (`:172`), so no reordering was
needed.

`apply_config_hook` ignores a payload it cannot parse back into `Config`, so a plugin bug
cannot cost the server its configuration.

Verified live: `"config"` now appears in omo's registered hooks, zero "skipping unsupported"
entries, zero hook errors, `GET /config` → 200. Config integrity was checked explicitly rather
than assumed — `~/.config/opencode/opencode.json` carries `$schema, experimental, mcp, plugin,
provider, agent` and Rust's `Config` models every one of them, so the round-trip cannot drop
user settings.

Known limitation: the plugin only ever sees fields Rust's `Config` models.

---

## The `event` hook was entirely missing (wrongly believed to need a push channel)

### The wrong premise I started from

I initially reported that omo's `event` hook "genuinely needs a new PUSH CHANNEL, not a
mapping entry". That was wrong, and it was the fourth wrong premise of this project. I had
inferred it from the fact that `event` is a *subscription* and never checked whether the
existing request/response dispatch already carries it.

### Why it was wrong

`handleHookInvoke` dispatches hooks **by name**:

```ts
const handler = pluginHooks[params.hook];
if (typeof handler !== "function") { sendError(id, -32601, `Hook not found: ${params.hook}`); return; }
```

and omo really does return an `event` hook (`dist/index.js:162882`,
`event: createEventHandler2({...})`). So an ordinary `hook.invoke` with `hook:"event"`
already reaches it. Rust initiates the call; no bidirectional streaming is involved.

### What was actually missing

- `host/plugin-host.ts` filtered `event` out of the registered hook names:
  `if (key === "auth" || key === "event") continue;`
- `hook_name_to_event` had no `"event"` arm.
- Nothing ever fired a plugin event.

### The fix (four small changes)

- `crates/opencode-plugin/src/lib.rs`: added `HookEvent::PluginEvent`. It is deliberately a
  **unit** variant: `HookEvent` derives `Eq`/`Hash`, which `serde_json::Value` cannot
  satisfy, so the event object travels in `HookContext::data` under the `event` key.
- `crates/opencode-plugin/host/plugin-host.ts`: stopped filtering `event` out of hookNames.
- `crates/opencode-plugin/src/subprocess/mod.rs`: `"event" => Some(HookEvent::PluginEvent)`.
- `crates/opencode-plugin/src/subprocess/loader.rs`: a branch that copies the raw event into
  `input["event"]` (not `output`, and not flattened) because that is exactly where the
  handler reads `type` and `properties` from.
- `crates/opencode-server/src/routes.rs`: a `notify_plugin_event` helper, fired from the
  prompt-completion path immediately after the session is marked idle.

### Proof

A temporary host-side diagnostic (since removed) confirmed the round trip against a real
prompt:

```
[evt-diag] event handler OK
input={"event":{"properties":{"sessionID":"ses_863d158366e44db28bee50fa66104c89"},"type":"session.idle"}}
```

Rust fired it, the loader nested it under `input.event`, the host dispatched by name, and
omo's handler ran and returned without throwing.

### Behaviour change worth knowing about

omo's `session.idle` handler does more than observe. It calls
`modelFallbackHandler.clearRetryDedupeAfterIdle(sessionID)`, runs `dispatchToHooks(input)`
(which is how an active `ulw-loop` continues), and mirrors state via `tuiStateMirror`. So
firing it per prompt means the model-fallback **retry dedupe is cleared on idle** — a
rate-limited model will be retried on the next prompt rather than being suppressed. That is
the correct parity behaviour (a real TS session does go idle after a turn), but it is a real
change in retry/continuation dynamics, not an inert notification. `shouldDispatchIdleEvent`
dedupes per session inside a window, so repeated idles are safe.

---

## `POST /session/:id/prompt_async` never actually ran the prompt

### Symptom

Team mailbox injection never happened: a member's prompt never contained
`<peer_message>`, and team members sat silent forever.

### Root cause

`prompt_async` recorded the message and answered `"queued"` — and then did nothing:

```rust
session.add_user_message(&message_text);
let assistant = session.add_assistant_message();
Ok(Json(json!({"status": "queued", "message_id": assistant_id, "model": req.model})))
```

There was no `tokio::spawn`, so no turn ever ran. Two consequences:

- `messages_to_wire` / `ChatMessagesTransform` never fired, so omo's
  `teamMailboxInjector` never ran, so nothing could ever be injected.
- Nothing was ever queued for processing, so the route lied to its caller. This is a
  plain parity bug independent of teams: **any** request to `/prompt_async` was silently
  dropped.

`session_prompt` (the synchronous `/prompt` route) *did* spawn a turn. `prompt_async`
never did — and omo uses `prompt_async` to spawn team members, so every member was
created, persisted with a correct `sessionId`, and then left silent.

### The misleading evidence

For a long time this looked like an omo-side lookup failure, and nine plausible causes
were eliminated one at a time — all of them genuinely correct: `base_dir` resolution,
`RuntimeStateSchema` validation, runtime status transitions, `member.sessionId`
persistence, the exact-match disk fallback in `findResolvedMemberSession`, the
`MESSAGES_TRANSFORM_HOOKS` registration, and the `team_mode.enabled` /
`safeHookEnabled` gates. Every one of those checks passed. They were simply never
reached.

One earlier observation actively misled: `<Category_Context>` appears in the member
prompt, which looked like proof that omo's transform hook had run. It is not — omo's
*spawn* code puts that text in the request body itself. It was never evidence of the
server's hook firing.

### Fix

- Extracted the ~110-line turn body out of `session_prompt` into a shared
  `run_prompt_turn(...)` (routes.rs), so both routes drive the identical path.
- `prompt_async` now resolves the provider/model and spawns `run_prompt_turn`.
- It no longer pre-adds a user or assistant message: `prompt_with_update_hook` calls
  `create_user_message` itself, so keeping the old lines would have duplicated the turn.
- `message_id` in the response is now a synthetic id. The real assistant message is
  created inside the turn, and `PromptInput.message_id` is a dead field that is never
  read, so it cannot be threaded through. Callers poll the session message list rather
  than resolving that id.
- Added `prompt_async_model_ref` because `/prompt_async` accepts `model` both as
  `"provider/model"` and as the SDK object `{providerID, modelID}`.

### Verification

- Before: member sessions had no assistant messages at all.
- After: `['user','assistant']` and `['user','assistant','assistant','assistant']`.
- The `chat.messages.transform` hook now fires for members (previously it never ran),
  and the `sessionID` omo observes from `info.sessionID` matches the on-disk
  `member.sessionId` exactly.

### Remaining

A final end-to-end observation of literal `<peer_message>` text was not completed. The
probe cannot show it: it captures omo→server traffic, while injection happens
server→LLM. Each member also gets only one turn, at spawn, which happens *before* a
message is in the mailbox — so there is nothing to inject on that turn, and `peer=false`
there is correct rather than a defect. Driving a second member turn with a message
already in the mailbox is the outstanding check.

---

## Injection pipeline is LIVE — proven after the `prompt_async` fix

After fixing `prompt_async`, member prompts finally run, and the omo team hooks in the
**server's** plugin host are observably working. Two independent proofs, read off a real
member session:

1. **Team-mode status injection works.** Beta's first user message carries omo's
   `# Team Communication / You are running as a team member / IMPORTANT: Just writing a
   response in text is NOT visible to others...` block. That is omo's
   `teamModeStatusInjector` output, delivered through the same
   `MESSAGES_TRANSFORM_HOOKS` path as the mailbox injector.

2. **The mailbox injector runs and reads the real mailbox.** After a second prompt,
   beta's messages contained:

   ```
   You have 1 new team messages. They will be injected on your next turn.
   ```

   That is `teamMailboxInjector` counting an actual unread message in
   `~/.omo/runtime/<teamRunId>/inboxes/beta/` — proof it reached
   `pollAndBuildInjection`, found the member via the disk fallback, and polled the inbox.

So the whole chain is now confirmed live end to end on the server side:

```
prompt_async -> run_prompt_turn -> messages_to_wire -> ChatMessagesTransform
  -> teamModeStatusInjector      (proven: status block in member message)
  -> teamMailboxInjector         (proven: "1 new team messages")
  -> pollAndBuildInjection -> findResolvedMemberSession -> inbox poll
```

Every previously-eliminated omo-side cause is confirmed irrelevant: the lookup is
reached, and it succeeds.

### Still outstanding

The literal `<peer_message>` body has not yet been observed. omo's injector is
**two-phase**: one turn announces the count, and the content is delivered on a
*subsequent* turn, gated by the member's `lastInjectedTurnMarker`. A third turn was
driven and still produced no `<peer_message>` text, while the message count kept growing
(4 -> 6), so turns are completing but the announce->inject handoff has not been
observed firing. That gate is the single remaining unknown, and it lives entirely inside
`pollAndBuildInjection`'s turn-marker bookkeeping.

Note this is a different failure from the original: previously *nothing* fired at all;
now the injector's own announce path runs and demonstrably reads the mailbox.

---

## Team mailbox injection: three defects, all on our side

`prompt_async` running the turn was necessary but not sufficient. Injection still produced
no `<peer_message>` until three more bugs were fixed — every one of them ours, and two of
them silent (no error, no log, wrong output).

### 1. `loop_inner` decided "already answered" by coin flip

```rust
if has_finish && last_user.id < assistant.id { break; }
```

Message ids are random UUIDs (`msg_2c633a8d-6eee-…`), so this lexicographic comparison
decides by chance, not chronology. A legitimate second turn was dropped roughly half the
time, before the transform hook ever ran. Fixed by comparing positions in the
chronological vector (`rposition`) instead of ids.

Symptom before: a second `prompt_async` on a session that already had a reply left the
session at `['user','assistant','user','user']` with no assistant reply, at any wait
length. After: `['user','assistant','user','assistant']`.

### 2. One id-less injected message discarded the whole batch

`messages_from_wire` built each message with `id: info.get("id")?.as_str()?` and collected
into `Option<Vec<_>>`. omo injects its turn as a bare `{info:{role,sessionID}, parts:[…]}`
— no `id` — so the `?` yielded `None` for that message and the **entire** list was dropped,
leaving the current messages untouched. Now a stable id is derived from
`(session_id, index)` when absent, mirroring how a missing `created` was already tolerated.

### 3. The injected part was in a shape our parser could not read

This was the real blocker. `messages_to_wire` emits parts via
`serde_json::to_value(&MessagePart)`, which **nests** the variant under `part_type`.
Plugins emit the original flat TS shape (`{"type":"text","text":…}`). `MessagePart`
requires `id` and `created_at`, so `from_value::<MessagePart>` failed on the flat part and
`.ok()?` again discarded the whole list.

`PartType` is `#[serde(tag = "type")]`, so a flat object deserializes into it directly.
`parts_from_wire` now tries the nested shape first, then falls back to the flat shape,
synthesising `id` and `created_at` (`time.start` is honoured) instead of bailing.

### Proof

A temporary diagnostic on the transform output — since removed, and the vendored omo
bundle it was paired with restored byte-identical — showed the transition directly:

```
before:  1 INJDIAG msgs=3 peer=false      # 3 messages, nothing injected
after:   1 INJDIAG msgs=2 peer=true       # <peer_message> present in the turn
```

Independently, instrumenting omo's own `pollAndBuildInjection` showed it returning
`injected=true` with a real message id — omo had been doing its job the whole time; we were
throwing the result away.

### Lesson worth keeping

All three bugs shared one signature: a plugin's correct output was discarded by a parser
that treated optional-by-design fields as mandatory, and did so through `Option`/`?`, so
there was no error to trace. The only reason any of this was findable was comparing counts
(3 messages in, 3 out) instead of trusting the absence of an error.
