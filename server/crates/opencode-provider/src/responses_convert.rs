use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::message::{Content, ContentPart, Message, Role};
use crate::responses::{
    CallWarning, LocalShellAction, ResponsesInput, ResponsesReasoning, SystemMessageMode,
};

pub async fn convert_to_openai_responses_input(
    prompt: &[Message],
    system_message_mode: SystemMessageMode,
    file_id_prefixes: Option<&[String]>,
    store: bool,
    has_local_shell_tool: bool,
) -> (ResponsesInput, Vec<CallWarning>) {
    let mut input: ResponsesInput = Vec::new();
    let mut warnings = Vec::new();

    for message in prompt {
        match message.role {
            Role::System => {
                let Some(text) = message_text(message) else {
                    continue;
                };

                match system_message_mode {
                    SystemMessageMode::System => {
                        input.push(json!({ "role": "system", "content": text }));
                    }
                    SystemMessageMode::Developer => {
                        input.push(json!({ "role": "developer", "content": text }));
                    }
                    SystemMessageMode::Remove => {
                        warnings.push(CallWarning::UnsupportedSetting {
                            setting: "systemMessages".to_string(),
                            details: Some(
                                "system messages are not supported for this model and were removed"
                                    .to_string(),
                            ),
                        });
                    }
                }
            }
            Role::User => {
                let mut content = Vec::new();
                for part in message_parts(message) {
                    match part.content_type.as_str() {
                        "text" => {
                            if let Some(text) = &part.text {
                                if !text.is_empty() {
                                    content.push(json!({ "type": "input_text", "text": text }));
                                }
                            }
                        }
                        "image" | "image_url" => {
                            if let Some(url) = part.image_url.as_ref().map(|img| img.url.as_str()) {
                                if is_file_id(url, file_id_prefixes) {
                                    content.push(json!({
                                        "type": "input_image",
                                        "file_id": url,
                                    }));
                                } else {
                                    content.push(json!({
                                        "type": "input_image",
                                        "image_url": url,
                                    }));
                                }
                            }
                        }
                        "file" => {
                            let Some(url) = part.image_url.as_ref().map(|img| img.url.as_str())
                            else {
                                continue;
                            };
                            let mut item = json!({ "type": "input_file" });
                            if is_file_id(url, file_id_prefixes) {
                                item["file_id"] = Value::String(url.to_string());
                            } else if url.starts_with("data:") {
                                item["file_data"] = Value::String(url.to_string());
                            } else {
                                item["file_url"] = Value::String(url.to_string());
                            }
                            if let Some(filename) = &part.filename {
                                item["filename"] = Value::String(filename.clone());
                            }
                            content.push(item);
                        }
                        _ => {
                            if let Some(text) = &part.text {
                                if !text.is_empty() {
                                    content.push(json!({ "type": "input_text", "text": text }));
                                }
                            }
                        }
                    }
                }

                if !content.is_empty() {
                    input.push(json!({ "role": "user", "content": content }));
                }
            }
            Role::Assistant => {
                let mut assistant_content = Vec::new();
                let mut reasoning_by_id: HashMap<String, ResponsesReasoning> = HashMap::new();
                let mut reasoning_order = Vec::new();
                let mut item_refs_seen: HashSet<String> = HashSet::new();

                for part in message_parts(message) {
                    match part.content_type.as_str() {
                        "text" => {
                            if let Some(text) = &part.text {
                                if !text.is_empty() {
                                    assistant_content.push(json!({
                                        "type": "output_text",
                                        "text": text,
                                    }));
                                }
                            }
                        }
                        "tool_use" => {
                            let Some(tool_use) = &part.tool_use else {
                                continue;
                            };

                            let part_opts = merged_options(message, &part);
                            let item_id = option_string(&part_opts, &["itemId", "item_id", "id"]);

                            if has_local_shell_tool && is_local_shell_tool_name(&tool_use.name) {
                                let action = local_shell_action_from_input(&tool_use.input);
                                input.push(json!({
                                    "type": "local_shell_call",
                                    "id": item_id,
                                    "call_id": tool_use.id,
                                    "action": action,
                                }));
                            } else {
                                input.push(json!({
                                    "type": "function_call",
                                    "id": item_id,
                                    "call_id": tool_use.id,
                                    "name": tool_use.name,
                                    "arguments": serde_json::to_string(&tool_use.input)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                }));
                            }
                        }
                        "tool_result" => {
                            let Some(tool_result) = &part.tool_result else {
                                continue;
                            };
                            let part_opts = merged_options(message, &part);
                            let item_id = option_string(&part_opts, &["itemId", "item_id", "id"]);

                            if store {
                                if let Some(item_id) = item_id {
                                    if item_refs_seen.insert(item_id.clone()) {
                                        input.push(json!({
                                            "type": "item_reference",
                                            "id": item_id,
                                        }));
                                    }
                                    continue;
                                }
                            }

                            let tool_name =
                                option_string(&part_opts, &["toolName", "tool_name", "name"]);
                            if tool_name
                                .as_deref()
                                .map(is_local_shell_tool_name)
                                .unwrap_or(false)
                            {
                                input.push(json!({
                                    "type": "local_shell_call_output",
                                    "call_id": tool_result.tool_use_id,
                                    "output": tool_result.content,
                                }));
                            } else {
                                input.push(json!({
                                    "type": "function_call_output",
                                    "call_id": tool_result.tool_use_id,
                                    "output": tool_result.content,
                                }));
                            }
                        }
                        "reasoning" => {
                            let text = part.text.clone().unwrap_or_default();
                            let part_opts = merged_options(message, &part);
                            let item_id = option_string(&part_opts, &["itemId", "item_id", "id"])
                                .unwrap_or_else(|| {
                                    format!("reasoning_{}", reasoning_by_id.len() + 1)
                                });

                            if store
                                && option_string(&part_opts, &["itemId", "item_id", "id"]).is_some()
                            {
                                if item_refs_seen.insert(item_id.clone()) {
                                    input.push(json!({
                                        "type": "item_reference",
                                        "id": item_id,
                                    }));
                                }
                                continue;
                            }

                            let encrypted = option_string(
                                &part_opts,
                                &["encryptedContent", "encrypted_content"],
                            );

                            let entry =
                                reasoning_by_id.entry(item_id.clone()).or_insert_with(|| {
                                    reasoning_order.push(item_id.clone());
                                    ResponsesReasoning {
                                        item_type: "reasoning".to_string(),
                                        id: item_id.clone(),
                                        encrypted_content: encrypted.clone(),
                                        summary: Vec::new(),
                                    }
                                });

                            if entry.encrypted_content.is_none() {
                                entry.encrypted_content = encrypted;
                            }

                            if !text.is_empty() {
                                entry.summary.push(crate::responses::ReasoningSummaryText {
                                    text_type: "summary_text".to_string(),
                                    text,
                                });
                            }
                        }
                        _ => {
                            if let Some(text) = &part.text {
                                if !text.is_empty() {
                                    assistant_content.push(json!({
                                        "type": "output_text",
                                        "text": text,
                                    }));
                                }
                            }
                        }
                    }
                }

                if !assistant_content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": assistant_content,
                    }));
                }

                for id in reasoning_order {
                    if let Some(reasoning) = reasoning_by_id.remove(&id) {
                        input.push(json!(reasoning));
                    }
                }
            }
            Role::Tool => {
                for part in message_parts(message) {
                    let Some(tool_result) = &part.tool_result else {
                        continue;
                    };

                    let part_opts = merged_options(message, &part);
                    let tool_name = option_string(&part_opts, &["toolName", "tool_name", "name"]);

                    if tool_name
                        .as_deref()
                        .map(is_local_shell_tool_name)
                        .unwrap_or(false)
                    {
                        input.push(json!({
                            "type": "local_shell_call_output",
                            "call_id": tool_result.tool_use_id,
                            "output": tool_result.content,
                        }));
                    } else {
                        input.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_result.tool_use_id,
                            "output": tool_result.content,
                        }));
                    }
                }
            }
        }
    }

    (input, warnings)
}

