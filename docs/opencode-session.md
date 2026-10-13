# opencode-session

`opencode-session` is the session engine core: message flow, state machine, retries, compaction, summarization, and rollback, plus system prompt construction.

## Responsibilities

- Session lifecycle (create, continue, end, interrupt)
- Message organisation (user / assistant / tool / system)
- Coordination with provider, tool, mcp, lsp, plugin
- Session compaction, summary, and snapshots
- Undo/rollback metadata

## Core modules

- `session.rs` – Session entity and manager
- `llm.rs` – Model request/response assembly
- `message.rs` / `message_v2.rs` – Message structures and operations
- `prompt.rs` – Prompt construction
- `compaction.rs` / `summary.rs` – Compaction and summarization
- `revert.rs` / `snapshot.rs` – Rollback and snapshots
- `status.rs` / `todo.rs` – Status and todos

## Key exports (selection)

- `Session`
- `SessionManager`
- `SessionEvent`
- `SessionStatus`
- `SessionSummary`

## Development notes

- Message-order changes must cover streaming and interrupt cases
- Keep plugin hook input/output fields stable
- Prioritise recoverability in rollback and summary logic

## Validation

```bash
cargo check -p opencode-session
```
