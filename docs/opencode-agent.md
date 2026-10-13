# opencode-agent

`opencode-agent` handles agent definition, registration, execution orchestration, and message handling.

## Responsibilities

- Maintain agent metadata and capability information
- Drive agent execution flow (calling provider / tool / permission)
- Build and transform agent-side message structures

## Module structure

- `agent.rs` – Agent definition and registration
- `executor.rs` – Executor and flow orchestration
- `message.rs` – Message structures and conversion

## Dependencies

- Downstream: `opencode-provider`, `opencode-tool`, `opencode-permission`, `opencode-plugin`
- Upstream consumers: `opencode-session`, `opencode-server`, `opencode-cli`

## Development notes

- Define clear mode boundaries (responsibilities, tool scope, prompts) before adding new agents
- Executor changes should consider streaming output and interrupt behaviour

## Validation

```bash
cargo check -p opencode-agent
```
