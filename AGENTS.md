# AGENTS.md — OpenCodeRust

A standalone Rust port of the [opencode](https://github.com/anomalyco/opencode) engine
(CLI + TUI + HTTP server): a harness for running local AI agents. It has no runtime
dependency on any other system and no reference to one.

The CLI binary is named **`opencode`**, but the crate is `opencode-cli` — that mismatch
breaks every command guess. The npm/brew entry point is **`opr`**.

## This is the public mirror

This repo is the public build. It is the same engine as `vakandi/opencode-rust-private`,
with the builtin proxy system removed:

- here — 20 crates, one shared HTTP client, no proxy crate, no `proxy` field in `/health`.
- private — 21 crates, `crates/opencode-proxy` included: per-session egress, error-driven
  rotation, tunnel severing.

The removal is structural, not a feature flag: the crate is deleted and its wiring stripped.
Do not reintroduce it behind `#[cfg(feature = "proxy")]`, do not vendor it, and do not make
this tree depend on code that only exists in the private repo.

`npm`, Homebrew and the `v*` release workflow publish from **this** repo. The private repo's
publish workflow is disabled for that reason.

## Provenance and history

`git log` was rewritten with `git-filter-repo` to purge three credentials that had been
committed (an OpenRouter key, a Playwright-MCP extension token, and a Directus admin
token). Commits before that rewrite are **not reachable from any remote** — do not try to
restore them, and do not add a second remote pointing at the pre-rewrite history.

`config-home/` is gitignored and holds a personal agent-fleet config that was never part of
this project. `config-home.example/` is the tracked, scrubbed counterpart. The only
`opencode.jsonc` that belongs to this repo is the one at the root.

## Commands

```bash
cargo test --workspace                    # full suite (~570 tests)
cargo test -p opencode-cli                # one crate
cargo test -p opencode-cli --test run_turn_loop   # one integration test file
cargo test -p opencode-session --lib      # only inline #[cfg(test)] units
cargo build --release --bin opencode      # what install.sh runs
cargo build -p opencode-cli               # debug binary for ./target/debug/opencode
cargo clippy --workspace --all-targets    # the only lint configured
```

No CI, no `rustfmt.toml`, no `clippy.toml`, no `deny.toml`. Formatting is stock rustfmt;
clippy is advisory — nothing enforces either.

Run the binary from the repo, not from `$PATH`: `./target/debug/opencode <sub>` or
`cargo run -p opencode-cli -- <sub>`. A global `opencode` shadows this one.

`./install.sh` writes `~/.local/bin/opr`, a **generated dispatching wrapper**, not a symlink:
it forwards a known subcommand verbatim and otherwise falls through to `opencode tui`. So
`opr --some-flag` becomes `opencode tui --some-flag`. Editing the wrapper is gone after the
next install; change `install.sh` instead. `--force` defeats the source-fingerprint stamp that
otherwise skips the rebuild.

`clean.sh` refuses to run while this repo's `opencode` is live, and warns before touching a
foreign one. Never delete `target/` by hand while a server or agent run is in flight.

## Two SQLite schemas — do not mix them

This is the highest-cost trap in the repo. "omo" is the upstream TypeScript opencode host —
this port interoperates with a host already running on the machine.

| Who | Path | Tables |
|---|---|---|
| This port | `<root>/opencode/opencode.db` | **plural** — `sessions`, `messages`, `parts` |
| omo (host) | `~/.local/share/opencode/opencode.db` | **singular** — `session`, `message`, `part` |

`crates/opencode-cli/src/children.rs` queries omo's singular `session` table to discover
subagents. Point it at this port's database (or vice versa) and every query returns zero rows,
which reads as "no children" — a silent false negative, not an error.

Migrations are **not** in a `sqlx/migrations/` directory (none exists). They are raw SQL string
constants in `crates/opencode-storage/src/schema.rs`, collected in `ALL_MIGRATIONS` and replayed
on every connect. Adding a schema change means appending there. All queries use runtime
`sqlx::query(...)`, never the compile-checked `query!` macro, so there is no `DATABASE_URL` or
offline query cache to set up.

## `run` is the load-bearing path

`run` must drive `SessionPrompt::prompt_with_update_hook` (`crates/opencode-session/src/prompt.rs`).
It must **not** use `AgentExecutor::execute_streaming` — that path makes one provider call and
never executes a tool, so a spawned subagent prints `[Calling tool: …]` and dies with the process.
The TUI masked this for a long time because the TUI never exits.

After the turn, `run` deliberately holds the process open until four things settle:
the turn itself → the `session.idle` plugin notification → omo's child sessions going quiet →
the `.omo/run-continuation/<session>.json` ledger. Every stage **fails open** (absent db, missing
file, corrupt JSON, or a stale `active` entry all mean "done") — never tighten this into a hang.
`--wait-background-timeout <SECONDS>` defaults to 120, so a test that shells out to `run` can sit
idle for two minutes before it fails.

## Loop invariants in `prompt.rs::loop_inner`

Both are commented in-source with the reason; keep them:

- **A turn ends only when the last assistant message has text *and* no tool call.** Text emitted
  alongside a tool call is narration ("I'll search now…"), not an answer. Treating it as the end
  closes the loop before the model reads the tool result.
- **Message ordering comes from `rposition` over the vector, never from comparing ids.** Ids are
  random UUIDs, so a lexicographic comparison decides the branch by coin flip and silently drops a
  legitimate turn about half the time.

## Framing and config landmines

- MCP stdio is **newline-delimited JSON**. It is not LSP's `Content-Length: N\r\n\r\n{…}`.
  Both framings exist in the tree; using the wrong one makes every MCP server time out at boot
  while ignoring your bytes entirely.
- `McpServerConfig` has a **hand-written `Deserialize`** for a reason: as an untagged enum, the
  `{enabled}` toggle variant matched the full local-server variant first (it also carries
  `enabled`), so every server definition was discarded and a "successful" boot exposed zero tools.
  Do not "simplify" it back to `#[serde(untagged)]`.
- MCP servers boot in parallel under a 90 s ceiling. Sequential boot was the third cause of the
  same silent-MCP-death family.

## Paths

`OPENCODE_PATHS_ROOT` is the single lever for the database, provider auth, MCP auth and
well-known config — resolve them through `opencode_paths::{db_file, data_file,
provider_auth_dir}`. A blank or whitespace-only value counts as unset (a stray
`export OPENCODE_PATHS_ROOT=` must not relocate the db into cwd); a relative root anchors to cwd.

Config/state/cache directories still call `dirs::*` directly in several crates — that is
intentional and not a violation. `opencode_paths` governs the *data* files, and it never rewrites
a caller's fallback closure, so the per-platform difference (db under `data_local_dir`, MCP auth
under `data_dir`) stays real.

