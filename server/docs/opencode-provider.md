# opencode-provider

`opencode-provider` is the multi-provider model adapter layer: request building, streaming response parsing, retries, and model capability queries.

## Responsibilities

- Unified provider abstraction
- Integrate multiple vendor APIs (OpenAI, Anthropic, Google, xAI, Groq, etc.)
- Model registration, context window, capability metadata
- Retry and streaming event handling

## Key modules

- `provider.rs` – Provider trait and unified call entry
- `bootstrap.rs` – Build registry from config/env
- `models.rs` – Model metadata and capability queries
- `stream.rs` – Streaming event abstraction
- `retry.rs` – Retry policy and retryability
- `<vendor>.rs` – Per-vendor implementations

## Key exports

- `create_registry_from_bootstrap_config`
- `create_registry_from_env`
- `with_retry` / `with_retry_and_hook`
- `get_model_context_limit`

## Relations to other modules

- Used by `opencode-session`, `opencode-agent`, `opencode-cli`
- Integrates with `opencode-plugin` hooks (e.g. request params/headers modification)

## Validation

```bash
cargo check -p opencode-provider
```
