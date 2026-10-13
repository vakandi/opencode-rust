# opencode-util

`opencode-util` provides shared utilities and basic capabilities used across modules.

## Responsibilities

- Filesystem helpers (`filesystem`)
- Logging setup and structured logging (`logging`)
- General utilities (`util`)

## Module structure

- `filesystem.rs` – File I/O, path helpers
- `logging.rs` – Tracing init, log level, output
- `util.rs` – Token/timeout/git/lock/wildcard utilities

## Usage notes

- Prefer reusing util instead of reimplementing
- Do not put business-coupled helpers in this crate
- Logging init should typically be done once from CLI

## Validation

```bash
cargo check -p opencode-util
```
