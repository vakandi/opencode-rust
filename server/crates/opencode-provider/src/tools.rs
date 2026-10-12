//! Tool definitions and preparation for the OpenAI Responses API and
//! OpenAI-compatible chat completions.
//!
//! Mirrors the TS files:
//! - `openai-responses-prepare-tools.ts`
//! - `openai-compatible-prepare-tools.ts`
//! - `tool/web-search.ts`, `tool/code-interpreter.ts`, `tool/file-search.ts`,
//!   `tool/image-generation.ts`, `tool/local-shell.ts`, `tool/web-search-preview.ts`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::responses::CallWarning;

// ---------------------------------------------------------------------------
// Provider-Defined Tool Types
// ---------------------------------------------------------------------------

/// Web search action types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WebSearchAction {
    #[serde(rename = "search")]
    Search {
        #[serde(skip_serializing_if = "Option::is_none")]
        query: Option<String>,
    },
    #[serde(rename = "open_page")]
    OpenPage { url: String },
    #[serde(rename = "find")]
    Find { url: String, pattern: String },
}

/// User location for web search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLocation {
    #[serde(rename = "type")]
    pub location_type: String, // "approximate"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

/// Web search args (for `openai.web_search`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebSearchArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<WebSearchFilters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>, // "low" | "medium" | "high"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_location: Option<UserLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
}

/// Web search preview args (for `openai.web_search_preview`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebSearchPreviewArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_location: Option<UserLocation>,
}

/// Code interpreter args (for `openai.code_interpreter`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeInterpreterArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<CodeInterpreterContainer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CodeInterpreterContainer {
    Id(String),
    Config(CodeInterpreterContainerConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeInterpreterContainerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ids: Option<Vec<String>>,
}

/// Code interpreter input schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeInterpreterInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub container_id: String,
}

/// Code interpreter output schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeInterpreterOutputResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<CodeInterpreterOutputItem>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CodeInterpreterOutputItem {
    #[serde(rename = "logs")]
    Logs { logs: String },
    #[serde(rename = "image")]
    Image { url: String },
}

/// File search args (for `openai.file_search`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchArgs {
    pub vector_store_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_num_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranking: Option<FileSearchRanking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchRanking {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f64>,
}

/// File search output schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchOutput {
    pub queries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<FileSearchResultItem>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResultItem {
    #[serde(default)]
    pub attributes: HashMap<String, serde_json::Value>,
    pub file_id: String,
    pub filename: String,
    pub score: f64,
    pub text: String,
}

/// Image generation args (for `openai.image_generation`).
/// 10+ config options matching the TS `imageGenerationArgsSchema`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageGenerationArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>, // "auto" | "opaque" | "transparent"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_fidelity: Option<String>, // "low" | "high"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_image_mask: Option<ImageMask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation: Option<String>, // "auto"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_compression: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>, // "png" | "jpeg" | "webp"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_images: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>, // "auto" | "low" | "medium" | "high"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>, // "auto" | "1024x1024" | "1024x1536" | "1536x1024"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMask {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

/// Image generation output schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationOutput {
    pub result: String,
}

/// Local shell input schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalShellInput {
    pub action: LocalShellInputAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalShellInputAction {
    #[serde(rename = "type")]
    pub action_type: String, // "exec"
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "workingDirectory")]
    pub working_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

/// Local shell output schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalShellOutput {
    pub output: String,
}

// ---------------------------------------------------------------------------
// File Search Filter Types
// ---------------------------------------------------------------------------

/// Comparison filter for file search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonFilter {
    pub key: String,
    #[serde(rename = "type")]
    pub filter_type: String, // "eq" | "ne" | "gt" | "gte" | "lt" | "lte"
    pub value: serde_json::Value, // string | number | boolean
}

/// Compound filter for file search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundFilter {
    #[serde(rename = "type")]
    pub filter_type: String, // "and" | "or"
    pub filters: Vec<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// OpenAI Responses Tool (wire format)
// ---------------------------------------------------------------------------

/// Tool definitions sent to the Responses API.
/// Mirrors TS `OpenAIResponsesTool`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponsesTool {
    #[serde(rename = "function")]
    Function {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        parameters: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
    #[serde(rename = "web_search")]
    WebSearch {
        #[serde(skip_serializing_if = "Option::is_none")]
        filters: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        search_context_size: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        user_location: Option<UserLocation>,
    },
    #[serde(rename = "web_search_preview")]
    WebSearchPreview {
        #[serde(skip_serializing_if = "Option::is_none")]
        search_context_size: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        user_location: Option<UserLocation>,
    },
    #[serde(rename = "code_interpreter")]
    CodeInterpreter { container: serde_json::Value },
    #[serde(rename = "file_search")]
    FileSearch {
        vector_store_ids: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_num_results: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ranking_options: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filters: Option<serde_json::Value>,
    },
    #[serde(rename = "image_generation")]
    ImageGeneration {
        #[serde(skip_serializing_if = "Option::is_none")]
        background: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_fidelity: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_image_mask: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        moderation: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_compression: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_format: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        partial_images: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        quality: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<String>,
    },
    #[serde(rename = "local_shell")]
    LocalShell {},
}

