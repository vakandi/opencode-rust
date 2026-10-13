# opencode-watcher

`opencode-watcher` provides filesystem watching and event broadcasting.

## Responsibilities

- Watch directory and file changes
- Apply ignore rules and debouncing
- Broadcast standardised change events

## Core types

- `FileWatcher`
- `WatcherConfig`
- `WatcherEvent`
- `FileEvent`
- `WatcherError`

## Default behaviour

- Recursive watching by default
- Default ignores: `.git`, `node_modules`, `target`, temp files
- Default debounce: `100ms`

## Use cases

- File change notifications in sessions
- Context refresh for tooling
- Sidebar state and diagnostics updates

## Validation

```bash
cargo check -p opencode-watcher
```
