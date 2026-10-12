# opencode-permission

`opencode-permission` provides a unified permission decision layer for constraining high-risk tool operations.

## Responsibilities

- Rule set definition and parsing
- Operation arity/granularity classification
- Allow / deny / ask decisions
- Integration with plugin hooks (`PermissionAsk`)

## Module structure

- `ruleset.rs` – Rule structure, matching, parsing
- `arity.rs` – Operation granularity and parameter categories
- `engine.rs` – Permission engine logic

## Use cases

- `opencode-tool` pre-execution permission check
- `opencode-session` in-session approval flow
- TUI/Server permission request and response

## Development notes

- New rules should start from a minimal default policy
- Permission results should be explainable (matched rule, action, target)
- Avoid hardcoding business logic in the rule engine

## Validation

```bash
cargo check -p opencode-permission
```