Config merges from `opencode.jsonc` · `opencode.json` · `.opencode/opencode.jsonc` ·
`.opencode/opencode.json`, searched upward, then `~/.config/opencode/opencode.json[c]`.

## Tests

Two conventions coexist — both are used, so match whichever crate you are in:

- Inline `#[cfg(test)] mod tests` in 68 source files (pure units, no I/O).
- Integration files under `crates/*/tests/*.rs` (23 files) for anything behavioural: wire contracts,
  SSE livestream fidelity, child observability, run lifecycle.

Most integration tests bind a real loopback socket and serve the **real** router
(`opencode_server::routes::router()`), not a hand-written stub. Do not downgrade one to a stub —
`crates/opencode-tui/tests/transcript_real_server.rs` exists specifically because stub fixtures
cannot prove the server emits a shape.

Environment variables are process-global, so tests that mutate them serialize on a `OnceLock`
mutex and restore the previous value on drop (see `serial()` in `crates/opencode-paths/tests/root.rs`).
A new test that writes `OPENCODE_PATHS_ROOT` or `OPENCODE_CONFIG_DIR` without that guard becomes a
flaky race under `cargo test --workspace`. Use `tempfile` for isolation.

No test is `#[ignore]`d and no test needs a live provider or API key — provider tests instantiate
the adapters with `"test-key"`. `ANTHROPIC_API_KEY` / `OPENROUTER_API_KEY` appear in config-path
tests only, as values being resolved.

## Repo shape

Four files dominate and have no clean module boundary — do not assume the file tree maps to the
architecture, and use `codegraph_explore` rather than reading linearly:

- `crates/opencode-cli/src/main.rs` (~195 KB) — every subcommand plus the whole `run` drain.
- `crates/opencode-tui/src/app/app.rs` (~188 KB)
- `crates/opencode-session/src/prompt.rs` (~178 KB) — the turn loop.
- `crates/opencode-server/src/routes.rs` (~156 KB)

The plugin bridge (`crates/opencode-plugin`) spawns Node subprocesses with `kill_on_drop`;
`trigger_collect(HookContext::new(HookEvent::…))` is the fire-and-forget hook path, and the
reverse channel is `sendNotification` correlated by `params.requestId`.

## Docs

`docs/opencode-*.md` is a per-crate index and `docs/BUGS_FOUND_AND_FIXED.md` records the
post-mortems behind most of the invariants above. `USER_GUIDE.md` is user-facing and still calls
the project "RustingOpenCode (ROCode)".

Verify against the code before trusting prose: `README.md` was rewritten against verified
behaviour once already, and `docs/` was not rewritten with it — `docs/README.md` still calls the
project "RustingOpenCode (ROCode)" at a pinned version, and the per-crate docs predate the
packaging work below.

## Distribution

Three surfaces, and they are **not** interchangeable:

- **npm** — `npm/package.json` publishes `bin/opr.js`, a Node launcher. Node, not bash, because
  `install.sh` writes a bash `case` script that Windows cannot execute; rewriting it in Node is
  what made Windows possible. `chalk` is pinned to `^4` because chalk 5 is ESM-only and would
  break the CJS launcher.
- **Homebrew** — `npm/homebrew/opencode-rust.rb`, builds from source via `cargo install`. The
  class name must match the filename or Homebrew will not load it.
- **Source** — `./install.sh`, fingerprint-stamped so an unchanged tree skips the rebuild.

`opr` means different things per surface: npm ships a **dispatcher** (`opr --flag` →
`opencode tui --flag`), Homebrew ships a **symlink** (`opr --flag` → `opencode --flag`, rejected).
Both are intentional. Two real bugs came from assuming otherwise: `install.js` resolved the
binary at `__dirname/../node_modules`, which is outside the package because `install.js` sits at
the package root unlike `bin/opr.js`; and both `opr.js` and `install.js` probed the binary with
`--version`, but the clap definition has only a `version` subcommand, so the probe failed on
every install.

`opencode version` works. `opencode --version` does not.

## Secrets

`config-home/` is gitignored and untracked; `config-home.example/` holds placeholder values
only. Never commit a credential, and never `git add -f` one of the ignored patterns. The
repo is public — anything tracked here is published.