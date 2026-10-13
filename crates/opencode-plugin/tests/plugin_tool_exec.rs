//! Proves a plugin-contributed tool can be CAPTURED and INVOKED over the
//! `tool.execute` RPC, without depending on an LLM to emit a tool call.

use std::sync::Arc;

use opencode_plugin::plugin_tools;
use opencode_plugin::subprocess::client::{PluginContext, PluginSubprocess};
use opencode_plugin::subprocess::loader::PluginLoader;
use opencode_plugin::subprocess::runtime::detect_runtime;

#[tokio::test]
async fn plugin_tool_is_captured_and_executed() {
    let Some(runtime) = detect_runtime() else {
        eprintln!("skipping: no bun/deno/node on PATH");
        return;
    };

    // PluginLoader::new() materialises the embedded host script into the cache dir.
    let _loader = PluginLoader::new().expect("loader");
    let host = dirs::cache_dir()
        .expect("cache dir")
        .join("opencode")
        .join("plugin-host.ts");
    assert!(host.exists(), "host script not materialised at {host:?}");

    let dir = std::env::temp_dir().join(format!("probe-{}", uuid_like()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let plugin = dir.join("probe-plugin.ts");
    std::fs::write(
        &plugin,
        r#"
export default async () => ({
  tool: {
    probe_echo: {
      description: "echo a value back",
      async execute(args) {
        return "ECHO:" + (args && args.value);
      },
    },
  },
});
"#,
    )
    .expect("write plugin");

    let ctx = PluginContext {
        worktree: dir.to_string_lossy().to_string(),
        directory: dir.to_string_lossy().to_string(),
        server_url: "http://127.0.0.1:0".to_string(),
    };

    let client = PluginSubprocess::spawn(
        runtime,
        host.to_str().expect("host path"),
        plugin.to_str().expect("plugin path"),
        ctx,
        None,
    )
    .await
    .expect("spawn plugin subprocess");

    // CAPTURE: the tool definition crossed initialize.
    let tools = client.tools();
    assert!(
        tools.contains_key("probe_echo"),
        "probe_echo missing from captured tools: {:?}",
        tools.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        tools["probe_echo"]["description"],
        serde_json::json!("echo a value back")
    );

    // REGISTER: it lands in the global store and then in a fresh default registry.
    let client = Arc::new(client);
    plugin_tools::register(Arc::clone(&client)).await;
    let listed = plugin_tools::list().await;
    assert!(
        listed.iter().any(|t| t.name == "probe_echo"),
        "probe_echo missing from plugin_tools store"
    );

    // INVOKE: the round-trip that has never been observed end-to-end.
    let out = client
        .tool_execute("probe_echo", serde_json::json!({ "value": "hello" }), serde_json::json!({}))
        .await
        .expect("tool.execute RPC");
    assert_eq!(out["output"], serde_json::json!("ECHO:hello"), "raw: {out:?}");

    let _ = client.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

fn uuid_like() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos()
}
