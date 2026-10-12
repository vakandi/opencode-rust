# opencode-server

`opencode-server` provides HTTP/SSE/WebSocket APIs and bridges CLI, TUI, and external systems.

## Responsibilities

- Expose unified API routes
- Manage sessions, config, providers, MCP, permissions, files, and search
- Provide event stream and TUI control endpoints
- Handle OAuth callbacks, PTY, and workspace operations

## Route groups (selection)

As in `crates/opencode-server/src/routes.rs`:

- Base: `/health`, `/event`, `/path`, `/vcs`
- Session: `/session/*`
- Provider: `/provider/*`
- Config: `/config/*`
- MCP: `/mcp/*`
- File: `/file/*`
- Search: `/find/*`
- Permission: `/permission/*`
- Project: `/project/*`
- PTY: `/pty/*`
- TUI control: `/tui/*`
- Experimental: `/experimental/*`
- Plugin auth: `/plugin/*`

## Module structure

- `server.rs` – Server startup and lifecycle
- `routes.rs` – Route definitions and handlers
- `oauth.rs` / `mcp_oauth.rs` – OAuth flow
- `pty.rs` – Terminal session bridge
- `worktree.rs` – Workspace operations

## Development notes

- Define input/output models before adding routes and handlers
- Avoid blocking (I/O, DB, network) on high-concurrency paths
- Keep CLI/TUI call sites in sync when changing APIs

## Validation

```bash
cargo check -p opencode-server
```
