# BUNDLE — Rust port planning (adversarial team output, lead-assembled)

Team: `opencode-rust-port` · teamRunId `bc3f103e-f405-4e5e-b685-014e4bc7f0b3` · 5 members, 3 rounds each.

## Wael's goal (verbatim intent)
Convert the opencode TypeScript server to a **pure Rust** server — fastest, lowest RAM, lowest CPU.
Target: **100 concurrent sessions in 8 GB**. Today it stalls/bugs past ~20 in 8 GB even with good proxies.
"80% of the goals reached is still great."
**Non-negotiable:** every feature of the oh-my-openagent plugin must work in the Rust server
(unlimited-token AI, `ulw-loop`, team mode with agent-to-agent messaging, `call_omo` background
tasks without `delegate_task`), plus `/Users/vakandi/Documents/opencode-proxy-plugin`.
`https://github.com/ChrisFeldmeier/OpenCodeRust` may be used as a template — BUT if used, it must
retain the plugin system.
**Also required:** a mini-plan to delete the `packages/tui` coupling so only the server is ported.

## Artifacts (READ THESE — they are the evidence base)
All in `/tmp/rust-port-findings/`:
- Round 1 independent findings: `feasibility-skeptic.md`, `perf-architect.md`, `plugin-host.md`,
  `pragmatic-architect.md`, `ts-anatomy.md`
- Round 2 cross-attacks: same names + `-crossattack.md` (contain **Round 3** appended)
- `ts-anatomy.md` §APPENDIX A (line 1053) = the finished **TUI-decoupling mini-plan**

Codebases:
- TS source: `/Users/vakandi/Documents/opencode-server-rust` (anomalyco/opencode, server-only, v1.18.34,
  15 packages, ~146–160k LOC in `packages/*/src`; `packages/opencode/src` = 76,151 LOC)
- Rust reference: `/tmp/OpenCodeRust` (19 crates; `cargo check --workspace` exits 0)
- Real consumer: `/Users/vakandi/EliaAI/subworkers/server/app/utils/opencode_client.py` (self-declares v1.18.18)
- Plugin that must survive: `/Users/vakandi/EliaAI/setup/oh-my-openagent`
  (real source = **7,055 TS/TSX files / 1,135,619 LOC** excluding node_modules/dist)

## LEAD-VERIFIED FACTS (measured by me, not inferred)
1. `/Users/vakandi/EliaAI/subworkers/server/opencode-data/opencode.db` — `pragma quick_check` →
   **Tree 60 + Tree 25 `btreeInitPage() returns error code 11`**. `select count(*)` on **message,
   part, and event all fail** with "database disk image is malformed (11)". `session` = 1019 rows, clean.
2. Sibling `opencode.fixed` (1,349,709,824 B, mtime **Sep 23 14:34**) → `integrity_check: ok`,
   `message` = 20,667 rows readable.
   **`opencode.fixed` is OLDER than `opencode.db` (mtime Sep 25 20:34)** ⇒ it is a **ROLLBACK candidate,
   not a recovery.** Renaming it in silently loses Sep 23–25 data. perf-architect and the lead both
   perturbed `opencode.db`'s mtime while auditing (read-only queries); disclose this.
3. `docker ps` shows **no `elia-subworker-srv`** — running or stopped. The 8 GB container has never been
   profiled by this team. `vouch-opencode` (799 MiB / 1.172 GiB limit, 3 MCP) is a **different container**.
4. `memswap_limit == mem_limit` (8g) ⇒ **swap is disabled**.
5. Only **11 of 25** subworkers are `enabled: true` (`app/config/subworkers.json`).
6. opencode reads MCP from `cfg.mcp` (`mcp/index.ts:496`) = **3 entries** in `opencode.json`.
   `grep -rn mcp_servers` over `packages/` → **0 hits**. The 54-server `~/.config/mcp/mcp_servers.json`
   is the **mcp-cli wrapper catalogue for agents**, not the server's runtime config.

## TEAM FINAL POSITIONS (after mutual falsification)
| member | Round 1 | **Round 3 (final)** |
|---|---|---|
| perf-architect | (a) rewrite | **(c)** fix config+corruption, no rewrite. Carve-out: port **storage alone** if the sync handle proves binding. |
| plugin-host | (b) adopt | **(d)** recover→start→measure→decide. If port: **(b) over (a)**. |
| pragmatic-architect | (b) adopt | **(b)** heavily qualified + deferred, spine replaced, gated behind measurement. |
| feasibility-skeptic | (c) fix config | **(d)** recover, measure, then choose. Withdrew (c). |
| ts-anatomy | (c) | **(d)** restore service, pin artifact, capture baseline, re-decide. Never (a). |

**CONSENSUS: (d) — recover, pin the artifact, capture one live baseline, THEN decide.
Nobody reaches a full clean-room rewrite. Three members explicitly prefer (b) adopt-and-port
over (a) if a port happens. The burden of proof for a rewrite was never met.**

## Load-bearing claims that SURVIVED cross-attack
- `instance-state.ts:31` `ScopedCache.make({capacity: POSITIVE_INFINITY})` keyed by `yield* directory`;
  **22 `InstanceState.make` call sites**; MCP clients, LSP servers, and **every plugin incl. omo are
  instantiated per directory** and never evicted. `opencode_client.py:313-331` defines
  `dispose_instance()` and **nothing calls it** (`grep` over `server/app/` = 0 callers).
