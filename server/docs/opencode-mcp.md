# opencode-mcp

`opencode-mcp` implements the MCP (Model Context Protocol) client stack, with support for multiple transports and OAuth.

## Responsibilities

- MCP client connection and session maintenance
- Tool sync and registration
- OAuth auth state management
- SSE/HTTP/stdio transport abstraction

## Module structure

- `client.rs` – Client and registry
- `tool.rs` – MCP tool wrapper
- `oauth.rs` / `auth.rs` – OAuth and auth flow
- `transport.rs` – Transport layer (HTTP/SSE/Stdio)
- `protocol.rs` – JSON-RPC protocol structures

## Key exports

- `McpClient` / `McpClientRegistry`
- `McpToolRegistry`
- `McpOAuthManager` / `OAuthRegistry`
- `MCP_TOOLS_CHANGED_EVENT`

## Use cases

- CLI `opencode mcp ...`
- Server `/mcp/*` routes
- Tool/session dynamic tool extension

## Validation

```bash
cargo check -p opencode-mcp
```
