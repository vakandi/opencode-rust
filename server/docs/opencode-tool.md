# opencode-tool

`opencode-tool` provides the tool invocation system: tool definition, registry, execution context, and built-in tool implementations.

## Responsibilities

- Define unified `Tool` interface
- Maintain tool registry (`registry`)
- Provide built-in tools (read/write/edit, shell, search, patch, todo, etc.)
- Integrate with permission system, plugin hooks, LSP/MCP

## Built-in modules (selection)

- File: `read`, `write`, `edit`, `multiedit`, `ls`
- Search: `grep_tool`, `glob_tool`, `codesearch`
- Execution: `bash`, `batch`, `apply_patch`
- Task: `plan`, `task`, `todo`, `question`
- Network: `webfetch`, `websearch`
- Support: `registry`, `tool`, `truncation`

## Feature flags

- `lsp` feature: enables `opencode-lsp` and `lsp-types` integration

## Development notes

- New tools should be idempotent where possible and log for observability
- All side-effectful tools should go through the permission system
- Tool output should account for TUI truncation

## Validation

```bash
cargo check -p opencode-tool
cargo check -p opencode-tool --features lsp
```
