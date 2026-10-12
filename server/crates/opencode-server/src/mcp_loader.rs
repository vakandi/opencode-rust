use std::sync::{Arc, OnceLock};

static MCP_REGISTRY: OnceLock<Arc<opencode_mcp::McpClientRegistry>> = OnceLock::new();
static MCP_TOOLS: OnceLock<Vec<opencode_provider::ToolDefinition>> = OnceLock::new();

/// Tool definitions from the MCP servers declared in the config, started once per process.
pub fn configured_tools() -> Vec<opencode_provider::ToolDefinition> {
    MCP_TOOLS.get().cloned().unwrap_or_default()
}

/// Start every MCP server declared in the config, once, at boot.
/// Hand the live MCP registry to the tool crate, which rebuilds its registry every turn.
pub fn install_into_tool_registry() {
    opencode_tool::registry::set_mcp_source(Arc::new(
        |tool_registry: &opencode_tool::ToolRegistry| {
            Box::pin(async move {
                for tool in crate::mcp_proxy::proxy_tools().await {
                    tool_registry.register(tool).await;
                }
            })
        },
    ));
}

pub async fn boot(cwd: std::path::PathBuf) {
    let tools = start_configured_mcp(cwd).await;
    let _ = MCP_TOOLS.set(tools);
}

/// The live MCP registry, for wiring proxy tools into a `ToolRegistry`.
pub fn registry() -> Option<Arc<opencode_mcp::McpClientRegistry>> {
    MCP_REGISTRY.get().cloned()
}
pub async fn start_configured_mcp(
    cwd: std::path::PathBuf,
) -> Vec<opencode_provider::ToolDefinition> {
    let Ok(config) = opencode_config::load_config(&cwd) else {
        tracing::warn!("MCP: config load failed");
        return Vec::new();
    };
    let declared = config.mcp.clone().unwrap_or_default();
    tracing::info!(count = declared.len(), "MCP: servers declared in config");
    if declared.is_empty() {
        return Vec::new();
    }

    let registry = Arc::new(opencode_mcp::McpClientRegistry::new());
    // `npx` resolving a package on first run takes tens of seconds. Without a bound the boot
    // awaited it forever; started one after another the slowest server held up the rest.
    const MCP_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

    let mut pending: Vec<(String, opencode_mcp::McpServerConfig)> = Vec::new();
    for (name, entry) in &declared {
        let opencode_config::McpServerConfig::Full(server) = entry else {
            tracing::info!(mcp = %name, "MCP: not a Full variant, skipped");
            continue;
        };
        if server.enabled == Some(false) {
            continue;
        }
        let Some(command) = server.command.first().cloned() else {
            tracing::info!(mcp = %name, "MCP: no command, skipped");
            continue;
        };
        let mut args: Vec<String> = server.command.iter().skip(1).cloned().collect();
        args.extend(server.args.iter().cloned());

        pending.push((
            name.clone(),
            opencode_mcp::McpServerConfig {
                name: name.clone(),
                command,
                args,
                env: server
                    .environment
                    .as_ref()
                    .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
                timeout_ms: server.timeout,
            },
        ));
    }

    let starts = pending.into_iter().map(|(name, cfg)| {
        let registry = registry.clone();
        async move {
            let outcome = tokio::time::timeout(MCP_START_TIMEOUT, registry.add_stdio(cfg)).await;
            match outcome {
                Ok(Ok(_)) => tracing::info!(mcp = %name, "MCP server started"),
                Ok(Err(e)) => tracing::warn!(mcp = %name, error = %e, "MCP server failed"),
                Err(_) => tracing::warn!(mcp = %name, "MCP server start timed out"),
            }
        }
    });
    futures::future::join_all(starts).await;

    let tools: Vec<opencode_provider::ToolDefinition> = registry
        .tool_registry()
        .list()
        .await
        .into_iter()
        .map(|tool| opencode_provider::ToolDefinition {
            name: tool.full_name,
            description: tool.description,
            parameters: tool.input_schema,
        })
        .collect();

    let _ = MCP_REGISTRY.set(registry);
    install_into_tool_registry();
    tracing::info!(count = tools.len(), "MCP: tools exposed to agents");
    tools
}