// ---------------------------------------------------------------------------
// Tool Choice Types
// ---------------------------------------------------------------------------

/// Tool choice for the Responses API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsesToolChoice {
    /// "auto", "none", "required"
    Mode(String),
    /// Specific tool type: { type: "function", name: "..." } etc.
    Specific(ResponsesToolChoiceSpecific),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesToolChoiceSpecific {
    #[serde(rename = "type")]
    pub choice_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Tool choice for OpenAI-compatible chat completions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatToolChoice {
    Mode(String),
    Specific(ChatToolChoiceSpecific),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolChoiceSpecific {
    #[serde(rename = "type")]
    pub choice_type: String,
    pub function: ChatToolChoiceFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolChoiceFunction {
    pub name: String,
}

/// Input tool definition from the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InputTool {
    #[serde(rename = "function")]
    Function {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(rename = "inputSchema")]
        input_schema: serde_json::Value,
    },
    #[serde(rename = "provider-defined")]
    ProviderDefined {
        id: String,
        name: String,
        #[serde(default)]
        args: serde_json::Value,
    },
}

/// Input tool choice from the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InputToolChoice {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "none")]
    None,
    #[serde(rename = "required")]
    Required,
    #[serde(rename = "tool")]
    Tool {
        #[serde(rename = "toolName")]
        tool_name: String,
    },
}

// ---------------------------------------------------------------------------
// Prepare Responses Tools
// ---------------------------------------------------------------------------

/// Result of preparing tools for the Responses API.
pub struct PreparedResponsesTools {
    pub tools: Option<Vec<ResponsesTool>>,
    pub tool_choice: Option<ResponsesToolChoice>,
    pub tool_warnings: Vec<CallWarning>,
}

/// Prepare tools for the OpenAI Responses API.
/// Mirrors TS `prepareResponsesTools()`.
pub fn prepare_responses_tools(
    tools: Option<&[InputTool]>,
    tool_choice: Option<&InputToolChoice>,
    strict_json_schema: bool,
) -> PreparedResponsesTools {
    let tools = match tools {
        Some(t) if !t.is_empty() => t,
        _ => {
            return PreparedResponsesTools {
                tools: None,
                tool_choice: None,
                tool_warnings: vec![],
            }
        }
    };

    let mut openai_tools = Vec::new();
    let mut tool_warnings = Vec::new();

    for tool in tools {
        match tool {
            InputTool::Function {
                name,
                description,
                input_schema,
            } => {
                openai_tools.push(ResponsesTool::Function {
                    name: name.clone(),
                    description: description.clone(),
                    parameters: input_schema.clone(),
                    strict: if strict_json_schema { Some(true) } else { None },
                });
            }
            InputTool::ProviderDefined { id, name, args } => match id.as_str() {
                "openai.file_search" => {
                    if let Ok(fs_args) = serde_json::from_value::<FileSearchArgs>(args.clone()) {
                        let ranking_options = fs_args.ranking.map(|r| {
                            serde_json::json!({
                                "ranker": r.ranker,
                                "score_threshold": r.score_threshold,
                            })
                        });
                        openai_tools.push(ResponsesTool::FileSearch {
                            vector_store_ids: fs_args.vector_store_ids,
                            max_num_results: fs_args.max_num_results,
                            ranking_options,
                            filters: fs_args.filters,
                        });
                    }
                }
                "openai.local_shell" => {
                    openai_tools.push(ResponsesTool::LocalShell {});
                }
                "openai.web_search_preview" => {
                    if let Ok(ws_args) =
                        serde_json::from_value::<WebSearchPreviewArgs>(args.clone())
                    {
                        openai_tools.push(ResponsesTool::WebSearchPreview {
                            search_context_size: ws_args.search_context_size,
                            user_location: ws_args.user_location,
                        });
                    }
                }
                "openai.web_search" => {
                    if let Ok(ws_args) = serde_json::from_value::<WebSearchArgs>(args.clone()) {
                        let filters = ws_args.filters.map(|f| {
                            serde_json::json!({
                                "allowed_domains": f.allowed_domains,
                            })
                        });
                        openai_tools.push(ResponsesTool::WebSearch {
                            filters,
                            search_context_size: ws_args.search_context_size,
                            user_location: ws_args.user_location,
                        });
                    }
                }
                "openai.code_interpreter" => {
                    if let Ok(ci_args) = serde_json::from_value::<CodeInterpreterArgs>(args.clone())
                    {
                        let container = match ci_args.container {
                            None => serde_json::json!({"type": "auto"}),
                            Some(CodeInterpreterContainer::Id(id)) => serde_json::Value::String(id),
                            Some(CodeInterpreterContainer::Config(cfg)) => {
                                serde_json::json!({
                                    "type": "auto",
                                    "file_ids": cfg.file_ids,
                                })
                            }
                        };
                        openai_tools.push(ResponsesTool::CodeInterpreter { container });
                    }
                }
                "openai.image_generation" => {
                    if let Ok(ig_args) = serde_json::from_value::<ImageGenerationArgs>(args.clone())
                    {
                        let input_image_mask = ig_args.input_image_mask.map(|m| {
                            serde_json::json!({
                                "file_id": m.file_id,
                                "image_url": m.image_url,
                            })
                        });
                        openai_tools.push(ResponsesTool::ImageGeneration {
                            background: ig_args.background,
                            input_fidelity: ig_args.input_fidelity,
                            input_image_mask,
                            model: ig_args.model,
                            moderation: ig_args.moderation,
                            output_compression: ig_args.output_compression,
                            output_format: ig_args.output_format,
                            partial_images: ig_args.partial_images,
                            quality: ig_args.quality,
                            size: ig_args.size,
                        });
                    }
                }
                _ => {
                    tool_warnings.push(CallWarning::UnsupportedTool {
                        tool_name: Some(name.clone()),
                    });
                }
            },
        }
    }

    let mapped_choice = tool_choice.map(|tc| match tc {
        InputToolChoice::Auto => ResponsesToolChoice::Mode("auto".to_string()),
        InputToolChoice::None => ResponsesToolChoice::Mode("none".to_string()),
        InputToolChoice::Required => ResponsesToolChoice::Mode("required".to_string()),
        InputToolChoice::Tool { tool_name } => {
            let builtin = [
                "code_interpreter",
                "file_search",
                "image_generation",
                "web_search_preview",
                "web_search",
            ];
            if builtin.contains(&tool_name.as_str()) {
                ResponsesToolChoice::Specific(ResponsesToolChoiceSpecific {
                    choice_type: tool_name.clone(),
                    name: None,
                })
            } else {
                ResponsesToolChoice::Specific(ResponsesToolChoiceSpecific {
                    choice_type: "function".to_string(),
                    name: Some(tool_name.clone()),
                })
            }
        }
    });

    PreparedResponsesTools {
        tools: Some(openai_tools),
        tool_choice: mapped_choice,
        tool_warnings,
    }
}

