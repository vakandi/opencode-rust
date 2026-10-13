# opencode-core

`opencode-core` is the workspace’s lowest-level shared library, providing a global event bus and ID generation.

## Responsibilities

- Provide async event bus (`bus`)
- Provide unified ID utilities (`id`)
- Act as a light dependency base for upper crates

## Module structure

- `bus.rs` – Event publish/subscribe infrastructure
- `id.rs` – ID generation, parsing, formatting
- `lib.rs` – Re-exports

## Dependencies

- Upstream: no business logic dependencies
- Downstream: most business crates (session, tool, provider, server, etc.)

## Development notes

- Avoid putting business logic in core
- New capabilities should be side-effect free and loosely coupled
- Consider full-workspace impact for any change

## Validation

```bash
cargo check -p opencode-core
```