- `core/src/database/sqlite.node.ts:110` `Semaphore.make(1)` over synchronous `node:sqlite`
  `DatabaseSync`; `statement.all()` (`:63`) is a sync C call on the JS main thread;
  `PRAGMA cache_size = -64000` = one 64 MB page cache on one connection.
- Event-sourced persistence: `Session.updatePart/updateMessage` never touch SQLite, only publish events
  (`session/session.ts:629-643`); writes via projector (`core/src/session/projector.ts:310-328`);
  every durable event = one `BEGIN IMMEDIATE` txn, ~6 statements (`core/src/event.ts:237-363`)
  ⇒ **~35-55 SQL statements per assistant turn**. `event` has **no retention** — no `DELETE` anywhere;
  rows removed only by `ON DELETE CASCADE` on session delete.
- Per-LLM-step waste: full history re-read (`prompt.ts:1093`, `while(true)` at `:1080`), 3-slice spread
  (`message-v2.ts:569-572`), 85 tool schemas deep-cloned per step (`tools.ts:92-101` →
  `transform.ts:1575-1596`), system prompt re-read + remote fetch per step (`instruction.ts:155-169`),
  `text += delta` rope per turn (`processor.ts:297,515`).
- "Pure Rust" is **architecturally impossible** — 5 independent requirements each force a JS runtime:
  `.ts` dynamic import (`tool/registry.ts:192`), zod introspection on the hot path (`registry.ts:375`),
  `BunShell`/`Buffer` (`plugin/src/shell.ts:74,106`), the in-process HTTP CONNECT proxy
  (`proxy-local.ts`, port 18888), `@opentui/*` type resolution.
  ⇒ Realistic architecture is **Rust + a bun/node child in the same cgroup** — which moves bytes between
  processes and saves **zero total RAM**.
- `/tmp/OpenCodeRust` **builds** (`cargo check` exit 0), has a faithful session loop
  (`opencode-session/src/prompt.rs:937`), 99 axum routes, real SQLx/SQLite storage — but is
  **~55-65% complete while looking ~85%**, and has **4 verified wire-level breaks** vs the consumer,
  including **no `x-opencode-directory` support** and a persistence layer doing full-resync
  `DELETE`+re-`INSERT` (`server.rs:197` = data-loss vector).
- `/tmp/OpenCodeRust`'s plugin host **cannot load omo**: `host/plugin-host.ts:404` requires a function
  export; omo exports `{id, server}` → live probe returns `{"code":-32600}`. Tool registry discarded
  (`:441-445`). Missing hooks `event` + `config` (filtered at `:422`) — **`event` IS the ulw-loop
  state machine.** Patched host → loads with 12-14 hooks + 26 tools.
- The consumer has a **live completion-detection bug**: `session/status.ts:39-48` deletes the map entry
  on idle; `opencode_client.py:543,557` calls status with **no directory arg** ⇒ falls back to
  `process.cwd()`. **A running session can report idle.** Independent of any rewrite.
- **No built-in tool permission filter**: `tool/registry.ts:291-340` filters nothing permission-related;
  a `deny` on `bash` does not remove it from the model's tool list.
- `ts-anatomy`: the **live router is `packages/opencode/src/server/routes/instance/httpapi/**`**, NOT
  `packages/server` (a second, parallel "v2" API). A plan reading only `packages/server` ports the wrong API.
- `ts-anatomy`: the Dockerfile resolves **`releases/latest` at build time** under a comment claiming
  "pinned, not main" — the deployed version is unpinned.

## The TUI-decoupling mini-plan Wael asked for
Already delivered: `ts-anatomy.md` line 1053, **APPENDIX A**. Headline: **~196 LOC inline + wholesale
CLI/TUI subtree delete**, far smaller than expected, because the server's `/tui/*` HTTP group does not
import `@opencode-ai/tui` at all.
- Group A (inline, 196 LOC): `src/util/record.ts` (3 LOC) and `src/util/error.ts` (193 LOC) are
  one-line re-exports of `@opencode-ai/tui/util/*`. Copy the bodies in place, **keep the `@/util/error`
  specifiers** so all **18 server call sites need zero edits**.
- Group B (delete): 17 CLI-only sites; then `src/config/tui.ts` (276) + `tui-migrate.ts` (132) +
  `tui-host-attention.ts` (21) become orphaned and can go too — **do NOT inline `TuiConfig`**.
- Verification: 0 `@opencode-ai/tui` imports, `bun run typecheck` green, server boots, `/tui/*` route
  group still serves what the server needs.
- ⚠️ Caveat from ts-anatomy's own concession: do **not** delete `src/plugin/tui/**` — it carries
  `PluginModule.tui`, which loads `tui-switch-proxy.tsx` (the proxy switcher Wael uses).

## WHAT THE PLAN MUST RESOLVE
1. The **ordering constraint**: no porting code before (a) the DB situation is decided and (b) one live
   baseline is captured. Every member demanded this gate.
2. The **rollback-vs-recovery decision** on `opencode.fixed` (older than the corrupt file).
3. A **decision gate** with explicit re-decide criteria: what measurement outcome selects (b) vs (c).
4. If (b): the **4 wire-level breaks** and the **missing `x-opencode-directory`** are blocking, plus
   `/tmp/OpenCodeRust`'s persistence layer must be replaced (data-loss vector).
5. The **plugin parity contract**: the 5 JS-runtime requirements mean the plugin host is a child
   process; the plan must state what parity actually costs and how ulw-loop/team-mode/call_omo are
   preserved and **verified**, not assumed.
6. The **TUI-decoupling mini-plan** must be sequenced as its own early step (it shrinks the port
   surface before any porting starts).