fn message_text(message: &Message) -> Option<String> {
    match &message.content {
        Content::Text(text) if !text.trim().is_empty() => Some(text.clone()),
        Content::Parts(parts) => {
            let combined = parts
                .iter()
                .filter_map(|part| part.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n");
            if combined.trim().is_empty() {
                None
            } else {
                Some(combined)
            }
        }
        _ => None,
    }
}

fn message_parts(message: &Message) -> Vec<ContentPart> {
    match &message.content {
        Content::Text(text) => vec![ContentPart {
            content_type: "text".to_string(),
            text: Some(text.clone()),
            ..Default::default()
        }],
        Content::Parts(parts) => parts.clone(),
    }
}

fn merged_options(message: &Message, part: &ContentPart) -> HashMap<String, Value> {
    let mut merged = HashMap::new();

    if let Some(opts) = &message.provider_options {
        merged.extend(opts.clone());
    }
    if let Some(opts) = &part.provider_options {
        merged.extend(opts.clone());
    }

    merged
}

fn option_string(options: &HashMap<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = options.get(*key) {
            if let Some(s) = value_as_string(value) {
                return Some(s);
            }
        }
    }

    // A few providers nest these fields under openai/provider buckets.
    for nested_key in ["openai", "provider", "metadata"] {
        let Some(Value::Object(map)) = options.get(nested_key) else {
            continue;
        };
        for key in keys {
            if let Some(value) = map.get(*key) {
                if let Some(s) = value_as_string(value) {
                    return Some(s);
                }
            }
        }
    }

    None
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn is_file_id(value: &str, file_id_prefixes: Option<&[String]>) -> bool {
    let Some(prefixes) = file_id_prefixes else {
        return false;
    };

    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

fn is_local_shell_tool_name(name: &str) -> bool {
    matches!(
        name,
        "local_shell" | "openai.local_shell" | "local-shell" | "shell_exec"
    )
}

fn local_shell_action_from_input(input: &Value) -> LocalShellAction {
    if let Some(action) = input.get("action") {
        if let Ok(parsed) = serde_json::from_value::<LocalShellAction>(action.clone()) {
            return parsed;
        }
    }

    if let Ok(parsed) = serde_json::from_value::<LocalShellAction>(input.clone()) {
        return parsed;
    }

    let command = input
        .get("command")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    LocalShellAction {
        action_type: "exec".to_string(),
        command,
        timeout_ms: None,
        user: None,
        working_directory: None,
        env: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Content, ImageUrl, Message, Role, ToolResult, ToolUse};

    fn text_message(role: Role, text: &str) -> Message {
        Message {
            role,
            content: Content::Text(text.to_string()),
            cache_control: None,
            provider_options: None,
        }
    }

    #[tokio::test]
    async fn test_convert_system_message_modes() {
        let prompt = vec![text_message(Role::System, "Follow rules")];

        let (sys_input, sys_warn) = convert_to_openai_responses_input(
            &prompt,
            SystemMessageMode::System,
            None,
            false,
            false,
        )
        .await;
        assert!(sys_warn.is_empty());
        assert_eq!(sys_input[0]["role"], "system");

        let (dev_input, dev_warn) = convert_to_openai_responses_input(
            &prompt,
            SystemMessageMode::Developer,
            None,
            false,
            false,
        )
        .await;
        assert!(dev_warn.is_empty());
        assert_eq!(dev_input[0]["role"], "developer");

        let (removed_input, removed_warn) = convert_to_openai_responses_input(
            &prompt,
            SystemMessageMode::Remove,
            None,
            false,
            false,
        )
        .await;
        assert!(removed_input.is_empty());
        assert_eq!(removed_warn.len(), 1);
    }

    #[tokio::test]
    async fn test_convert_user_image_base64() {
        let prompt = vec![Message {
            role: Role::User,
            content: Content::Parts(vec![ContentPart {
                content_type: "image_url".to_string(),
                image_url: Some(ImageUrl {
                    url: "data:image/png;base64,SGVsbG8=".to_string(),
                }),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        }];

        let (input, warnings) = convert_to_openai_responses_input(
            &prompt,
            SystemMessageMode::System,
            None,
            false,
            false,
        )
        .await;

        assert!(warnings.is_empty());
        let content = input[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "input_image");
        assert_eq!(content[0]["image_url"], "data:image/png;base64,SGVsbG8=");
    }

    #[tokio::test]
    async fn test_convert_assistant_reasoning_dedup() {
        let mut opts = HashMap::new();
        opts.insert("itemId".to_string(), Value::String("rs_1".to_string()));
        opts.insert(
            "encryptedContent".to_string(),
            Value::String("encrypted".to_string()),
        );

        let prompt = vec![Message {
            role: Role::Assistant,
            content: Content::Parts(vec![
                ContentPart {
                    content_type: "reasoning".to_string(),
                    text: Some("first".to_string()),
                    provider_options: Some(opts.clone()),
                    ..Default::default()
                },
                ContentPart {
                    content_type: "reasoning".to_string(),
                    text: Some("second".to_string()),
                    provider_options: Some(opts),
                    ..Default::default()
                },
            ]),
            cache_control: None,
            provider_options: None,
        }];

        let (input, warnings) = convert_to_openai_responses_input(
            &prompt,
            SystemMessageMode::System,
            None,
            false,
            false,
        )
        .await;

        assert!(warnings.is_empty());
        let reasoning_items: Vec<_> = input
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("reasoning"))
            .collect();

        assert_eq!(reasoning_items.len(), 1);
        assert_eq!(reasoning_items[0]["id"], "rs_1");
        let summary = reasoning_items[0]["summary"].as_array().unwrap();
        assert_eq!(summary.len(), 2);
    }

    #[tokio::test]
    async fn test_convert_tool_local_shell() {
        let prompt = vec![Message {
            role: Role::Assistant,
            content: Content::Parts(vec![ContentPart {
                content_type: "tool_use".to_string(),
                tool_use: Some(ToolUse {
                    id: "call_123".to_string(),
                    name: "local_shell".to_string(),
                    input: json!({
                        "action": {
                            "type": "exec",
                            "command": ["bash", "-lc", "echo hello"],
                        }
                    }),
                }),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        }];

        let (input, warnings) = convert_to_openai_responses_input(
            &prompt,
            SystemMessageMode::Developer,
            None,
            false,
            true,
        )
        .await;

        assert!(warnings.is_empty());
        let shell_call = input
            .iter()
            .find(|item| item.get("type").and_then(Value::as_str) == Some("local_shell_call"))
            .expect("local shell call must be emitted");

        assert_eq!(shell_call["call_id"], "call_123");
        assert_eq!(shell_call["action"]["type"], "exec");
        assert_eq!(shell_call["action"]["command"][0], "bash");
    }

    #[tokio::test]
    async fn test_convert_tool_message_local_shell_output() {
        let mut opts = HashMap::new();
        opts.insert(
            "toolName".to_string(),
            Value::String("local_shell".to_string()),
        );

        let prompt = vec![Message {
            role: Role::Tool,
            content: Content::Parts(vec![ContentPart {
                content_type: "tool_result".to_string(),
                tool_result: Some(ToolResult {
                    tool_use_id: "call_1".to_string(),
                    content: "ok".to_string(),
                    is_error: Some(false),
                }),
                provider_options: Some(opts),
                ..Default::default()
            }]),
            cache_control: None,
            provider_options: None,
        }];

        let (input, _) = convert_to_openai_responses_input(
            &prompt,
            SystemMessageMode::Developer,
            None,
            false,
            true,
        )
        .await;

        assert_eq!(input[0]["type"], "local_shell_call_output");
        assert_eq!(input[0]["call_id"], "call_1");
    }
}
