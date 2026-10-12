# opencode-command

`opencode-command` implements the slash command system, supporting built-in commands, file-based commands, MCP commands, and skill commands.

## Responsibilities

- Maintain command registry (`CommandRegistry`)
- Unify command metadata (`Command` / `CommandSource`)
- Handle template variable substitution and command execution context injection
- Load commands dynamically from `.opencode/commands/*.md`

## Core types

- `Command`
- `CommandSource`
- `CommandContext`
- `CommandRegistry`

## Built-in commands

Current built-in templates include:

- `init`
- `review`
- `commit`
- `test`

## Usage

- CLI / TUI slash command entrypoint
- Server command execution endpoints
- Plugin hooks (e.g. `command.execute.before`)

## Source entrypoint

- `crates/opencode-command/src/lib.rs`

## Validation

```bash
cargo check -p opencode-command
```