// ---------------------------------------------------------------------------
// OpenAI-Compatible Chat Tool (wire format)
// ---------------------------------------------------------------------------

/// Tool definition for OpenAI-compatible chat completions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub tool_type: String, // "function"
    pub function: ChatToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolFunction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

/// Result of preparing tools for OpenAI-compatible chat.
pub struct PreparedChatTools {
    pub tools: Option<Vec<ChatTool>>,
    pub tool_choice: Option<ChatToolChoice>,
    pub tool_warnings: Vec<CallWarning>,
}

/// Prepare tools for OpenAI-compatible chat completions.
/// Mirrors TS `prepareTools()`.
pub fn prepare_tools(
    tools: Option<&[InputTool]>,
    tool_choice: Option<&InputToolChoice>,
) -> PreparedChatTools {
    let tools = match tools {
        Some(t) if !t.is_empty() => t,
        _ => {
            return PreparedChatTools {
                tools: None,
                tool_choice: None,
                tool_warnings: vec![],
            }
        }
    };

    let mut openai_tools = Vec::new();
    let mut tool_warnings = Vec::new();

    for tool in tools {
        match tool {
            InputTool::Function {
                name,
                description,
                input_schema,
            } => {
                openai_tools.push(ChatTool {
                    tool_type: "function".to_string(),
                    function: ChatToolFunction {
                        name: name.clone(),
                        description: description.clone(),
                        parameters: input_schema.clone(),
                    },
                });
            }
            InputTool::ProviderDefined { name, .. } => {
                tool_warnings.push(CallWarning::UnsupportedTool {
                    tool_name: Some(name.clone()),
                });
            }
        }
    }

    let mapped_choice = tool_choice.map(|tc| match tc {
        InputToolChoice::Auto => ChatToolChoice::Mode("auto".to_string()),
        InputToolChoice::None => ChatToolChoice::Mode("none".to_string()),
        InputToolChoice::Required => ChatToolChoice::Mode("required".to_string()),
        InputToolChoice::Tool { tool_name } => ChatToolChoice::Specific(ChatToolChoiceSpecific {
            choice_type: "function".to_string(),
            function: ChatToolChoiceFunction {
                name: tool_name.clone(),
            },
        }),
    });

    PreparedChatTools {
        tools: Some(openai_tools),
        tool_choice: mapped_choice,
        tool_warnings,
    }
}
