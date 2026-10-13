# opr

Short alias for [`opencode-rust`](https://www.npmjs.com/package/opencode-rust) — the open
source AI coding agent, in Rust.

```bash
npm install -g opr
```

Same engine, shorter command. Both names work identically from here:

```bash
opr                  # interactive TUI
opr run "your task"  # one prompt, runs to completion
opr serve --port 3000
```

This package holds no code of its own — it depends on `opencode-rust`, which is installed
alongside it. See that package for configuration, supported platforms, and requirements.

> There is no `opencoderust` package. npm rejects it as too similar to `opencode-rust`, and
> `npm install -g opencoderust` will not work. Use `opr` or `opencode-rust`.

## License

MIT
