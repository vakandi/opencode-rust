# opencode-plugin

`opencode-plugin` provides a global hook system and TS plugin subprocess bridge.

## Responsibilities

- Define hook events and context
- Register and trigger hooks (including collecting return values)
- Provide global plugin system instance
- Manage TS plugin subprocesses (JSON-RPC)

## Core types

- `HookEvent`
- `HookContext`
- `HookOutput`
- `Hook`
- `PluginSystem`
- `PluginRegistry`

## Key events (selection)

- `ToolExecuteBefore` / `ToolExecuteAfter`
- `ToolDefinition`
- `ChatSystemTransform` / `ChatMessagesTransform` / `ChatHeaders`
- `CommandExecuteBefore`
- `PermissionAsk`
- `SessionCompacting`

## Subprocess bridge

- Directory: `crates/opencode-plugin/src/subprocess`
- Responsibilities: plugin discovery, subprocess lifecycle, hook forwarding, auth bridge

## Development notes

- When plugins must modify output, callers should use `trigger_collect()` and apply the returned payload
- Hook input/output fields should be separated by event semantics; avoid passing full context
- Global system init should happen once with a single entrypoint

## Validation

```bash
cargo check -p opencode-plugin
```
