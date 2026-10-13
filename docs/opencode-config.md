# opencode-config

`opencode-config` handles config file discovery, loading, parsing, and merging. It is the configuration entrypoint for runtime behaviour.

## Responsibilities

- Search for config files (project and global)
- Parse JSON/JSONC (comments supported)
- Map config to strongly typed structures
- Provide well-known paths and defaults

## Module structure

- `loader.rs` – Config loading, path lookup, merge flow
- `schema.rs` – Config structure definitions
- `wellknown.rs` – Common directory/file path constants

## Config paths (common)

- Project: `opencode.jsonc` / `opencode.json`
- Project extension: `.opencode/opencode.jsonc` / `.opencode/opencode.json`
- Global: `~/.config/opencode/opencode.jsonc` (or `.json`)

## Usage notes

- When adding new config fields, define default behaviour
- Merge behaviour should remain predictable
- Changes to provider/mcp/agent fields should be reflected in docs

## Validation

```bash
cargo check -p opencode-config
```
