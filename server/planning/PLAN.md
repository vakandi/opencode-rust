Task completed in 13m 25s.

Agent: plan
Model: opencode/space-bunny-free

---

Confirmed — `get_session_status(session_id)` takes no `directory` at lines 543/557, so it falls back to `process.cwd()`. The completion-detection bug is real.

---

# RUST PORT PLAN — Recover → Pin → Measure → Decide

**Plan mode: read-only. Nothing was modified.**

I re-verified the load-bearing facts myself rather than trusting the bundle: `docker ps -a` → no `elia-subworker-srv` · disk **95% / 23 GiB** · `docker-compose.yml:9-10` → `mem_limit: 8g`, `memswap_limit: 8g` (**swap off**) · `Dockerfile:48-56` resolves `releases/latest` at build time under a false "pinned" comment · `opencode.fixed` 1,349,709,824 B @ Sep 23 14:34 vs `opencode.db` 1,405,136,896 B @ Oct 5 00:27 · `opencode_client.py:543,557` calls status with **no `directory`** · per-crate tests in `/tmp/OpenCodeRust`: **mcp 2, plugin 3, storage 0, types 0, cli 0** · `storage/schema.rs` = **6 tables, no `event`** · `server.rs:197 sync_sessions_to_storage` → `delete_for_session` at `:213`/`:231` = the full-resync DELETE+re-INSERT data-loss vector, and it is called on **19 routes** including the SSE stream path (`routes.rs:1598`) · `SessionManager.events: Vec<SessionEvent>` is **never drained and never truncated** (only `drain_events` at `session.rs:1113` consumes it, and nothing calls it) · `cargo check -p opencode-tool` **passes**, so ts-anatomy's suspected lsp-feature build break does **not** exist — `lsp_tool.rs` is correctly `#[cfg(feature = "lsp")]`-gated.

---

# ⚠️ AMENDMENT A1 — WAEL'S RULING: omo BECOMES A BUILT-IN PLUGIN

**This amendment supersedes every claim in the plan below that a JavaScript runtime is required to deliver omo's features.** Those claims were true of the original framing. They are false under this ruling, and §5.4, §7.1, §7.2 and §7.3 have been rewritten accordingly.

## A1.1 The ruling

Wael's words, 2026-10-05 09:44:

> *"opencode rust can't load omo — who cares? you can just recode it inside opencode as a builtin plugin like I said, ADD IT INTO THE PLAN."*

**Do not make the Rust server load oh-my-openagent as an external TypeScript plugin over a bun/node subprocess. Reimplement omo's features as opencode's own internal (built-in) plugins, compiled into the binary.**

This is not a preference. It removes from the plan, permanently:

| Removed | Why it is gone |
|---|---|
| the JSON-RPC subprocess plugin host as omo's delivery path | no plugin crosses a process boundary |
| the `-32600` export-shape bug (**blocker 1**) | there is no export shape to reject — nothing is serialised |
| the discarded tool registry / 26 unreachable tools (**blocker 2**) | tools are registered in-process into a Rust registry, not returned over RPC |
| the noop SDK client that silently kills 3 of 4 features (**blocker 4**) | a native plugin receives a real client by construction |
| the missing `tool.execute` RPC (**blocker 5**) | tool execution is a direct function call, not an RPC |
| the stdin lost-wakeup race (**blocker 7**) | no stdin |
| the `event`/`config` hook filter (**blocker 3**) | nothing is filtered; hooks are `HookEvent` enum variants dispatched natively |
| the node-fallback version gate (**blocker 6**) | no node |
| **11 × 106 MB = 1,166 MB** of plugin hosts | zero extra processes. One in-process registry |
| "**Pure Rust is architecturally impossible**" (§5.4) | it is now possible for this deployment. See A1.5 for the exact boundary |

## A1.2 The mechanism exists and is verified — this is not a proposal

Read in the source, not inferred:

```ts
// packages/opencode/src/plugin/index.ts:67
function internalPlugins(flags: RuntimeFlags.Info): PluginInstance[] {
  return [
    (input) => CodexAuthPlugin(input, { experimentalWebSockets: ... }),
    CopilotAuthPlugin, ModalPlugin, GitlabAuthPlugin, PoeAuthPlugin,
    CloudflareWorkersAuthPlugin, CloudflareAIGatewayAuthPlugin, AzureAuthPlugin,
    DigitalOceanAuthPlugin, SnowflakeCortexAuthPlugin, XaiAuthPlugin, CerebrasPlugin,
  ]
}

// :170 — directly invoked, no dynamic import(), no subprocess, no npm
for (const plugin of flags.disableDefaultPlugins ? [] : internalPlugins(flags)) {
  const init = yield* Effect.tryPromise({ try: () => plugin(input), ... })
  if (init._tag === "Some") hooks.push(init.value)
}
```

A `PluginInstance` is `(input: PluginInput) => Promise<Hooks>`. That is the entire contract. The `input` built at `:147-168` already carries everything omo's four features need:

| `PluginInput` field | line | What omo uses it for |
|---|---|---|
| `client` | :148 | `session.promptAsync`, `session.create`, `session.abort`, `messages` polling — team mode, `call_omo_agent`, ulw-loop |
| `directory` | :152 | multi-directory scoping — the routing team mode's live delivery requires |
| `project` | :150 | project identity |
| `worktree` | :151 | worktree identity |
| `serverUrl` | :157 | proxy target |
| `experimental_workspace.register` | :153 | workspace adapter registration |
| `$` | :159 | shell — already gated `typeof Bun === "undefined" ? undefined : Bun.$` |

The `client` is not a stub. `:145-147` builds it over `Server.Default().app.fetch` when there is no external server URL — **in-process fetch against the app itself**. Even the round-trip that blockers 4 and 5 killed is already handled natively.

**`disableDefaultPlugins` is the kill-switch** — `packages/opencode/src/effect/runtime-flags.ts:19`, env `OPENCODE_DISABLE_DEFAULT_PLUGINS`. One flag returns the process to a plugin-free state, which is exactly what the negative control in S4 needs.

## A1.3 The Rust side does not have it — that is the build

`/tmp/OpenCodeRust` today:

| Primitive | Present? | Where |
|---|---|---|
| `HookEvent` enum (23 variants) | ✅ | `crates/opencode-plugin/src/lib.rs:43` |
| `PluginSystem::register` / `trigger` | ✅ | `lib.rs:172`, `:186` |
| `Hook` with priority + enable | ✅ | `lib.rs:143` |
| `opencode_tool::ToolContext` + `ToolRegistry` | ✅ | `crates/opencode-session/src/prompt.rs:1478-1479` |
| **`internal_plugins()` registry** | ❌ **absent** | — |
| **`PluginInput` equivalent (client/project/worktree)** | ❌ **absent** — `PluginContext` is `{worktree, directory, serverUrl}` only | `crates/opencode-plugin/src/subprocess/client.rs:111-116` |
| **`tool` hook → tool registry contribution** | ❌ **absent** — `ToolDefinition` is an LLM type, not a plugin contribution | — |
| `dispose` hook | ❌ | — |
| `experimental.compaction.autocontinue` | ❌ | — |

And the anti-pattern is present and should be deleted:

```rust
// crates/opencode-plugin/src/subprocess/loader.rs:156-176
pub async fn load_builtins(&self, context: &PluginContext) -> ... {
    std::fs::write(&code_dir.join("builtin-codex-auth.ts"), BUILTIN_CODEX_AUTH)?;  // writes .ts to disk…
    let specs = vec![format!("file://{}", codex_path.display())];                   // …then subprocesses it
    self.load_all(&specs, context).await
}
```

Two bundled **auth** plugins are shipped as TypeScript strings written to a cache directory and executed in a subprocess. Wael's ruling deletes this path entirely: those become two lines in a native registry.

## A1.4 What it costs — stated, not hidden

| Cost | Detail |
|---|---|
| **It is a re-implementation, not a port** | omo is ~27k lines of TS. Re-expressing 4 features natively is real work, not a config flag |
| **omo loses independent lifecycle** | an internal plugin is compiled in. It cannot be updated without rebuilding opencode; the current `bun upgrade` path no longer applies to it |
| **`disableDefaultPlugins` is all-or-nothing** | there is no per-plugin disable. If a native omo plugin must be switched off, it needs its own internal flag — add one (`RuntimeFlags`, not an env guess) |
| **Coupling** | omo's 4 features now live inside opencode's tree. That is the trade Wael chose: 1 process instead of 11 |
| **The TUI stays TypeScript** | `@opentui/*` is a bun/Solid concern and is out of scope. §7.4's carve-out is unaffected |
| **Not covered: everything omo does that is not one of the 4 features** | see A1.6 |

## A1.5 The honest boundary of the win

Native plugins remove the JS-runtime requirement **for the plugins opencode ships**. They do not remove it from the product:

| Still needs a JS runtime | Still true? |
|---|---|
| `.ts` dynamic import for **user-authored** tools (`tool/registry.ts:192`) | **YES — unchanged.** A third-party plugin the user writes is not an internal plugin |
| zod introspection for **user** tool schemas (`registry.ts:375`) | **YES — unchanged** |
| `BunShell` / `Buffer` for **user** plugins (`plugin/src/shell.ts:74,106`) | **YES — unchanged** |
| `@opentui/*` type resolution | **YES — unchanged** |
| in-process HTTP CONNECT proxy on 18888 | **NO** — becomes omo-native. `reqwest` honours `HTTP_PROXY`/`HTTPS_PROXY` natively. ~600 LOC of Rust, already costed in §7.2 as LOW risk |

So: **the external plugin host is not dead, it is just no longer omo's problem.** That is the correct and honest form of this win. A claim that the port is now "pure Rust, no JS anywhere" would be false — `registry.ts:192` and `registry.ts:375` are in opencode's own tool registry, not in omo.

## A1.6 The scope that is now fixed: exactly four features

The ruling covers the four features that were blocking, and nothing else:

1. **unlimited-token AI** (proxy rotation) — native `reqwest`, ~600 LOC
2. **ulw-loop** — needs `event` as a genuine notification channel + `promptAsync`
3. **team mode** — 11 tools + `tool.execute.before`; live delivery via `promptAsync`
4. **`call_omo_agent`** — `session.create` → `promptAsync` → poll → `abort`

**Out of scope, explicitly:** omo's slash commands, its non-4 headline tooling, the proxy switcher TUI (§7.4), and any feature outside this list. The subworker fleet's 25 agents depend on **team mode + `call_omo_agent`**; those two are the load-bearing ones. If only those two are reimplemented, the fleet runs.

---

## 0. HONEST POSITION

**Amendment A1 (above) changes one thing and leaves everything else standing.** Wael ruled that omo becomes a native built-in plugin rather than an external subprocess plugin. That deletes the plugin-host blocker list and the 1,166 MB line. **It does not change the decision procedure**: the port remains gated on W2's measurement and W4's verdict, and under branch (c) — the expected branch — **W5C does not apply at all**, because omo already runs natively in the TypeScript server. W5C is the price of branch (b), and it makes (b) shippable.

