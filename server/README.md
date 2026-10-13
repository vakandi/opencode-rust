# OpenCodeRust

**The Rust engine behind EliaAgent** — 21 crates, 568 tests, no container runtime.

OpenCodeRust is a Rust implementation of OpenCode. It provides the full CLI / TUI / HTTP-server
workflow for local AI agents: sessions, tool execution, MCP, LSP, and a TypeScript plugin bridge.
It is the engine [EliaAgent](https://github.com/vakandi/EliaAgent) runs its subworker fleet on.

<p align="center">
  <img src="https://img.shields.io/badge/core-Rust-DEA584?style=flat-square&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/crates-21-informational?style=flat-square" alt="21 crates">
  <img src="https://img.shields.io/badge/tests-568_passing-success?style=flat-square" alt="568 tests">
  <img src="https://img.shields.io/badge/runtime-no_Docker_no_container-brightgreen?style=flat-square" alt="No Docker">
  <img src="https://img.shields.io/badge/license-MIT-10B981?style=flat-square" alt="License">
</p>

---

## ⚡ Why Rust

OpenCode's engine was TypeScript. It held conversation history in RAM, re-derived the full tool
schema set on every LLM step, and kept per-directory instance state (MCP servers, LSP servers,
plugins) unbounded and never evicted. A Docker runtime existed to *contain* that.

It wasn't enough. Five teams of five agents pinned the CPU at 99% on a MacBook Pro M5 and the
machine became unusable. That was the limit, not the workload.

Rust replaces it. Conversation history spills to SQLite, request bodies stream, tool schemas are
cached per `(model, tool-set)`, and instance state is pooled by config hash with LRU+TTL eviction.
No container, no cgroup, no leak to contain.

---

## 🚀 Quick Start

```bash
git clone https://github.com/vakandi/subworker-opencode-rust.git
cd subworker-opencode-rust
./install.sh          # builds the release binary + installs `opr`
```

`install.sh` fingerprints every `.rs` file plus `Cargo.toml`/`Cargo.lock`. Re-running it when the
code is unchanged takes ~0.07 s and skips the rebuild entirely; `--force` rebuilds regardless.

```bash
opr                                   # interactive TUI
opr run "check this repo for risks"    # one prompt, runs to completion
opr serve --port 3000                 # HTTP server
```

### Named-agent shortcuts

`install.sh` also installs one shortcut per agent, each running a single prompt as that agent:

```bash
gilfoyle  "fix this failing test"     # --agent gilfoyle
setbon    "write 5 ad hooks"          # --agent setbon
googlebot "audit my meta tags"        # --agent googlebot
```

On macOS's case-insensitive filesystem `Gilfoyle`, `Setbon` and `GoogleBot` resolve to the same
files. No prompt given → usage on stderr and exit 2.

---

## 🧠 `run` Does Not Abandon Its Work

The single most important behaviour in this engine.

`run` used to call `AgentExecutor::execute_streaming` — one provider call that **never executed a
tool**. A turn that spawned a subagent, a team, or a background task printed
`[Calling tool: …]` and the process exited, taking that work with it. The TUI masked this because
the TUI never exits.

`run` now drives `SessionPrompt`, the same loop the HTTP server uses, so tools actually run and
awaited subagents complete. After the turn it holds the process open until the work it started is
genuinely finished:

| Stage | What it waits for |
|---|---|
| 1. The turn | `prompt_with_update_hook` returns — tools run, awaited subagents land, pending subtasks drain |
| 2. `session.idle` | Fires the notification omo needs to release its background manager (the CLI never fired this before — a second, independent cause of the cut-off) |
| 3. Children | omo records each spawned subagent/team member as a real row in its own SQLite database; `run` streams their frames and waits for them to go quiet |
| 4. Ledger | `.omo/run-continuation/<session>.json` background-task state, with a staleness guard |

### Observing child sessions

Subagents and team members are **not** opaque. They live in omo's database
(`~/.local/share/opencode/opencode.db`, singular table names — a different schema from the one
this port writes), linked by `session.parent_id`. `run` resolves them, streams their `message` /
`part` rows to stdout as `[subagent: <title>]`, and holds until each stops producing.

The drain fails **open**, never hangs: no database means no children, a missing ledger file means
idle, corrupt JSON means idle, and a stale `active` entry (this repo carries three) is treated as
finished after the staleness window.

`--wait-background-timeout <SECONDS>` caps the wait (default 120 s).

---

## 🔌 MCP Actually Works

MCP was declared in config and silently dead. Three separate defects, each verified in isolation:

| Defect | Symptom | Cause |
|---|---|---|
| stdio framing | Every MCP server timed out at boot | The transport wrote `Content-Length: N\r\n\r\n{…}` — **LSP framing**. MCP stdio is newline-delimited JSON, so every server ignored our bytes |
| config deserialisation | Zero tools after a "successful" boot | `McpServerConfig` was an untagged enum whose toggle variant `{enabled}` matched a full local server first (it also carries `enabled`), so every definition was discarded. Replaced with an explicit `Deserialize` |
| sequential boot | Slowest server held up the rest | Servers started one after another with no bound; now started in parallel under a 90 s ceiling |

A server proved the framing bug by answering a raw JSON line while ignoring ours. After the fix:
**3 servers start in ~4 s and 77 tools are exposed to agents.** Tools reach the model *and* execute
— the proxy registers into the same per-turn `ToolRegistry` that resolves the call.

### Plugin bridge

Plugins are Node subprocesses (`kill_on_drop`). omo's `team_create` is a blocking JSON-RPC into
that subprocess. The reverse channel already exists — `sendNotification` on the TS side, correlated
by `params.requestId` on the Rust side — and is what lets a host observe in-flight progress.

---

## 🧰 Feature overview

- **Interaction modes** — TUI (default), `run`, HTTP server, Web, ACP
- **Sessions** — create, continue, fork, import/export, parent/child links
- **Tool system** — 22 built-in tools plus live MCP tools in one registry
- **Permissions** — per-path scoping; last matching rule wins
- **Providers** — multi-provider with routing, reasoning-effort control, streaming tool calls
- **Extensibility** — TypeScript plugin bridge, MCP stdio + HTTP, LSP
- **Storage** — SQLite (plural schema: `sessions`, `messages`, `parts`)

---

## 📋 CLI commands

```
tui      attach   run      serve    web      acp      models
session  stats     db       config   auth     agent    debug
mcp      export    import   github   pr       upgrade  uninstall
generate version
```

`run` flags:

```
--command  --continue  --session  --fork  --share  --agent  --format
--title    --attach    --dir      --port  --variant  --thinking
--wait-background-timeout <SECONDS>
```

```bash
opr run --help
```

---

## ⚙️ Configuration

Merged from these paths, searched upward:

- `opencode.jsonc` · `opencode.json` · `.opencode/opencode.jsonc` · `.opencode/opencode.json`

Global default: `~/.config/opencode/opencode.json[c]`.

Data location is overridable with `OPENCODE_PATHS_ROOT`; the subworker server uses this to
isolate its database. Resolve it at runtime with:

```bash
opr db path
```

---

## 🏗️ Repository structure

```
crates/
  opencode-cli        CLI entrypoint (binary: opencode) + agent aliases
  opencode-server     HTTP/SSE/WebSocket server, MCP loader
  opencode-session    Session prompt loop, subtasks, status
  opencode-agent      Agent executor (looping + streaming variants)
  opencode-tool       22 built-in tools + registry + permission context
  opencode-provider   Provider adapters (OpenAI, Anthropic, …)
  opencode-mcp        MCP client + stdio/HTTP transports
  opencode-plugin     Plugin host bridge (subprocess)
  opencode-lsp        LSP clients
  opencode-storage    SQLite storage
  opencode-tui        Terminal UI
  opencode-permission opencode-config  opencode-core  opencode-types
  opencode-paths      opencode-command opencode-grep  opencode-proxy
  opencode-util       opencode-watcher
```

---

## 🧹 Operations

| Script | Purpose |
|---|---|
| `install.sh` | Build + install `opr` and agent aliases. `--force` rebuilds |
| `start.sh` / `stop.sh` | Launch / stop the server |
| `clean.sh` | Refuses to clean while an agent runs; flags foreign `opencode` processes in red before killing; sweeps MCP residue left by agent runs |

`clean.sh` exists because MCP servers spawned by our own sessions outlive the run that started
them. It refuses to run while any agent is active, warns before touching a process that is not
ours, and clears the residue.

---

## 🧪 Development

```bash
cargo test --workspace          # 568 tests
cargo build --release --bin opencode
cargo clippy --workspace --all-targets
```

---

## ⚠️ Known limitation

`run` is verified against a real `bash` tool through the real session engine. It has **not** been
exercised end-to-end against a funded provider: free OpenRouter models reject the tool schema and
paid ones return 402 without credits. The loop is provider-agnostic, but we publish what we
measured rather than a result we did not get.

---

## 📄 License

MIT — see [LICENSE](../LICENSE).