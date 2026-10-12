# opencode-storage

`opencode-storage` provides SQLite persistence: database setup and repository access.

## Responsibilities

- Database connection and migrations
- Session, message, and todo repository implementations
- Unified storage error model

## Module structure

- `database.rs` – DB init, connection management
- `schema.rs` – Table definitions and migrations
- `repository.rs` – Session/Message/Todo repositories

## Key exports

- `Database`
- `DatabaseError`
- `SessionRepository`
- `MessageRepository`
- `TodoRepository`

## Development notes

- Schema changes must remain compatible across migrations
- Keep transaction boundaries clear in repository APIs
- Consider indexes and query cost for hot read/write paths

## Validation

```bash
cargo check -p opencode-storage
```
