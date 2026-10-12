use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{Tool, ToolContext, ToolError, ToolResult};

const API_BASE_URL: &str = "https://mcp.exa.ai";
const DEFAULT_NUM_RESULTS: usize = 8;

pub struct WebSearchTool {
    client: Client,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WebSearchInput {
    query: String,
    #[serde(default = "default_num_results", alias = "numResults")]
    num_results: usize,
    #[serde(default)]
    livecrawl: Option<String>,
    #[serde(rename = "type", default)]
    search_type: Option<String>,
    #[serde(default, alias = "contextMaxCharacters")]
    context_max_characters: Option<usize>,
}

fn default_num_results() -> usize {
    DEFAULT_NUM_RESULTS
}

#[derive(Debug, Serialize)]
struct McpSearchRequest {
    jsonrpc: String,
    id: u32,
    method: String,
    params: McpSearchParams,
}

#[derive(Debug, Serialize)]
struct McpSearchParams {
    name: String,
    arguments: McpSearchArguments,
}

#[derive(Debug, Serialize)]
struct McpSearchArguments {
    query: String,
    #[serde(rename = "type")]
    search_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "numResults")]
    num_results: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    livecrawl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "contextMaxCharacters")]
    context_max_characters: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct McpSearchResponse {
    result: McpSearchResult,
}

#[derive(Debug, Deserialize)]
struct McpSearchResult {
    content: Vec<McpContent>,
}

#[derive(Debug, Deserialize)]
struct McpContent {
    #[serde(rename = "type")]
    _content_type: String,
    text: String,
}

static DESCRIPTION: &str = r#"Search the web for real-time information using Exa AI search engine.

This tool provides access to current information from across the web. Use it when you need:
- Current events or news
- Latest documentation or library updates
- Real-time data (weather, stock prices, etc.)
- Recent research or publications
- Any information that may have changed since the knowledge cutoff date

The search returns relevant web pages with their content, optimized for LLM context."#;

#[async_trait]
impl Tool for WebSearchTool {
    fn id(&self) -> &str {
        "websearch"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Web search query"
                },
                "numResults": {
                    "type": "integer",
                    "default": 8,
                    "description": "Number of search results to return"
                },
                "num_results": {
                    "type": "integer",
                    "default": 8,
                    "description": "Number of search results to return (snake_case alias)"
                },
                "livecrawl": {
                    "type": "string",
                    "enum": ["fallback", "preferred"],
                    "default": "fallback",
                    "description": "Live crawl mode - 'fallback': use live crawling as backup if cached content unavailable, 'preferred': prioritize live crawling"
                },
                "type": {
                    "type": "string",
                    "enum": ["auto", "fast", "deep"],
                    "default": "auto",
                    "description": "Search type - 'auto': balanced search, 'fast': quick results, 'deep': comprehensive search"
                },
                "contextMaxCharacters": {
                    "type": "integer",
                    "default": 10000,
                    "description": "Maximum characters for context string optimized for LLMs"
                },
                "context_max_characters": {
                    "type": "integer",
                    "default": 10000,
                    "description": "Maximum characters for context string optimized for LLMs (snake_case alias)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: WebSearchInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        ctx.ask_permission(
            crate::PermissionRequest::new("websearch")
                .with_pattern(&input.query)
                .with_metadata("query", serde_json::Value::String(input.query.clone()))
                .with_metadata(
                    "numResults",
                    serde_json::Value::Number(input.num_results.into()),
                )
                .always_allow(),
        )
        .await?;

        let search_request = McpSearchRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "tools/call".to_string(),
            params: McpSearchParams {
                name: "web_search_exa".to_string(),
                arguments: McpSearchArguments {
                    query: input.query.clone(),
                    search_type: input.search_type.or(Some("auto".to_string())),
                    num_results: Some(input.num_results),
                    livecrawl: input.livecrawl.or(Some("fallback".to_string())),
                    context_max_characters: input.context_max_characters,
                },
            },
        };

        let response = self
            .client
            .post(format!("{}/mcp", API_BASE_URL))
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .json(&search_request)
            .timeout(std::time::Duration::from_secs(25))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ToolError::ExecutionError("Search request timed out".to_string())
                } else {
                    ToolError::ExecutionError(format!("Search request failed: {}", e))
                }
            })?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(ToolError::ExecutionError(format!(
                "Search error ({}): {}",
                status, response_text
            )));
        }

        let output = parse_sse_response(&response_text).unwrap_or_else(|| {
            "No search results found. Please try a different query.".to_string()
        });

        Ok(ToolResult {
            title: format!("Web search: {}", input.query),
            output,
            metadata: std::collections::HashMap::new(),
            truncated: false,
        })
    }
}

fn parse_sse_response(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(response) = serde_json::from_str::<McpSearchResponse>(data) {
                if !response.result.content.is_empty() {
                    return Some(response.result.content[0].text.clone());
                }
            }
        }
    }
    None
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}