Wael asked for a Rust port. The team's unanimous final position is **(d): recover → pin → capture one live baseline → THEN decide.** Nobody reached greenfield; three of five prefer *adopt-and-port* over *greenfield* if a port happens.

This plan **does not pretend a measured baseline exists**, because none does. perf-architect retracted **8 of its own claims** in Round 3, including its entire RAM thesis. Three analysts measured the same wrong database (perf's 17.68 GiB host-TUI file vs the real 1.31 GiB container file — **off by 13.5×**). plugin-host's Round 4 R7 sets the bar this plan applies:

> *"a port is justified only if it is scoped to something the TS server provably cannot do. Today that set is empty."*

This plan **also does not drop Wael's request**. The port survives as a fully-costed branch (Wave 5B + **Wave 5C**, per A1), reachable the moment the gate selects it. What is refused is *authorising* it before evidence exists.

---

## 1. SCENARIO CONTRACT

### S1 — Happy path: one subworker run completes and reports| | |
|---|---|
| **Binary pass** | `docker logs <srv>` contains `response_agent=<requested-name>` — `gilfoyle`, **not** `unknown`, **not** `default` — **AND** the message envelope yields assistant text of length > 0 |
| **Real surface** | `runner.py:455` logs `response_agent=sess.get("agent","unknown")`. Plus `curl -H 'x-opencode-directory: <dir>' "localhost:5655/session/<id>/message?limit=50"` returning `{"info":{"role":"assistant","parts":[{"type":"text","text":"<non-empty>"}]}}` — the **nested** `info` envelope |
| **Test** | `pytest tests/integration/test_subworker_e2e.py::test_single_run -k gilfoyle`, driving the **unmodified** `opencode_client.py` |
| **Falsifies** | `/tmp/OpenCodeRust` wire breaks (b) and (d): agent hardcoded `"default"`, flat `MessageInfo` |

### S2 — Edge / boundary

| | |
|---|---|
| **Binary pass** | All four in one test: **(a)** `GET /session/` **with trailing slash** (what the client actually sends) does not 404; **(b)** `GET /session/status` returns `{}` for an idle session — *omission-not-idle* is load-bearing (`status.ts:39-48`); **(c)** `POST /session/:id/message` with a **mismatched** `x-opencode-directory` routes to the *session's own* directory, not the header's, and not a 404 (`workspace-routing.ts:182`); **(d)** the system holds at both **N=1** and **N=N_max** |
| **Real surface** | `curl -o /dev/null -w '%{http_code}'` per case; `docker stats --no-stream` for (d) |
| **Test** | `pytest tests/integration/test_wire_contract.py -k boundary` |

### S3 — Adjacent-surface regression: the 25 live agents' HTTP contract

| | |
|---|---|
| **Binary pass** | All **11** endpoints return their documented shape **with `x-opencode-directory` set**: `GET /global/health`→`{"healthy":true,…}` · `GET /session/`→bare array · `POST /session`→session with `agent` echoed · `GET /session/:id` · `GET /session/status` · `POST /session/:id/message`→one-chunk stream · `GET /session/:id/message`→nested `info` + `Link`/`X-Next-Cursor` **headers** · `DELETE /session/:id`→`true` · `POST /session/:id/abort`→`true` · `POST /instance/dispose`→`true` · `GET /provider`. **Plus** precedence proven individually: `?directory=` > `x-opencode-directory` > `process.cwd()` |
| **Real surface** | `curl` status + body per endpoint. Requests are **extracted from `opencode_client.py` itself** (`_dir_headers` at `:99-101`, `_dir_params` at `:105-107`, `abort` sends both at `:299-300`), so the contract cannot drift from the consumer |
| **Test** | `pytest tests/integration/test_client_contract.py` — golden replay |
| **Why first-class** | perf-architect demanded "rollback for **25 live agents** — dual-run + shadow traffic before cutover". This is the *pre-cutover* half |

### S4 — Plugin parity: **runtime round-trip, never registration** · *(unchanged by A1, and now the sole gate for W5C)*

| | |
|---|---|
| **Binary pass** | (a) `team_send_message` → peer agent's transcript contains the message within 60 s; (b) `call_omo_agent` returns non-empty final text; (c) `/ulw-loop` produces a **second** assistant turn with no human input; (d) `background_output` returns real output for a known task id. **AND** the negative control: with the noop SDK client injected, all four **must fail** |
| **Real surface** | `session.promptAsync` round-trip; `curl GET /session/<peer-id>/message` then grep the transcript. **Not** hook counts. **Not** registration counts |
| **Test** | `pytest tests/integration/test_plugin_parity.py` — under (c) it runs against the TS server as the **reference oracle** |
| **Why the negative control** | plugin-host §2: *"three of four features are gated on the SDK client, not on hooks. Any host that stubs `client` satisfies the letter of the API and destroys the substance."* A patched host loads omo with **12 hooks + 26 tools, no error** — so any registration smoke test **passes while team mode, call_omo and ulw-loop are all dead** |
| **Why A1 does not weaken this** | A1 removes the *host* that could accidentally stub `client`. It does not remove the ability to write a plugin that registers 26 tools and never calls `promptAsync`. **The control is now the only thing standing between a green build and a dead fleet**, and W5C.0 must therefore ship the per-plugin enable flag it needs. *(A1.4, §8.16)* |

### S5 — Security: the confirmed prompt-text permission bypass

| | |
|---|---|
| **Binary pass** | **Both halves**: (a) a user message containing `@explore` **still** triggers `ctx.ask()` for `task`; (b) the model-initiated `task` tool at `prompt.ts:331` **still** skips it |
| **Real surface** | Permission-prompt emission asserted in-process + a live `curl` with an `@agent` token showing the prompt IS raised |
| **Test** | Two unit tests, `packages/opencode/test/session/task-permission.test.ts` |
| **Why both halves** | plugin-host §4.3: half (b) is what a naive fix breaks — `prompt.ts:331` hardcodes `true` **legitimately** (double-prompt-avoidance when the model itself called `task`) |
| **Exploit chain (verified twice)** | `prompt.ts:176-178` synthesises an `agent` part from any `@name` token → `prompt.ts:1223` `bypassAgentCheck = true` → `tool/task.ts:119` `ctx.ask()` skipped. And `tools.ts:64` injects `extra` for **all** resolved tools, not just `task` |

### S6 — Integrity: the database

| | |
|---|---|
| **Binary pass** | `pragma integrity_check` → `ok` **AND** `select count(*)` returns an integer on **all three** of `message`, `part`, `event` **AND** `count(*) from session` ≥ the pre-promotion count |
| **Real surface** | `sqlite3` CLI on the live file |
| **Test** | `scripts/db_health.sh` (new; non-zero on any failure) |

---

## 2. TASK DEPENDENCY GRAPH

```
W0 (no code)  W0.1 snapshot ─┬─> W0.3 rollback-vs-recovery ─┐
                W0.2 free2.15G ┴─> W0.4 .recover ──────────┴─> [GATE W0]
W1 (code)      W1.1 bypassAgentCheck ─┐ W1.2 pin Dockerfile  ─┴─> [GATE W1]
W2 (measure)   W2.1 boot ─> W2.2 harness ─┬─> W2.3 N=1 ─> W2.4 N=5 ─> W2.5 sweep ─> W2.6 report ─> [GATE W2]
W3 (TUI)       W3.1 typecheck baseline ─> W3.2 Group A inline ─> [W3.3 Group B delete — GATED ON W4]
W4            RE-DECIDE GATE ─────────────┬─> W5A (c) fix in TypeScript  ─> W6 cutover
                                         │        (omo keeps running as-is, 1.06 GB)
                                         └─> W5B.0 admission ─> W5B.1..5 ─┐
                                                                      └─> W5C.0..7 omo-native ─> W6 cutover
W6            dual-run + shadow traffic + staged cutover for 25 live agents
```

### Hard ordering constraints — each is a refusal, not a preference

1. **No porting code before W4 returns.** Not one line.
2. **No porting work, CI included, before W2 returns.** *"A CI pass on a ref nobody ships is wasted motion, and I was about to schedule exactly that."*
3. **W0.3 must precede W2.1.** plugin-host **R9**, found in Round 4 — the sharpest ordering hazard in the exercise: *"Decision 2 would write to the corrupt `opencode.db`, at which point the two files diverge and the clean snapshot becomes lossy… **Measuring the running system is what makes the recovery lossy, so recovery must precede measurement.**"*
4. **W1.1 must precede W2.1.** Running the measurement = running 25 root agents with the bypass live. plugin-host: *"my 'measure before porting' gate was **unsafe as written**."*
5. **W0.2 must precede any DB write.** Volume at 95%; `SQLITE_FULL|ENOSPC` → **zero hits in production code**; `journal_mode=WAL` + `synchronous=NORMAL` + `wal_autocheckpoint` absent (default ~4 MB). ENOSPC during a WAL write produces exactly the `btreeInitPage() error code 11` signature we measured.
6. **W3.3 must not execute under branch (c).** Deleting `src/cli/**` removes Wael's operator interface.
7. **W5C must complete before W6 cutover.** Added by A1. Branch (b) without W5C ships a server with **zero** team-mode capability and 25 live agents that cannot message each other. **A green W5B is not a shippable product.**
8. **W5C.1 must precede W5C.2 and W5C.3.** A plugin cannot *add* tools until the contribution channel exists. Team mode's 11 tools are the whole feature.

---

## 3. PARALLEL EXECUTION GRAPH

```
Round 1   W0.1 ∥ W0.2                                   → GATE W0
Round 2   W0.3 → then W0.4 (W0.3 informs W0.4)
Round 3   W1.1 (TS repo)  ∥  W1.2 (Dockerfile) → GATE W1
Round 4   W2.1 → W2.2                                    → serial (profiler attaches to a booted process)
Round 5   W2.3 ∥ W2.4 ∥ W3.1                            → independent measurement points + independent repo
Round 6   W2.5 → W2.6                                    → sweep must be serial (contaminates RSS)
Round 7   W4.1 re-decide                                 → GATE W4
Round 8   W5A.1 ∥ W5A.2 ∥ W5A.3 ∥ W5A.4 ∥ W5A.6          → max 2 code-writing agents; distinct files          |  W5B.0 admission MUST precede W5B.1..W5B.5    → W5B strictly serial (one DTO file)
Round 8b  W5C.0 → W5C.1 (serial: both are the plugin foundation) → {W5C.2 ∥ W5C.3} → W5C.4 → W5C.5 → W5C.6 → W5C.7
         ↑ W5C runs on the SAME branch as its W5B, in parallel with W5A only if the TS repo is untouched by both.
           Under (b), W5C.2 ∥ W5C.3 is the only sanctioned 2-writer slot (distinct files: tools/team_* vs tools/call_omo.rs).
Round 9   W6 dual-run → shadow traffic → staged cutover   → BLOCKED until W5C returns (constraint 7)
```

**Concurrency budget:** max **2** concurrent code-writing agents. Inside W5A the TS repo and the subworkers repo are separate and parallel-safe; two agents inside the TS repo are not. W5B is serial per subsystem. **W5C.2 ∥ W5C.3 is the only sanctioned 2-writer slot inside the Rust tree** (A1: distinct files, and they are the two features the 25 live agents need).

---

## 4. WAVES

### WAVE 0 — STOP THE BLEEDING · no code · blocks everything · ~1 day

| ID | Task | Category | Skills | Verification |
|---|---|---|---|---|
| **W0.1** | Read-only snapshot + SHA-256 manifest of all `opencode-data/` artifacts | `quick` | `verification-before-completion` | manifest for `opencode.db`/`-wal`/`-shm`, `opencode.fixed`, `opencode.db.corrupt*`, `opencode.db.new`; copies stored **outside** `opencode-data/`; manifest committed |
| **W0.2** | Free 2.15 GiB: delete `opencode.db.corrupt` (954,097,664 B), its `-shm`/`-wal`, and `opencode.db.new` (0 B) | `quick` | `verification-before-completion` | `df -h` avail ≥25 GiB; **all files snapshotted in W0.1 first** |
| **W0.3** | **The rollback-vs-recovery decision** | `ultrabrain` | `shared/programming`, `verification-before-completion` | rule below |
| **W0.4** | `.recover` the corrupt file; settle **`event` absent-vs-corrupt** and size it | `deep` | `shared/programming`, `verification-before-completion` | `sqlite3 corrupt.db ".recover" recovered.db` exits 0; `count(*) from event` returns an integer; `event` byte size recorded |

**W0.3 — decision-complete rule.** `opencode.fixed` = **1,019 sessions / 20,667 messages / 95,739 parts / events=0**, `integrity_check: ok`. The corrupt file's **readable** surface is `session` = **1,019 rows, clean**; `message`/`part`/`event` all error. plugin-host R8 measured both files report **identical session count (1,019)** and **identical `max(time_created)` = 1789819239280** — "nothing was written after Sep 23 because the container has not run."

Because `session` is the **one readable table**, the promotion is *verifiable*, not hopeful:

1. Read-only-extract the full `session` table from **both** files.
2. On each: `SELECT count(*), max(time_created), max(time_updated), sum(length(id)) FROM session`.
3. **If the corrupt file's `session` table is a strict superset → DO NOT promote.** Recover instead.
4. Otherwise: corrupt `opencode.db` → `opencode.db.corrupt-20261005`, then `opencode.fixed` → `opencode.db`.

**Disclosure carried forward:** mtime is no longer evidence of write recency — perf-architect and the lead both perturbed it with read-only queries. Steps 1–3 are what settle the question.

**Most likely answer: promote.** R8's timestamp equality plus 12 days of container absence makes loss near-zero. Step 3 makes it *provable* rather than assumed.

**If step 3 trips:** **STOP.** Escalate to fresh-DB rebuild; the 20,667-message history in `.fixed` becomes the only surviving copy. Re-plan W2.

---

### WAVE 1 — MAKE IT SAFE TO RUN · code · blocks the baseline · ~half day

| ID | Task | Category | Skills | Verification |
|---|---|---|---|---|
| **W1.1** | Remove the `bypassAgentCheck` **derivation**; keep the legitimate writer | `deep` | `shared/programming`, `test-driven-development`, `security-research`, `verification-before-completion` | **both** S5 halves green; `bun run --cwd packages/opencode test` green; `bun turbo typecheck` green |
| **W1.2** | Pin the deployed artifact | `quick` | `verification-before-completion` | see below |

**W1.1 — exact change set (plugin-host §4.3 writer census; do not deviate):**

| Site | Action | Reason |
|---|---|---|
| `session/prompt.ts:1223` | **REMOVE** `const bypassAgentCheck = lastUserMsg?.parts.some((p) => p.type === "agent") ?? false` | the defect |
| `session/prompt.ts:1231` | **REMOVE** from `SessionTools.resolve({...})` | plumbing |
| `session/tools.ts:46,64` | **REMOVE** declaration + `extra` injection | plumbing; also closes the flag leaking to **all** resolved tools |
| `session/prompt.ts:331` | **KEEP** `extra: { bypassAgentCheck: true, promptOps }` | **legitimate** — model-initiated `task` |

**TDD order (mandatory):** write both tests → demonstrate **RED** → commit with RED evidence in the body → implement → demonstrate **GREEN**. Half (b) is what a naive fix breaks.

**W1.2:** `Dockerfile:48-56` resolves `releases/latest` at build time under `# ... (pinned, not main)`. **That comment is false — the least-pinned method available.** Three versions are in play: client docstring `1.18.18`, host binary `1.18.34`, container *unknown and drifting every rebuild*.

- `ARG OPENCODE_VERSION=<exact tag>`, no API call.
- Label the image with `opencode --version` + tarball `sha256`.
- Endpoint-fingerprint script (`GET /global/health` + `GET /doc`, diff the route set).
- **Acceptance:** two builds of the same tag → identical fingerprint; version readable from `docker inspect` without starting the container.

**Gate W1 → W2:** both S5 halves green; artifact identifiable. **Never** skip W1.1 to "just get a baseline."

---

### WAVE 2 — CAPTURE ONE LIVE BASELINE · the gate · blocks all porting · ~1 day

| ID | Task | Category | Skills | Verification |
|---|---|---|---|---|
| **W2.1** | Boot `elia-subworker-srv` | `unspecified-high` | `systematic-debugging` | `docker ps` shows it; S6 green |
| **W2.2** | Build the measurement harness | `deep` | `shared/programming`, `verification-before-completion` | harness emits the full record below |
| **W2.3** | Run **N=1** | `unspecified-high` | — | S1 + S2 green at N=1 |
| **W2.4** | Run **N=5** | `unspecified-high` | — | S1 green ×5; first failure point recorded |
| **W2.5** | Sweep **N=10, 15, 20, 25, 30** until OOM or stall | `unspecified-high` | — | **N_max measured, not inferred** |
| **W2.6** | Emit the baseline report | `writing` | `verification-before-completion` | every number labelled MEASURED / ESTIMATE / INFERRED; **no estimated line may appear in the verdict** |

**The harness must capture, at each N:** `docker stats --no-stream` + `docker inspect` (cap, `OOMKilled`, restart count) · per-PID `/proc/<pid>/status` (`VmRSS`, `VmHWM`) and `/proc/<pid>/smaps_rollup` · **process attribution by class** — opencode/Bun, MCP children **counted per directory, never derived**, omo plugin hosts (**counted, not assumed 11**), Chromium, FastAPI, cloudflared · `SELECT` read latency on `message` and `part` (p50/p95/p99) · whether the `event` write path is active (W0.4 decides) · turn-latency p50/p99 under concurrency as the event-loop-lag proxy · `memory.peak`, `memory.events`, OOM kills.

**Gate W2 → W4:** N_max exists as a **measurement**. Every load-bearing claim is now confirmed or falsified.

---

### WAVE 3 — TUI DECOUPLING (port-surface reduction) · independent of the branch

**Why here and not later:** it shrinks the port surface before any porting. **Headline: ~196 LOC inline + wholesale CLI/TUI subtree delete — much smaller than expected, because the server's `/tui/*` HTTP group imports `@opencode-ai/tui` not at all.** It is a pure control channel (18 routes publishing bus events to an already-connected client), and `opencode_client.py` touches **none** of them and opens **no SSE connection** (it polls `/session/status`).

| ID | Task | Category | Skills | Verification |
|---|---|---|---|---|
| **W3.1** | Establish the typecheck/test baseline | `quick` | `verification-before-completion` | `bun turbo typecheck` + `bun run --cwd packages/opencode test` recorded green or red-with-reasons. ts-anatomy ran none; without this, W3.2's breakage is indistinguishable from pre-existing breakage |
| **W3.2** | **Group A inline — unconditional** | `unspecified-high` | `shared/programming`, `test-driven-development`, `refactor` | V0–V5 below |
| **W3.3** | **Group B/D delete — GATED ON W4 selecting a port** | `unspecified-high` | `shared/programming`, `test-driven-development`, `refactor` | §7.3 |

**W3.2 — +196 LOC inlined, zero call-site edits:**

| Action | Source → Destination | LOC |
|---|---|---|
| Inline | `packages/tui/src/util/record.ts` (whole file) → `packages/opencode/src/util/record.ts` | **3** |
| Inline | `packages/tui/src/util/error.ts` (whole file, 4 exports: `cliErrorMessage`, `errorFormat`, `errorMessage`, `errorData`) → `packages/opencode/src/util/error.ts` | **193** |

**Keep the `@/util/error` / `@/util/record` specifiers unchanged** — that is why inlining (not re-pointing) means all **18** consuming server files need zero edits. Both bodies are pure functions with zero tui-internal imports.

**Sequencing (load-bearing):** inline A2 **before** any CLI deletion — 15 of the Group-B sites also import `errorMessage`. Reversed, you eat ~15 spurious type errors.

| | Verification |
|---|---|
| **V0** | `grep -rn --include='*.ts' --include='*.tsx' -E '@opencode-ai/tui' packages/ \| grep -v node_modules \| grep -v '/test/'` → **0 lines** (currently 36). Plus `grep -n '"@opencode-ai/tui"' packages/*/package.json` → no output |
| **V1** | `bun turbo typecheck` green |
| **V2** | `bun run --cwd packages/opencode build` exit 0 |
| **V3** | `bun run --cwd packages/opencode test` **and** `test:httpapi` (`--fail-on-missing --fail-on-skip`) green. V3 is the real regression gate — it exercises the live `HttpApi` tree |
| **V4** | Boot + **S1/S2/S3 green**, and `POST localhost:5655/tui/open-themes` → **404** (intended under (b)) |
| **V5** | `curl -s localhost:5655/doc \| grep -oE '"/(global/health\|session[^"]*\|provider\|instance/dispose)"' \| sort -u` still lists the 11 consumer endpoints |

---

### WAVE 4 — THE RE-DECIDE GATE

Criteria in **§5**. Deliverable: `docs/decisions/0001-port-or-not.md` — the matrix applied to the measured baseline, signed verdict, and the selected branch's fix list.

---

### WAVE 5A — BRANCH (c): FIX IN TYPESCRIPT · **expected branch**

All items are verified-present defects, not speculation. TDD applies to 5A.2, 5A.3, 5A.6.

| ID | Task | Category | Skills | Verification |
|---|---|---|---|---|
| **W5A.1** | **One global plugin host** — 11 × 106 MB → 106 MB, **~1.06 GB reclaimed** | `deep` | `shared/programming`, `ast-grep`, `test-driven-development` | omo process count in the cgroup drops 11→1. omo's only cross-call module state is session-keyed (`plugin/chat-headers.ts:19,62`, cacheKey `${sessionID}:${messageID}`) so directories cannot collide; `proxy-local.ts:1205` `initialised` is a per-process singleton guard — **one process is the intended shape** |
| **W5A.2** | **`event` retention + `wal_autocheckpoint` + `SQLITE_FULL`/`ENOSPC` handling** | `deep` | `shared/programming`, `test-driven-development`, `verification-before-completion` | `event` rows pruned by policy; `wal_autocheckpoint` explicitly set; injected `SQLITE_FULL` yields a logged degrade, not an unhandled throw; `event` stops growing monotonically under soak |
| **W5A.3** | **Call `dispose_instance()`** — it has **zero callers** | `quick` | `shared/programming`, `ast-grep`, `test-driven-development` | `InstanceStore`/`InstanceState` entry count bounded after N directories; unit test asserts eviction |
| **W5A.4** | **Pooled Chromium + session affinity** | `deep` | `shared/programming`, `test-driven-development` | Chromium process count capped and independent of N |
| **W5A.5** | **A real swap budget** — `memswap_limit: 8g == mem_limit: 8g` | `quick` | `verification-before-completion` | `docker inspect` shows `MemorySwap > Memory`; JSC's non-returning heap can no longer turn transient spikes into permanent ratchets |
| **W5A.6** | **Fix the consumer's completion-detection bug** | `unspecified-high` | `shared/programming`, `test-driven-development`, `fastapi-pro` | `pytest tests/unit/test_opencode_client.py::test_wait_for_response_passes_directory` green |

**W5A.6:** `status.ts:39-48` **deletes** the map entry on idle and status is `InstanceState`-keyed by directory. `opencode_client.py:543,557` calls status with **no directory argument** → falls back to `process.cwd()`. **A running session can report idle.** Fix: thread `directory=` through `wait_for_response`/`get_session_status`. Also unimplementable on the Rust base (no `type` field), so any port must change the client — **outside the 25-agent blast radius the team treated as fixed.**

---

### WAVE 5B — BRANCH (b): ADOPT `/tmp/OpenCodeRust` + PORT INCREMENTALLY

**Only if W4 selects it.** Serial. Every item has a blocking predecessor.

| ID | Task | Category | Skills | Verification |
|---|---|---|---|---|
| **W5B.0** | **Pay the admission cost** — 4 wire breaks + `x-opencode-directory` + persistence replacement, **before anything else** | `ultrabrain` | `shared/programming`, `verification-before-completion` | all four breaks' regressions green |
| **W5B.1** | **Replace the persistence layer** | `ultrabrain` | `shared/programming`, `test-driven-development` | no `DELETE`-then-re-`INSERT`; 9 tables present |
| **W5B.2** | **Build the instance/header layer** (586 LOC absent) | `deep` | `shared/programming`, `test-driven-development` | S3 green incl. the precedence triple |
| **W5B.3** | ~~Fix plugin-host blockers B1→B7~~ → **SUPERSEDED BY W5C.** Under A1, B1/B2/B4/B5/B6/B7 are structurally impossible and B3 is moot. The external plugin host is still hardened for **user** plugins, but that is no longer on omo's critical path | `deep` | `shared/programming`, `test-driven-development` | S4 green **with the negative control** |
| **W5B.3a** | **Native tool contribution channel** — `HookEvent::ToolDefinition` is a *transform*, not a *contribution*. Add `PluginSystem::register_tools` so a plugin can **add** tools into `ToolRegistry` (`prompt.rs:1478-1479`). ~150 LOC. **CRITICAL PATH under A1** | `deep` | `shared/programming`, `test-driven-development` | a native plugin registers 11 team tools and they appear in the LLM tool list; unit test on resolution order |

---

### WAVE 5C — BRANCH (c'): OMO AS NATIVE BUILT-IN PLUGINS · **APPLIES UNDER BOTH (b) AND (c)** · ~2–3 weeks

**This is Wael's ruling, and it is the direct replacement for "make the Rust server load omo".** It is a *re-implementation*, not a port. Under (c) it is **not applicable** — (c) keeps the TS server where omo already runs perfectly and pays **1.06 GB** for the privilege (W5A.1). Under (b) or a hybrid, it is **mandatory**: there is no other path to a working fleet.

**The primitives are verified present** (A1.3): `HookEvent` (23 variants, `lib.rs:43`), `PluginSystem::register`/`trigger` (`lib.rs:172`/`:186`), `Hook` with priority (`lib.rs:143`), `ToolContext` + `ToolRegistry` (`prompt.rs:1478-1479`). **What is absent is a registry.** This wave builds one.

| ID | Task | Category | Skills | Verification |
|---|---|---|---|---|
| **W5C.0** | **`internal_plugins()` registry** — the Rust mirror of `plugin/index.ts:67`. `pub trait NativePlugin { fn init(&self, input: &PluginInput) -> BoxFuture<'_, Result<Hooks, PluginError>> }`. `PluginInput` = `{client, project, worktree, directory, server_url}` — **the 5 fields `PluginContext` (`subprocess/client.rs:111-116`) is missing today**. Per-plugin enable flag on `RuntimeFlags` (not an env guess — `disableDefaultPlugins` is all-or-nothing, see A1.4) | `ultrabrain` | `shared/programming`, `test-driven-development` | a no-op native plugin loads, is listed by a registry-dump command, and is individually disableable while the others stay live |
| **W5C.1** | **`register_tools` contribution** (= W5B.3a) — promote it into this wave; it is a prerequisite, not a sibling | `deep` | `shared/programming`, `test-driven-development` | 11 team tools register and reach the LLM tool list |
| **W5C.2** | **`team mode`** — 11 tools + `tool.execute.before` + fs mailbox (port `tolerant-fsync.ts`'s *algorithm*, not its code). **The load-bearing feature for the 25 live agents** | `ultrabrain` | `shared/programming`, `test-driven-development` | **S4(a) green**: `team_send_message` reaches the peer transcript in 60 s, **and S4's noop negative control still fails** |
| **W5C.3** | **`call_omo_agent`** — `session.create` → `promptAsync` → poll `messages` → `abort`. **Second load-bearing feature** | `deep` | `shared/programming`, `test-driven-development` | **S4(b) green**: non-empty final text |
| **W5C.4** | **`ulw-loop`** — `ChatMessage` + `ToolExecuteBefore` + `ChatSystemTransform`, with a **genuine** notification channel (`MessageSent`/`MessageReceived` already in `HookEvent`) | `deep` | `shared/programming`, `test-driven-development` | **S4(c) green**: a second assistant turn with no human input, vs the TS server as reference oracle |
| **W5C.5** | **unlimited-token AI proxy** — `reqwest` CONNECT proxy on 18888, rotate on free-tier/rate-limit verdict, advertise via the `shell.env` hook | `deep` | `shared/programming`, `test-driven-development` | proxy egress IP rotates on a forced rate-limit verdict; `HTTP_PROXY` reaches the provider |
| **W5C.6** | **Delete the `.ts`-to-disk builtin path** — `load_builtins` (`subprocess/loader.rs:156-176`) writes TS strings to a cache dir and subprocesses them. Two auth plugins become two registry entries. **This is the anti-pattern Wael named** | `quick` | `shared/programming`, `verification-before-completion` | no `.ts` written to `~/.cache/opencode/plugins/builtin`; `grep -rn "include_str!.*\.ts" crates/` → 0 hits; auth plugins still authenticate |
| **W5C.7** | **Delete the subprocess host if nothing uses it** — after W5C.6, check whether any **user** plugin is configured. The subworker fleet configures none ⇒ the bun child never starts ⇒ the cgroup holds one process for plugins | `unspecified-high` | `shared/programming`, `verification-before-completion` | `cfg.plugin_origins` empty ⇒ no child spawned (assert via `/proc`); non-empty ⇒ child spawns and user `.ts` plugins still work (**test both**) |

**Sequencing:** W5C.0 → W5C.1 → {W5C.2 ∥ W5C.3} → W5C.4 → W5C.5 → W5C.6 → W5C.7. **W5C.2 and W5C.3 may run in parallel** — different files (`tools/team_*` vs `tools/call_omo.rs`), and they are the two features the 25 live agents depend on. Max 2 concurrent writers per §3.

**W5C is not optional under (b).** Without it, branch (b) ships a server with **zero** team-mode capability and 25 live agents that cannot message each other. W5B.0–W5B.5 alone do **not** produce a shippable product.
| **W5B.4** | Kill `unwrap()`/`expect()` on server paths; add CI | `unspecified-high` | `shared/programming`, `verification-before-completion` | `cargo clippy --workspace --all-targets` clean; CI green on push |
| **W5B.5** | Load test + soak at N_max | `unspecified-high` | `verification-before-completion` | RSS flat over multi-hour soak; zero OOM; turn p99 ≤ TS baseline |

**The 4 wire-level breaks (all blocking):**

| # | Endpoint | Break | Location |
|---|---|---|---|
| **(a)** | `POST /session/:id/message` | `content: String` **required, no `#[serde(default)]`**; `model: Option<String>` receives an **object**; `agent`/`parts` don't exist; `deny_unknown_fields` count = **0** so survivors are silently dropped. **Fails on every call.** | `routes.rs:710-715` |
| **(b)** | `POST /session` | `CreateSessionRequest { parent_id, title, permission }` — **no `agent`**; handler hardcodes `create("default", directory)`. Every subworker runs as the literal agent `"default"` | `routes.rs:370-374` |
| **(c)** | `GET /session/status` | `SessionStatusInfo` has **no `type` field**. Client checks `status.get("type") == "busy"` / `"idle"`; both `None` forever ⇒ `OpenCodeTimeoutError("never became busy")` ⇒ **every run fails** | `routes.rs:357-367` |
| **(d)** | `GET /session/:id/message` | `MessageInfo` is **flat**; client reads `msg.get("info",{})` then `info.get("role")` ⇒ `info` always `{}` ⇒ `extract_assistant_text` returns `""` ⇒ **every subworker posts an empty report** | `routes.rs:1098-1113` |

Plus, outside the four: **no `x-opencode-directory` port.** ts-anatomy retracted "absent" (the header is parsed) but **`ServerState` is single-tenant and no crate provides `InstanceStore`/`InstanceState`/`WorkspaceRoutingMiddleware`** — the entire multi-directory model is missing. Verified in `plugin-host` R4: this is the **only** thing that lets team mode's live delivery work, because `messaging-live-delivery-client.ts:1-15` requires exactly `client.session.promptAsync({ path:{id}, body:{parts, agent, model, variant}, query:{directory} })`.

**W5B.1 — the persistence data-loss vector (verified by me in code):**

```rust
// server.rs:197 sync_sessions_to_storage — called on 19 routes (routes.rs:235 → 413,439,503…1598,1858)
let persisted = session_repo.list(None, 100_000).await?;
for stale in persisted {
    if !snapshot_ids.contains(&stale.id) {
        message_repo.delete_for_session(&stale.id).await?;   // :213 ← DELETE
        session_repo.delete(&stale.id).await?;                // :214  ← DELETE
    }
}
...
message_repo.delete_for_session(&stored_session.id).await?;  // :231  ← DELETE every message, every sync
for message in stored_messages { message_repo.create(&message).await?; }  // then re-INSERT
```

**Every sync deletes and re-inserts every message of every in-RAM session.** If the process dies mid-sync, or a session is merely not loaded into RAM, its persisted messages are gone. On the recovered DB (1,019 sessions / 20,667 messages) this is a live data-loss vector, not a theoretical one. Compounding: `load_sessions_from_storage` (`:183-192`) loads **all** sessions into `HashMap<String, Session>` with `messages: Vec<SessionMessage>` (`session.rs:294`) — so the port holds all history in RAM, which is precisely the budget failure in §5.4.

**Two independent unbounded-RAM defects, both verified:** `SessionManager.events: Vec<SessionEvent>` (`session.rs:754`) is pushed12× and **never truncated**; `drain_events` (`:1113`) has **zero callers**. And `messages: Vec<SessionMessage>` grows per session with no cap. ts-anatomy C.2's verdict: *"TS spills it to a 15 GiB append-only table, Rust spills it to RAM. Neither is acceptable; the Rust one is strictly worse per byte."*

---

## 5. THE RE-DECIDE GATE

### 5.1 The decision

| | |
|---|---|
| **(a)** | Greenfield clean-room Rust rewrite |
| **(b)** | Adopt `/tmp/OpenCodeRust`, port incrementally |
| **(b-narrow)** | Port **storage alone** behind a stable interface (perf-architect's carve-out) |
| **(c)** | Fix config + corruption in TypeScript, no rewrite |
| **(d)** | Unavailable — it *is* this plan |

### 5.2 Explicit criteria

**Select (c) — default — if ANY holds:**

| ID | Criterion |
|---|---|
| **C-1** | The stall **does not reproduce** on the clean DB, single directory, 3 MCP servers. *"If it evaporates, this entire exercise was answering a corruption artifact."* |
| **C-2** | The `Semaphore.make(1)` synchronous handle is **not** binding: p99 `message`/`part` read latency < 20 ms while event-loop lag is dominated elsewhere |
| **C-3** | Peak RSS at the failing point < 4.0 GB of the 8 GB cap **and** swap headroom → the container is not RAM-constrained. *"The RAM problem was **disproven**."* |
| **C-4** | The dominant RSS term is Chromium / MCP / plugin-host, not the opencode process itself |
| **C-5** | Turn latency, not memory, is the reported failure **and** the latency floor is provider network time, not DB |

**Select (b) — adopt + port incrementally — ONLY IF ALL hold:**

| ID | Criterion |
|---|---|
| **B-1** | With a clean DB, the stall **is** reproducible in a single directory with pruned MCP |
| **B-2** | The binding constraint is `Semaphore.make(1)` over sync `node:sqlite` (`sqlite.node.ts:110,:63`) — measured: p99 `message`-table read latency scales linearly with concurrent sessions, event-loop lag > 100 ms attributable to `statement.all()` |
| **B-3** | The reported failure is **latency/CPU**, not RAM |
| **B-4** | The **entire** admission cost of §6 is accepted as the price: 4 wire breaks + `x-opencode-directory` + persistence replacement + the 3 no-test crates |
| **B-5** | A named owner exists for the plugin contract. *"Unowned, and it can ship 3-of-4 headline features silently dead while health checks pass."* |
| **B-6** | **(added by A1)** W5C's ~2–3 weeks is accepted, and **W5C.2 + W5C.3 are funded** — team mode and `call_omo_agent` are what the 25 live agents actually call. Without both, branch (b) is **not shippable** (constraint 7). |

**A1 changed the cost of (b), not its shape.** The original §5.2 required B-4 to accept "the entire admission cost of §6". That cost is now **smaller** — W5B.3's seven blockers collapse into W5C.0 (a registry) + W5C.1 (a tool contribution channel), because the JSON-RPC boundary they all lived on is gone. **B-6 is the new requirement, and it is the expensive one**: the work moved from *fixing a host* to *writing the features*. Anyone reading B-4 as "the port is now cheap" has misread A1. It is **~27k lines of TS re-expressed**, and the honest cost went **up** for the features that matter and **down** for the plumbing.

**Select (b-narrow) — perf-architect's carve-out — if:** B-2 holds but B-1 and B-3 do not, i.e. the sync handle is binding and nothing else is. The carve-out is precise: *"a serialization ceiling **no config change removes**. If measurement shows it is the binding constraint, port **storage alone**. Not the server."*

**Never (a).** `opencode-session` maps module-for-module (14 modules, 1:1, 151 tests). Greenfield discards working structure.

### 5.3 The kill-switch, stated for both branches

> **If the stall does not survive on a clean DB, in one directory, with 3 MCP servers, this exercise was answering a corruption artifact. Stop.**

And the symmetric clause, so the gate cannot be gamed into a rewrite:

> **If the stall does survive all of that, and the binding constraint is not the synchronous DB handle, stop. The port is not the lever.**

### 5.4 What "100 sessions in 8 GB" actually requires — and why the port is not on the list

The measured floor, rebuilt on anchors only (perf-architect's own Round-3 correction, superseding a table he called **56% invented**):

| term | MiB | status |
|---|---|---|
| 1 × Bun server (idle, 3 MCP configured) | **465** | MEASURED (on `vouch-opencode` — a **different container**) |
| 11 enabled dirs × 3 MCP cfg × 1 spawnable × ~50 MB | **550** | MEASURED unit |
| 11 dirs × omo plugin host @ 106 MB | **1,166** | MEASURED (darwin) |
| **subtotal before any Chromium** | **~2,180** | |
| Chromium per engaged `parallel-browser-mcp` | **+275–550 each** | **ESTIMATE — the deciding term** |

**~2.2 GB of the 8 GB before a single browser starts.**

Then the honest ceiling:

- **`memswap_limit == mem_limit == 8g` ⇒ swap is disabled.** Zero-swap cgroup where JSC's non-returning heap turns transient spikes into **permanent ratchets** (pragmatic-architect R2). This alone makes "100 sessions in 8 GB" fragile regardless of language.
- **The container holds FastAPI + cloudflared + MCP children + Chrome**, not just opencode. On perf's numbers:1.1 GB MCP + ~2.7 GB Chromium + 1.06 GB single host + 0.5 GB server ≈ **5.4 GB** — inside 8 GB, **on config fixes alone**.
- ~~**"Pure Rust" is architecturally impossible.** Five requirements each force a JS runtime~~ **— SUPERSEDED BY AMENDMENT A1.** Four of the five are properties of **omo being an external plugin**, not of opencode: the CONNECT proxy (`proxy-local.ts`) becomes ~600 LOC of Rust with `reqwest`; the other three (`registry.ts:192` dynamic `.ts` import, `registry.ts:375` zod introspection, `plugin/src/shell.ts:74,106` `BunShell`/`Buffer`) are **opencode's own user-plugin surface** and are unaffected by where omo runs. `@opentui/*` is also unaffected. Under A1 the 1,166 MB line **disappears entirely** — 11 subprocesses become 0. What remains is: *omo's features can be pure Rust; user-authored plugins still need a bun child.* plugin-host R6's withdrawal still stands and is now the whole argument: *"the 1,166 MB belongs to omo being ~27k lines of TypeScript, not to opencode's language."*
- **Browser ceiling is language-independent:** 8 GB ÷ ~400 MB ≈ **15–18 concurrent browser-driving sessions**. A Rust rewrite moves that number by **~5%**.

**Realistic N, by topology:**

| topology | max concurrent sessions | why |
|---|---|---|
| current (25 dirs, 11 enabled) | **~10** | matches the reported 20-session stall order-of-magnitude; the stall is the cgroup, not the language |
| single dir + history externalization | **~25–40** | 3.1 GB floor, Chrome still competing |
| single dir + pruned MCP + externalized history + swap | **~100** | **achievable without Rust** |

**So:100 sessions in 8 GB requires five things, and all five are TypeScript changes:**

1. one global plugin host (~1.06 GB) — **W5A.1**
2. a pooled single Chromium with session affinity — **W5A.4**
3. history externalized, not RAM-resident — already true in TS (SQLite); **not** true in the Rust base
4. a real swap budget — **W5A.5**
5. MCP pruned to 1 spawnable per directory — config

**What100 sessions would cost if the port *did* proceed, unchanged by it:** the per-session budget is 42 MiB and a spill-to-SQLite + streamed-request design uses ~1 MiB —40× margin — **but the base as it stands does not spill.** `server.rs:183` loads all sessions into RAM. And `83.5 MiB` measured worst-case session history is **2× the entire per-session budget**. perf-architect's condition was explicit: *"any design holding history in RAM per session (exactly what the existing Rust reference does) **fails on your real data, not on a hypothetical**."*

### 5.5 Most likely answer at each gate

| Gate | Most likely | Why |
|---|---|---|
| **W0.3** | **Promote `opencode.fixed`** | R8: identical 1,019 sessions, identical `max(time_created)`. Step 3 makes it provable. Loss ≈0 |
| **W2 (does the stall exist?)** | **It largely evaporates, or reproduces only under multi-directory fan-out** | the corrupt `message` b-tree sits on the hot path of every turn; nobody has ever observed the system running; the 20-session stall is secondhand |
| **W4** | **(c)**, with **(b-narrow)** live if `Semaphore.make(1)` proves binding | the RAM problem was **disproven**; the surviving costs are CPU/latency; a JS runtime stays in-container anyway; plugin-host R7's bar is unmet |
| **(a)** | **~0** | module-for-module session loop already exists in Rust |

---

## 6. PER-SUBSYSTEM PORT SCOPE (IF A PORT PROCEEDS)

### 6.1 Ports cleanly — faithful and adequately tested

| Rust crate | LOC | tests | Note |
|---|---|---|---|
| `opencode-session` | 14,115 | **151** | module-for-module 1:1 (14 modules) against TS `session/` |
| `opencode-provider` | 17,489 | **73** | faithful **and larger**; 21 providers vs TS ~15; has the Responses API TS lacks |
| `opencode-config` | 3,730 | **53** | faithful on shape. ⚠️ unverified whether it keeps TS's per-config-dir `npm install` fork (`config.ts:453-470`) — a per-directory process spawn |
| `opencode-tool` | 8,897 | **17** | core faithful; tree-sitter-bash ported; adds Batch/MultiEdit/CodeSearch/Ls/PlanEnter-Exit |
| `opencode-agent` | 1,792 | **9** | faithful; and it **does** gate tool visibility by permission, which TS does **not** — a genuine improvement |
| `opencode-util` | 900 | **8** | faithful |
| `opencode-lsp` | 975 | **5** | faithful |
| `opencode-watcher` | 303 | **6** | faithful (global singleton, not per-directory) |
| `opencode-permission` | 807 | **5** | mostly faithful, one deliberate divergence (`ask` is async) |
| `opencode-grep` | 330 | **4** | faithful |
| `opencode-server` | 6,832 | **5** | **routes faithful — the instance/header layer is missing.** See 6.2 |

### 6.2 Does **not** port cleanly

| Crate | LOC | tests | Gap |
|---|---|---|---|
| `opencode-storage` | 1,263 | **0** | 🔴 **BIGGEST GAP.** Faithful on SQLite mechanics (sqlx async pool — **better** than TS's sync handle) but **the event-sourcing layer is missing entirely**: no `event`/`event_sequence`, no per-aggregate seq, no projector (455 TS LOC), no `commitDurableEvent` (638 TS LOC). 6 tables vs TS's 9. Plus the DELETE+re-INSERT vector at `server.rs:213/231` |
| `opencode-plugin` | 1,951 | **3** | architecturally divergent by design (JSON-RPC subprocess vs in-process `import()`). **Cannot load omo unpatched.** ⚠️ **A1 changes the verdict:** this crate stops being omo's delivery path entirely. W5C **adds** a native registry alongside it (W5C.0) and W5C.6/7 **delete** `load_builtins`' `.ts`-to-disk path. The crate's 1,951 LOC are kept for **user** plugins only — which the subworker fleet has none of. |
| `opencode-mcp` | 2,376 | **2** | faithful/arguably better —2 tests for the crate that fans out per directory |
| `opencode-types` | 359 | **0** | LOSSY: 359 LOC of serde vs 4,969 LOC of Effect Schema. Hand-written conversion at every boundary |
| `opencode-core` | 267 | **3** | PARTIAL. TS's `listeners[]` + `projectors` map + inline sequential dispatch has no equivalent |
| `opencode-server` | 6,832 | **5** | 106 routes present; **no `InstanceStore`/`InstanceState`/`WorkspaceRoutingMiddleware`**; `worktree` service 192 LOC vs TS 623; no remote-workspace proxying |

**Note a correction:** ts-anatomy suspected `lsp_tool.rs` breaks the build under `default = []`. I ran `cargo check -p opencode-tool` — **it passes**. `lsp_tool.rs` is correctly `#[cfg(feature = "lsp")]`-gated throughout. That risk is **withdrawn**.

### 6.3 Must NOT be ported

| Item | LOC | Why |
|---|---|---|
| `opencode-tui` | 21,418 | 3.6× larger than TS TUI; irrelevant to a server-only port; 24% of the reference's LOC |
| `opencode-cli` | 5,551 | binary composition root, **0 pub items**, **0 tests** |
| `packages/codemode` + `tool/code-mode.ts` | 6,878 + 310 | absent from Rust; a re-derive, not a port. Gated behind `flags.experimentalCodeMode` |

### 6.4 Absent from Rust entirely — must be **built**, not ported

| TS subsystem | LOC |
|---|---|
| `core/src/event.ts` durable commit + `session/projector.ts` | 638 + 455 |
| `middleware/workspace-routing.ts` + `instance-store.ts` + `instance-state.ts` + `lifecycle.ts` | **586** |
| `src/project/*` + `core/src/project/sql.ts` | ~600 |
| `src/sync/*` + `core/src/control-plane/*` + `core/src/workspace*` + proxy middleware | ~1,500+ |
| `packages/http-recorder` | 1,540 |
| `packages/protocol` + `packages/schema` typed errors/cursors | 1,582 + 3,387 |

**Honest score:** 89,698 Rust LOC, **330 test lines = 0.37%**, ~55–65% complete while looking ~85%, **0 CI**, 1 commit, **280 `unwrap()` + 127 `expect()`**, and the 4 wire breaks mean **0% adoptable for this consumer today**. ts-anatomy's category error is the right frame: *"A base can be 70% feature-complete and 0% adoptable."*

### 6.5 The persistence decision (W5B.1) — three options, pick explicitly

| Option | Cost | Verdict |
|---|---|---|
| **Port the event-sourced layer** (1,093 TS LOC + 40 versioned migrations) | High | faithful; keeps replay/idempotency/audit |
| **Keep direct-write CRUD, add incremental sync** | Medium | *Recommended.* Kills the data-loss vector at `server.rs:213/231` with a small diff. Accepts the audit loss **explicitly** |
| **Keep as-is** | Zero | **Refused.** Live data-loss vector on a recovered DB |

Whichever is chosen, it must be **recorded as a design decision, not discovered as an oversight.** ts-anatomy: *"This is a design decision, not an oversight, and the team must make it explicitly rather than discover it."*

---

## 7. THE PLUGIN PARITY CONTRACT

### 7.1 The five requirements that each force a JS runtime — **REVISED BY A1**

| # | Requirement | Anchor | Still forces a JS runtime? |
|---|---|---|---|
| 1 | `.ts` dynamic import for **user** tools | `tool/registry.ts:192` | **YES** — opencode's own surface, unaffected by where omo runs |
| 2 | zod introspection on the hot path, **user** schemas | `registry.ts:375` | **YES** — same |
| 3 | `BunShell` / `Buffer`, **user** plugins | `plugin/src/shell.ts:74,106` | **YES** — same |
| 4 | in-process HTTP CONNECT proxy, port 18888 | `proxy-local.ts` | **NO** → **~600 LOC Rust**, `reqwest` honours `HTTP_PROXY`/`HTTPS_PROXY` natively |
| 5 | `@opentui/*` type resolution | — | **YES** — TUI, out of scope |

**Requirements 1, 2, 3 and 5 are opencode's user-plugin surface. They are not removed by A1 — they never were about omo.** A1 removes only #4 from omo's critical path, and removes the *subprocess host* from omo's delivery entirely.

⇒ **Revised architecture: Rust server + native in-process omo plugins (A1) + a bun child retained only for user-authored `.ts` plugins.** The child's cost is now bounded by "does this deployment use user `.ts` plugins" — for the subworker fleet, **zero**, so the child never starts.

omo's plugin boundary remains **clean and worth preserving as the port spec**: across **7,055 TS/TSX files**, exactly **1 file** references the opencode source tree at all. 248 `@opencode-ai/plugin`, 51 `@opencode-ai/sdk`, 44 `@opencode-ai/plugin/tool`, 3 `@opencode-ai/sdk/v2`, 2 `@opencode-ai/plugin/tui`. **That is exactly why A1 is tractable: omo depends on opencode's *types*, not on opencode's *internals*.** A native re-implementation is a rewrite against a stable surface, not an inversion of a deep coupling.

### 7.2 What "omo features fully working" actually costs — **UNDER A1**

**These costs are now NATIVE RE-IMPLEMENTATION costs, not host-plumbing costs.** Under the old framing, blockers 1–7 (§7.3) were the budget. Under A1 they cost zero because the failure mode they describe — crossing a process boundary — no longer happens.

| Feature | Native implementation | Needs | Risk under A1 |
|---|---|---|---|
| **unlimited-token AI** | `reqwest` CONNECT proxy, port 18888, rotating residential IP; rotate on free-tier/rate-limit verdict; advertise via `shell.env` hook | ~600 LOC Rust. `reqwest` honours `HTTP_PROXY`/`HTTPS_PROXY` natively | **LOW** — was LOW, now with no host risk at all |
| **ulw-loop** | `HookEvent::ChatMessage` + `ToolExecuteBefore` + `ChatSystemTransform` (all exist in `lib.rs:43`) + a genuine `promptAsync` call | A real notification channel. `HookEvent` already carries `MessageSent`/`MessageReceived` | **MEDIUM** — was MEDIUM-HIGH; the `event`-filtered blocker is gone |
| **team mode** | 11 tools registered into `opencode_tool::ToolRegistry` + `tool.execute.before` | `promptAsync` + filesystem mailbox (fs + fsync + locks — `tolerant-fsync.ts` exists for this, port the algorithm not the code) | **HIGH** — unchanged in difficulty. This is the real work |
| **`call_omo_agent`** | `session.create` → `promptAsync` → poll `messages` → `session.abort` | all four exist as Rust methods | **MEDIUM** — was HIGH. No RPC layer to get wrong |

**The trap survives A1, in a new form.** *"Three of four features are gated on the SDK client, not on hooks. Any host that stubs `client` satisfies the letter of the API and destroys the substance."* Under A1 there is no host to stub — **but a native plugin can still take a client and not call it.** S4's round-trip test and its noop negative control are therefore **unchanged and still mandatory**. A1 removes the ability to accidentally stub the client; it does not remove the ability to write a plugin that registers 26 tools and never calls `promptAsync`. **Parity is still proven by S4, never by registration.**

The load-bearing SDK dep remains exactly one file: `messaging-live-delivery-client.ts:1-15`.

### 7.3 The blockers to fix first (W5B.3), in order — **MOSTLY MOOT UNDER A1**

**Read this table as the record of *why* A1 was the right call.** Every one of these blockers is a symptom of the subprocess boundary. A native plugin registry makes them structurally impossible rather than fixed.

| # | Blocker | Location | Status under A1 |
|---|---|---|---|
| **1** | `PluginModule` export shape rejected → **`-32600`** | `plugin-host.ts:404` requires a function; omo exports `{id, server}`; TS handles via `readV1Plugin` (`plugin/index.ts:37`) with no Rust equivalent | **MOOT.** Nothing is serialised. A native plugin is a Rust function returning `Hooks`. The `{id, server}` shape is never inspected. (Still fixed upstream for external plugins.) |
| **2** | Tool registry discarded — returns `{name, hooks, auth}`, no `tools`, no `tool.execute` RPC. All 26 omo tools unreachable, including every `team_*`, `call_omo_agent`, `background_output`. No bridge for `ToolContext.metadata()/.ask()/.abort` | `plugin-host.ts:441-445` | **MOOT.** Native tools are inserted into `opencode_tool::ToolRegistry` (`prompt.rs:1478-1479`) directly. `ToolContext` already exists — the three methods are free. |
| **3** | **`event` + `config` filtered out** | `plugin-host.ts:422` filters `auth`, `event`, `config`. Live: **12 hooks, not 14** | **MOOT.** `HookEvent` is a Rust enum with no allowlist to filter. `MessageSent`/`MessageReceived`/`ConfigLoaded` are variants. Note `config` remains a *mutation* hook upstream — natively it becomes `&mut Config`, which is semantically **correct**, unlike the rejected `ConfigLoaded` variant. |
| **4** | noop SDK client → features silently dead | `plugin-host.ts:242-260,367` | **MOOT.** No host, no stub. **But see §7.2 — a native plugin can still ignore its client. S4's noop negative control is retained.** |
| **5** | no `tool.execute` RPC | absent | **MOOT.** Tool execution is a function call. |
| **6** | node fallback dead (`--experimental-strip-types` needs ≥22.6; this machine is v20.20.2) | `subprocess/runtime.rs:28-31` | **MOOT.** No node. |
| **7** | stdin `once("readable")` lost-wakeup race | `plugin-host.ts:96-103` | **MOOT.** No stdin. |

**The one blocker that A1 does not delete: the `tool` hook's contribution channel.** `HookEvent::ToolDefinition` exists (`lib.rs:60`) but it is a **transform**, not a **contribution** — a plugin cannot yet *add* tools. That is **W5B.3a** and it is the critical path for team mode's 11 tools. It is a ~150 LOC change: a `register_tools` contribution on `PluginSystem`, resolved into `ToolRegistry` construction.

**Also absent natively** (unchanged from the original finding, now build items): `dispose` and `experimental.compaction.autocontinue`.

**Parity is verified by S4, never by registration.** A patched host reports **12 hooks + 26 tools, no error** while team mode, call_omo and ulw-loop are all dead. Under A1 the same failure mode is reachable by a plugin that registers tools and never calls the client — so the rule stands verbatim.

### 7.4 The conditional TUI carve-out — a correction the analysts forced

ts-anatomy's Appendix A said delete `src/plugin/tui/**`. **That was withdrawn**: it carries `PluginModule.tui`, which loads `tui-switch-proxy.tsx` (`SolidPlugin` from `@opentui/solid`) — **Wael's proxy switcher**.

| Branch | `src/plugin/tui/**` |
|---|---|
| **(b) port** | **REVIEW-THEN-KEEP**, gated on a runtime assertion: boot the CLI, open the sidebar, confirm it populates. Add as **V4.5** — *"A green typecheck is not evidence for deleting a user-facing surface."* |
| **(c) no rewrite** | **KEEP.** Also **W3.3 does not execute** — deleting `src/cli/**` removes the operator interface entirely. W3.2's 196-LOC inline is the **entire** TUI work under (c). |

**Also gated before any `src/cli/` deletion:** read `bin/opencode` and verify `serve` routes to the server entrypoint, not the CLI arg parser. ts-anatomy: *"This is the one real behavioural risk in the whole change."*

**Unresolved, must be checked during W3.2:** reverse importers of `test/fixture/tui-{environment,sdk,plugin,runtime}.*`. If one is used by `test/server/**`, it needs rewriting, not deleting.

---

## 8. WHAT THIS PLAN REFUSES TO DO

1. **Refuses to write porting code before the baseline gate.** Not one line. *(All five members demanded it.)*
2. **Refuses to promote `opencode.fixed` after starting the container.** plugin-host R9.
3. **Refuses to run the measurement before the `bypassAgentCheck` fix.** Running it means running 25 root agents with a live permission bypass.
4. **Refuses to derive the MCP count instead of counting it.** opencode reads `cfg.mcp` — 3 entries; the 54-server `mcp_servers.json` is the mcp-cli catalogue, never read by the server (`mcp/index.ts:496`; `grep -rn mcp_servers packages/` → **0 hits**).
5. **Refuses to skip the 480 TS test files as the parity baseline.**
6. **Refuses to treat "the plugin loaded and registered 26 tools" as parity evidence.**
7. **Refuses to delete `src/plugin/tui/**`** — carries `PluginModule.tui` → the proxy switcher.
8. **Refuses to bundle retention into the port** — *"a rewrite doesn't add a byte of pruning."*
9. **Refuses to adopt `/tmp/OpenCodeRust` while it holds history in RAM** with 0.37% tests and no CI.
10. **Refuses to keep `server.rs:213/231`'s DELETE-then-re-`INSERT`.**
11. **Refuses to call config fixes hypothetical** — `dispose_instance` has **zero callers**, verifiable right now.
12. **Refuses to derive causation between disk pressure and corruption.** Mechanism is standard; causation unproven. Stated as a time bomb plus a missing error path, not a proven cause.
13. **Refuses (a).** Nobody reaches greenfield.
14. **Refuses to claim A1 makes the port "pure Rust, no JS anywhere."** Verified: `registry.ts:192` (dynamic `.ts` import) and `registry.ts:375` (zod introspection) are **opencode's own user-plugin surface**. A1 moves omo; it does not delete the external plugin host. A claim stronger than that would be false and would fail at the first user-authored `.ts` plugin. *(A1.5)*
15. **Refuses to treat W5C as optional under (b).** 25 live agents call `team_send_message` and `call_omo_agent`. A branch that ships without W5C.2 and W5C.3 is a server that cannot run the fleet. **Constraint 7 blocks cutover.** *(A1, §10.12)*
16. **Refuses to accept "26 tools registered" as W5C evidence.** The A1 trap in new form: a native plugin can register every tool and never call `promptAsync`. S4's round-trip test and its noop negative control are **retained unchanged** — A1 removes the ability to accidentally stub the client, not the ability to write a dead plugin. *(§7.2)*
17. **Refuses W5C.6 before W5C.2–W5C.5.** Deleting `load_builtins` while it still carries working auth, before the native replacements are proven, trades a working capability for a clean commit log.

### Honest risks

| Risk | Mitigation |
|---|---|
| The promotion loses data if R8's timestamp equality is wrong | W0.1 snapshot + W0.3's `session` diff makes it provable before the rename |
| The `bypassAgentCheck` fix breaks legitimate delegation if done naively | S5 asserts **both** halves; `(b)` is the half a naive fix breaks |
| The container may not boot on the promoted DB — `event` absent-vs-corrupt unresolved; three state generations coexist | W0.4's `.recover` decides; if boot fails, escalate to rebuild |
| The measurement is **impossible** if the container cannot boot | Then the "20-session stall" ground truth is **unrecoverable** — escalate to Wael rather than fabricate a baseline |
| `.fixed` may not match the deployed schema if the container's opencode ≠ what wrote it | **W1.2 pins the artifact; this is the fix** |
| Under (b), the client must change — 25 live agents' blast radius, explicitly outside a server-only port | W5B.0 + W6 dual-run/shadow traffic are blocking |
| W3.3 is irreversible from the operator's perspective and wrong under (c) | Gated on W4 |
| omo pins `@opencode-ai/plugin` **1.15.13** while the server is **1.18.34** — skew unexamined, may itself be a break | Must be checked in W5B.0 before any parity claim |
| **A1: W5C is ~2–3 weeks of re-implementation, not a config change** | §7.2 re-costs every feature; **B-6** makes the funding an explicit gate condition, not an assumption. Mitigation: W5C.2 ∥ W5C.3 is the only sanctioned 2-writer slot |
| **A1: omo loses independent upgrade lifecycle** — it becomes compiled-in | Accepted by the ruling (A1.4). Mitigation: features live behind per-plugin flags on `RuntimeFlags`, so a bad build disables one feature rather than requiring a revert |
| **A1: `disableDefaultPlugins` is all-or-nothing** — no per-plugin off switch exists upstream | W5C.0 adds one on `RuntimeFlags`. Without it, S4's negative control cannot be run natively — it would have to kill every plugin including the auth ones |
| **A1: the tool contribution channel is the real critical path**, and it is 150 LOC that does not exist yet | W5C.1 is sequenced **before** W5C.2/W5C.3 (constraint 8) and carries its own unit test on resolution order. This is the item most likely to reveal that a team tool needs something `ToolContext` does not expose |
| **A1: the win is bounded by user plugins, not by omo** — requirements 1/2/3/5 of §7.1 survive | Stated in A1.5 so no stakeholder believes the external host is dead. If the fleet ever adopts a user `.ts` plugin, W5C.7's second direction becomes load-bearing |

---

## 9. ATOMIC COMMIT STRATEGY

One concern per commit. **TDD pairs are one commit, not two** — a red-test-only commit breaks CI. The red state is proven by running the test first and recording the failure output in the commit body.

| # | Commit | Branch | Gate |
|---|---|---|---|
| 1 | `chore(db): snapshot opencode-data artifacts + sha256 manifest` | `main` | W0.1 |
| 2 | `chore(disk): remove2.15 GiB orphaned opencode DB artifacts` | `main` | W0.2 |
| 3 | `docs(db): rollback-vs-recovery decision for opencode.fixed` | `main` | W0.3 |
| 4 | `fix(security): derive bypassAgentCheck from prompt content` | `fix/bypass-agent-check` | W1.1 |
| 5 | `fix(docker): pin opencode release tag + add endpoint fingerprint` | `main` | W1.2 |
| 6 | `test(baseline): add N-session memory + latency measurement harness` | `spike/baseline` | W2.2 |
| 7 | `docs(baseline): measured N_max + per-class RSS attribution` | `spike/baseline` | W2.6 |
| 8 | `refactor(opencode): inline util/record + util/error, drop tui dep on server` | `refactor/tui-decouple` | W3.2 |
| 9 | `docs(decision): port-or-not re-decide gate result` | `main` | W4 |
| 10–14 | one commit per W5A fix, each with its regression test | `fix/*` | W5A |
| 15+ | one commit per W5B subsystem, each with conformance tests | `port/*` | W5B |
| 16+ | `feat(plugin): native internal_plugins registry (PluginInput + per-plugin enable)` | `port/*` | W5C.0 |
| | `feat(plugin): native tool contribution channel (register_tools -> ToolRegistry)` | `port/*` | W5C.1 |
| | `feat(team): native team mode — 11 tools + fs mailbox` | `port/*` | W5C.2 |
| | `feat(omo): native call_omo_agent` | `port/*` | W5C.3 |
| | `feat(ulw): native ulw-loop on genuine event channel` | `port/*` | W5C.4 |
| | `feat(proxy): native CONNECT proxy on 18888` | `port/*` | W5C.5 |
| | `refactor(plugin): delete .ts-to-disk builtin path` | `port/*` | W5C.6 |
| | `chore(plugin): drop bun child when no user plugins configured` | `port/*` | W5C.7 |

**W5C commits are ordered by dependency, not by convenience.** W5C.0's registry is one commit because W5C.1's tool channel cannot compile without `PluginInput`. W5C.6's deletion lands **after** the features that replace it — deleting `load_builtins` before team mode exists removes working auth for no gain.

**Rules:** never mix a refactor and a behaviour change; never bundle two subsystems; never commit without the verification output in the body; port commits land behind a feature flag with shadow-traffic evidence attached.

---

## 10. SUCCESS CRITERIA

1. **`elia-subworker-srv` runs**, and its artifact is identifiable (`docker inspect` → exact version + sha256).
2. **S6 green** — `integrity_check: ok`, `message`/`part`/`event` all readable.
3. **S5 both halves green** — the permission bypass is closed without breaking delegation.
4. **S1/S2/S3 green against the real container** at N=1 and N=N_max.
5. **N_max is a measurement**, with per-class RSS attribution and no estimated line in the verdict.
6. **W4 verdict signed** against the explicit criteria in §5.2, with the losing branch's argument recorded.
7. **V0 = 0 `@opencode-ai/tui` imports**, typecheck + `test:httpapi` green.
8. **If (c):** W5A.1–W5A.6 shipped, each with a regression test, and the 8 GB budget re-measured afterwards.
9. **If (b):** all four wire breaks green, `x-opencode-directory` honoured with the full precedence triple, persistence DELETE vector removed, S4 green **with its negative control**, and dual-run + shadow traffic clean before cutover.
10. **If (b) or hybrid — W5C complete (A1's shipping condition).** `internal_plugins()` exists with per-plugin enable; **11 team tools + `call_omo_agent` registered and reachable**; S4(a)(b)(c) green **while the noop negative control fails**; zero `.ts` written to the builtin cache dir; `team_send_message` reaches a peer transcript within 60 s.
11. **If (b) or hybrid — no phantom child.** With `cfg.plugin_origins` empty the bun child **never spawns** (asserted via `/proc`), and with it non-empty the child spawns and user `.ts` plugins still work. **Test both directions** — one direction proves nothing.
12. **W5C.2 and W5C.3 are funded.** Without team mode and `call_omo_agent` there is no product. A green W5B without W5C is not a shippable branch.

---

## 11. TODO LIST

```yaml
- id: wave0-freeze-disk
  content: "W0.1+W0.2: snapshot opencode-data artifacts with sha256, then delete opencode.db.corrupt/-shm/-wal/-new (2.15 GiB) — frees disk at 95% before any DB write"
  status: pending priority: high

- id: wave0-decide-db
  content: "W0.3: diff the session table between opencode.fixed and the corrupt file; promote .fixed only if not a strict superset — this is a ROLLBACK (Sep 23) not a recovery, and MUST precede starting the container (plugin-host R9)"
  status: pending
  priority: high

- id: wave0-recover-event
  content: "W0.4: sqlite3 .recover the corrupt file and settle whether `event` is absent or corrupt, plus its size — decides whether perf-architect's 15 GiB win is real here"
  status: pending
  priority: high

- id: wave1-fix-bypass
  content: "W1.1: remove the bypassAgentCheck derivation at session/prompt.ts:1223 + plumbing at :1231 and session/tools.ts:46,64, KEEPING the legitimate hardcoded true at :331 — test BOTH halves (TDD)"
  status: pending
  priority: high

- id: wave1-pin-artifact
  content: "W1.2: replace Dockerfile:48-56 releases/latest with a pinned ARG tag + image labels + endpoint fingerprint script — the 'pinned, not main' comment is false"
  status: pending
  priority: high

- id: wave2-measure-baseline
  content: "W2: boot elia-subworker-srv and capture ONE baseline — N=1/5/sweep to N_max, per-class RSS via /proc + smaps_rollup, message/part read latency p99, MCP children COUNTED per directory — this gate blocks all porting"
  status: pending
  priority: high

- id: wave3-tui-inline
  content: "W3.2: inline packages/tui/src/util/{record,error}.ts into packages/opencode/src/util/ (196 LOC), keep @/util/error specifiers so all 18 call sites need zero edits; verify V0-V5"
  status: pending
  priority: medium

- id: wave4-redecide
  content: "W4: apply the explicit (c) vs (b) vs (b-narrow) criteria to the measured baseline and sign docs/decisions/0001-port-or-not.md — no porting code before this returns"
  status: pending
  priority: high

- id: wave5a-fix-typescript
  content: "W5A (only under (c)): one global plugin host (~1.06 GB), event retention + wal_autocheckpoint + SQLITE_FULL handling, call dispose_instance (zero callers today), pooled Chromium, real swap budget, fix the client's missing directory arg in wait_for_response"
  status: pending
  priority: medium

- id: wave5b-admission-cost
  content: "W5B (only under (b)): fix the 4 wire-level breaks (content required / agent hardcoded default / no type field / flat envelope), build the absent 586-LOC instance+header layer, and replace the DELETE-then-re-INSERT persistence at server.rs:213/231 — NOTE W5B.3's 7 plugin-host blockers are superseded by W5C"
  status: pending
  priority: low

- id: wave5c-registry
  content: "W5C.0 (only under (b), per A1): build the Rust internal_plugins() registry mirroring plugin/index.ts:67 — NativePlugin trait + PluginInput{client,project,worktree,directory,server_url}, per-plugin enable flag on RuntimeFlags (disableDefaultPlugins is all-or-nothing). The primitives exist (HookEvent lib.rs:43, PluginSystem lib.rs:172, ToolContext prompt.rs:1478); the registry does not"
  status: pending
  priority: medium

- id: wave5c-tool-channel
  content: "W5C.1 = W5B.3a (A1 critical path): add PluginSystem::register_tools so a native plugin can ADD tools to ToolRegistry — HookEvent::ToolDefinition is a transform, not a contribution. ~150 LOC. Blocks W5C.2 and W5C.3; without it team mode's 11 tools cannot exist"
  status: pending
  priority: medium

- id: wave5c-team-mode
  content: "W5C.2 (only under (b)): native team mode — 11 tools + tool.execute.before + fs mailbox (port tolerant-fsync.ts's algorithm, not its code). LOAD-BEARING for the 25 live agents. Verify: S4(a) team_send_message reaches peer transcript in 60s AND S4's noop negative control still fails"
  status: pending
  priority: medium

- id: wave5c-call-omo
  content: "W5C.3 (only under (b)): native call_omo_agent — session.create to promptAsync to poll messages to abort. Second LOAD-BEARING feature. May run parallel to W5C.2 (distinct files). Verify: S4(b) non-empty final text"
  status: pending
  priority: medium

- id: wave5c-ulw-loop
  content: "W5C.4 (only under (b)): native ulw-loop on HookEvent::ChatMessage + ToolExecuteBefore + ChatSystemTransform with a GENUINE notification channel (MessageSent/MessageReceived already in the enum). Verify: S4(c) second assistant turn with no human input, vs the TS server as oracle"
  status: pending
  priority: low

- id: wave5c-proxy
  content: "W5C.5 (only under (b)): native unlimited-token proxy — reqwest CONNECT on 18888, rotate on free-tier/rate-limit verdict, advertise via the shell.env hook. Verify: egress IP rotates on a forced rate-limit verdict"
  status: pending
  priority: low

- id: wave5c-delete-ts-builtins
  content: "W5C.6 (only under (b)): DELETE the .ts-to-disk builtin anti-pattern — load_builtins (subprocess/loader.rs:156-176) writes TS strings to ~/.cache/opencode/plugins/builtin and subprocesses them; the 2 auth plugins become registry entries. This is the anti-pattern Wael named. Verify: grep -rn 'include_str!.*\\.ts' crates/ returns 0 hits, no .ts written, auth still works"
  status: pending
  priority: low

- id: wave5c-drop-child
  content: "W5C.7 (only under (b)): after W5C.6, check whether any USER plugin is configured — the fleet configures none, so the bun child never starts. Verify BOTH directions: empty plugin_origins means no child in /proc, non-empty means child spawns and user .ts plugins still work"
  status: pending
  priority: low

- id: wave6-cutover
  content: "W6: dual-run + shadow traffic + staged cutover for the 25 live agents — required before any port branch ships, and BLOCKED until W5C returns (constraint 7): a green W5B without W5C ships zero team-mode capability"
  status: pending
  priority: low
```

---

## 12. EXECUTION INSTRUCTIONS

1. **Execute waves in order. A wave's gate is a hard stop** — do not start the next wave on a partial pass.
2. **Wave 0 and Wave 1 are unconditional.** They are ~1.5 days and fix a live availability incident plus a live permission bypass. Do them regardless of what W4 eventually says.
3. **Wave 2 is the deliverable.** Its output is a measurement, not a port. Commit it as such.
4. **Wave 3 may run concurrently with Wave 2** — different repo, and it shrinks the port surface either way.
5. **Wave 4 is a stop-and-think.** Apply §5.2 literally. If in doubt, choose (c): the team converged there from four different directions.
6. **Do not begin Wave 5B by "starting small".** The 4 wire breaks are on the critical path of every single subworker run — a base that cannot deserialize the client's payload is 0% adoptable regardless of how little else is missing.
7. **Max 2 concurrent code-writing agents.** W5B is serial. **W5C.2 ∥ W5C.3 is the only sanctioned 2-writer slot inside the Rust tree.**
8. **Every task runs its verification before reporting done.** Evidence before assertions.
9. **Under (b), W5C is not optional and not parallel-to-consideration.** It is the branch's product. Sequence `W5C.0 → W5C.1 → {W5C.2 ∥ W5C.3} → W5C.4 → W5C.5 → W5C.6 → W5C.7`.
10. **Under (c), do not start W5C.** omo already runs natively in the TypeScript server; W5C is ~2–3 weeks that buy nothing (c) is not trying to achieve. The (c) cost is W5A.1's 1.06 GB, and that is the trade being made consciously.
11. **When W5C reports done, the evidence is S4's round-trip, not a registry dump.** Print the hook and tool counts *and* run the noop negative control. A count without the control is the exact failure this plan refuses in §8.16.

<task_metadata>
session_id: ses_ef675526bffeFdR8ptusyK7i5c
task_id: ses_ef675526bffeFdR8ptusyK7i5c
subagent: plan
</task_metadata>
---
## A2. AMENDMENT LOG

| Amendment | Date | Ruling | Sections touched | Verified how |
|---|---|---|---|---|
| **A1** | 2026-10-05 | Wael: *"opencode rust can't load omo — who care? you can just recode it inside opencode as a builtin plugin like I said, ADD IT INTO THE PLAN."* Recode omo as opencode **internal (built-in) plugins**. No subprocess, no JSON-RPC, no JS runtime for omo. | §0, §2 (+2 constraints, graph), §3 (Round 8b), §5.2 (B-6), §6.2, §7.1, §7.2, §7.3, §8 (+4 refusals), §9, §10 (+3), §11 (+8 todos), §12 (+3), §5.4 | Read in source, not inferred: `packages/opencode/src/plugin/index.ts:67` (`internalPlugins`), `:147-168` (`PluginInput` fields), `:170` (direct invocation), `effect/runtime-flags.ts:19` (kill-switch); `/tmp/OpenCodeRust/crates/opencode-plugin/src/lib.rs:43,143,172,186`, `subprocess/client.rs:111-116`, `subprocess/loader.rs:156-176`, `opencode-session/src/prompt.rs:1478-1479` |

**A1 was written by hand, not by the subagent.** The delegated amendment task returned `Cannot connect to API` and produced no output; the file was unchanged on disk when picked up. Every claim in A1 above was re-verified against the source in that session.

**What A1 does NOT change, restated so nobody mistakes it for a green light:** W2's measurement gate, W4's verdict, the (c)-is-expected position, and §8's refusal list. A1 removes a blocker list. It does not create evidence.

---

## PROVENANCE
- Bundle assembled by lead from 5 members x 3 rounds (Round 1 findings, Round 2 cross-attack, Round 3 final position):
  /tmp/rust-port-findings/{feasibility-skeptic,perf-architect,plugin-host,pragmatic-architect,ts-anatomy}{,-crossattack}.md
- Bundle: /tmp/rust-port-findings/BUNDLE.md
- Sequencing/parallelisation/verification gates owned by the `plan` agent (session ses_ef675526bffeFdR8ptusyK7i5c), not by the lead.
- Lead-verified facts: production DB corrupt (message/part/event unreadable, btreeInitPage error 11);
  opencode.fixed integrity ok but OLDER (rollback not recovery); no elia-subworker-srv container exists.
