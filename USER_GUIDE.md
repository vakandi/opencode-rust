# USER GUIDE - RustingOpenCode (ROCode)

This guide is for day-to-day users. It covers startup, common commands, configuration, and troubleshooting.  
The project is named `RustingOpenCode` (short: `ROCode`). The CLI command is still `opencode`.

**Use this project's opencode.** If you have another OpenCode (e.g. npm/global) on your PATH, the shell may run that instead. From the **RustingOpenCode repo root** use one of:

- **`./target/debug/opencode`** — after `cargo build -p opencode-cli`
- **`cargo run -p opencode-cli --`** — always uses this repo's version

Example: `./target/debug/opencode tui` or `cargo run -p opencode-cli -- tui`.  
In the commands below, `opencode` means this local binary (run from repo root).

## 1. Quick start

From the RustingOpenCode repo root:

```bash
cargo run -p opencode-cli -- --help
```

Start TUI by default:

```bash
cargo run -p opencode-cli --
```

Same as:

```bash
cargo run -p opencode-cli -- tui
```

Single non-interactive run:

```bash
cargo run -p opencode-cli -- run "Summarise the current risks in this repo"
```

Start HTTP server:

```bash
cargo run -p opencode-cli -- serve --port 3000 --hostname 127.0.0.1
```

## 2. Common commands

Run these from the **RustingOpenCode repo root**; use `./target/debug/opencode` or `cargo run -p opencode-cli --`.

### 2.1 Session management

```bash
./target/debug/opencode session list
./target/debug/opencode session list --format json
./target/debug/opencode session show <SESSION_ID>
./target/debug/opencode session delete <SESSION_ID>
```

### 2.2 Models and config

```bash
./target/debug/opencode models
./target/debug/opencode models --refresh --verbose
./target/debug/opencode config
```

### 2.3 Auth management

```bash
./target/debug/opencode auth list
./target/debug/opencode auth login --help
./target/debug/opencode auth logout --help
```

Note: For `auth login` / `auth logout` options, use the `--help` output.

### 2.4 MCP management

```bash
./target/debug/opencode mcp list
./target/debug/opencode mcp connect <NAME>
./target/debug/opencode mcp disconnect <NAME>
./target/debug/opencode mcp add --help
./target/debug/opencode mcp auth --help
```

If the local server is not at the default address:

```bash
./target/debug/opencode mcp --server http://127.0.0.1:3000 list
```

### 2.5 Debug commands

```bash
./target/debug/opencode debug paths
./target/debug/opencode debug config
./target/debug/opencode debug skill
./target/debug/opencode debug agent
```

## 3. TUI and Run common options

Show full options:

```bash
./target/debug/opencode tui --help
./target/debug/opencode run --help
```

Frequently used (both TUI and run):

- `-m, --model <MODEL>` – Set model
- `-c, --continue` – Continue latest session
- `-s, --session <SESSION>` – Continue specific session
- `--fork` – Fork session
- `--agent <AGENT>` – Set agent (default: `build`)
- `--port <PORT>` / `--hostname <HOSTNAME>` – Server address

Additional common options for `run`:

- `--format default|json`
- `-f, --file <FILE>`
- `--thinking`

## 4. Config file locations

The program merges config from multiple locations by priority (searching upward):

- `opencode.jsonc`
- `opencode.json`
- `.opencode/opencode.jsonc`
- `.opencode/opencode.json`

Global config default:

- `~/.config/opencode/opencode.jsonc` (or `.json`)

Recommendation: Start with a minimal project-level config, then add provider/mcp/agent/lsp as needed.

## 5. Using Claude (Anthropic) and testing a coding session

Claude is supported via the **anthropic** provider. You can run interactive coding in the TUI or a one-off task with `run`.

### 5.1 Set your API key

**Option A – Environment variable (easiest):**

```bash
export ANTHROPIC_API_KEY="sk-ant-your-key-here"
```

Get a key at [Anthropic Console](https://console.anthropic.com/).

**Option B – Config file:**

Create or edit `opencode.jsonc` in your project (or `~/.config/opencode/opencode.jsonc`):

```jsonc
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "provider": {
    "anthropic": {
      "api_key": "sk-ant-your-key-here"
    }
  }
}
```

Do not commit real API keys; use env vars or a local config that’s in `.gitignore`.

### 5.2 Check that the provider is available

```bash
./target/debug/opencode auth list
./target/debug/opencode models --refresh
```

You should see `anthropic` and Claude models in the list.

### 5.3 Run a coding session

**Interactive TUI (recommended for coding):**

```bash
./target/debug/opencode tui -m anthropic/claude-sonnet-4-20250514
```

Or with default model from config:

```bash
./target/debug/opencode tui
```

In the TUI you can type prompts, use slash commands, and let the agent edit files and run tools.

**Single task (no TUI):**

```bash
./target/debug/opencode run "Add a unit test for the login function in src/auth.rs"
```

With a specific model:

```bash
./target/debug/opencode run -m anthropic/claude-sonnet-4-20250514 "Review this repo for security issues"
```

### 5.4 Useful model IDs (Anthropic)

- `anthropic/claude-sonnet-4-20250514` – Claude Sonnet 4
- `anthropic/claude-3-5-sonnet-20241022` – Claude 3.5 Sonnet
- `anthropic/claude-3-opus-20240229` – Claude 3 Opus

Use `./target/debug/opencode models` to see the full list for your setup.

## 6. Recommended workflows

### 6.1 Local interactive use

1. `cargo run -p opencode-cli --` or `./target/debug/opencode tui`
2. Run tasks in the TUI
3. Use `./target/debug/opencode session list/show` to review history

### 6.2 Scripts or integration

1. `./target/debug/opencode serve --port 3000`
2. Use `./target/debug/opencode run ... --format json` or the server API for integration
3. Use `./target/debug/opencode stats` to track token/cost

## 7. Troubleshooting

### 7.1 Port in use

- Use another port: `./target/debug/opencode serve --port 3001`

### 7.2 Model not available

1. `./target/debug/opencode auth list`
2. `./target/debug/opencode models --refresh`
3. `./target/debug/opencode config` to check that provider config is applied

### 7.3 Config issues

1. `./target/debug/opencode debug paths` – see config search paths
2. `./target/debug/opencode debug config` – see final merged config

### 7.4 MCP connection problems

1. `./target/debug/opencode mcp list`
2. `./target/debug/opencode mcp debug <NAME>`
3. `./target/debug/opencode mcp connect <NAME>`

## 8. Documentation index

- Project overview: `README.md`
- Docs index: `docs/README.md`
- CLI: `docs/opencode-cli.md`
- TUI: `docs/opencode-tui.md`
- Config: `docs/opencode-config.md`
