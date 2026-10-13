# opencode-lsp

`opencode-lsp` provides Language Server Protocol client support for code intelligence and editing assistance.

## Responsibilities

- Start and manage LSP subprocesses
- Handle JSON-RPC requests, responses, and notifications
- Maintain client state and event stream
- Manage multi-language server registry

## Key types

- `LspClient`
- `LspClientRegistry`
- `LspServerConfig`
- `LspEvent`
- `JsonRpcRequest` / `JsonRpcResponse` / `JsonRpcNotification`
- `LspError`

## Use cases

- `opencode-tool` LSP tools (requires `lsp` feature)
- TUI / Server LSP status and debugging

## Development notes

- Handle timeouts, retries, and uninitialized server
- Keep URI/path conversion consistent
- Consider backpressure and capacity for event broadcast channels

## Validation

```bash
cargo check -p opencode-lsp
```
