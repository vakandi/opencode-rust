# opencode-grep

`opencode-grep` wraps text/file search for reuse by the tool and server layers.

## Responsibilities

- File traversal and filtering
- Regex and keyword matching
- Structured match output
- Search statistics aggregation

## Key exports

- `Ripgrep`
- `FileSearchOptions`
- `MatchResult`
- `SubMatch`
- `Stats`

## Use cases

- `opencode-tool` grep/codesearch tools
- Server `/find/*` routes
- Quick lookup in diagnostics and debugging

## Development notes

- Apply ignore filters early for large directory scans
- Result shape should work for both TUI and JSON output
- Keep errors traceable (path, pattern, line number)

## Validation

```bash
cargo check -p opencode-grep
```
