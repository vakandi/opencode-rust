# opencode-rust

**The open source AI coding agent, in Rust.** CLI, TUI and HTTP server for local AI agents.

```bash
npm install -g opencode-rust
```

Then run `opr`:

```bash
opr                  # interactive TUI
opr run "your task"  # one prompt, runs to completion
opr serve --port 3000
```

`opr` with no subcommand opens the TUI. `opr --some-flag` also opens the TUI — only a
known subcommand is forwarded verbatim.

## Prebuilt binaries

Prebuilt binaries ship as platform-scoped optional dependencies. `npm install` pulls only
the one matching your machine:

- `@opencode-rust/darwin-arm64` · `@opencode-rust/darwin-x64`
- `@opencode-rust/linux-x64-gnu` · `@opencode-rust/linux-arm64-gnu` · `@opencode-rust/linux-x64-musl`
- `@opencode-rust/win32-x64-msvc`

When no binary matches your platform the installer prints the alternatives and exits
cleanly — it never compiles during `npm install`, because that would force a Rust
toolchain on every user and trip security scanners.

## Build from source

```bash
git clone https://github.com/vakandi/opencode-rust
cd opencode-rust
./install.sh
```

## Homebrew

```bash
brew install vakandi/tap/opr
```

## Requirements

Node 18+. The engine spawns Node subprocesses for its plugin bridge and launches MCP
servers through `npx`, so Node is a runtime dependency regardless of how it was installed.

## Configuration

Merged from `opencode.jsonc` · `opencode.json` · `.opencode/opencode.jsonc` ·
`.opencode/opencode.json`, searched upward. Global default:
`~/.config/opencode/opencode.json[c]`.

Set a provider key before the first run:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

## License

MIT
