# opencoderust

Alias for [`opencode-rust`](https://www.npmjs.com/package/opencode-rust) — the open source
AI coding agent, in Rust.

Both names install the same engine. Pick whichever you type more comfortably:

```bash
npm install -g opencoderust
# or
npm install -g opencode-rust
```

Then:

```bash
opr                  # interactive TUI
opr run "your task"  # one prompt, runs to completion
opr serve --port 3000
```

This package holds no code of its own — it delegates to `opencode-rust`, which is installed as
a dependency. See that package for configuration and requirements.

## License

MIT