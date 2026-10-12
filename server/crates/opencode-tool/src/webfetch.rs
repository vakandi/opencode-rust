use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{Tool, ToolContext, ToolError, ToolResult};

const MAX_RESPONSE_SIZE: usize = 5 * 1024 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 120;

pub struct WebFetchTool {
    client: Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36")
                .timeout(std::time::Duration::from_secs(MAX_TIMEOUT_SECS))
                .build()
                .unwrap(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WebFetchInput {
    url: String,
    #[serde(default = "default_format")]
    format: String,
    #[serde(default)]
    timeout: Option<u64>,
}

fn default_format() -> String {
    "markdown".to_string()
}

#[async_trait]
impl Tool for WebFetchTool {
    fn id(&self) -> &str {
        "webfetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL. Returns the content in the specified format (text, markdown, or html). Defaults to markdown."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "default": "markdown",
                    "description": "The format to return the content in (text, markdown, or html). Defaults to markdown."
                },
                "timeout": {
                    "type": "number",
                    "description": "Optional timeout in seconds (max 120)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: WebFetchInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        let url = input.url.clone();

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::InvalidArguments(
                "URL must start with http:// or https://".to_string(),
            ));
        }

        ctx.ask_permission(
            crate::PermissionRequest::new("webfetch")
                .with_pattern(&url)
                .always_allow(),
        )
        .await?;

        let timeout_secs = input
            .timeout
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);

        let accept_header = match input.format.as_str() {
            "markdown" => "text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1",
            "text" => "text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1",
            "html" => "text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1",
            _ => "*/*",
        };

        let response = tokio::select! {
            result = self.fetch_with_retry(&url, accept_header, timeout_secs) => result,
            _ = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)) => {
                return Err(ToolError::Timeout(format!("Request timed out after {} seconds", timeout_secs)));
            }
            _ = ctx.abort.cancelled() => {
                return Err(ToolError::Cancelled);
            }
        };

        let response = response?;

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let content_length = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok());

        if let Some(len) = content_length {
            if len > MAX_RESPONSE_SIZE {
                return Err(ToolError::ExecutionError(
                    "Response too large (exceeds 5MB limit)".to_string(),
                ));
            }
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to read response: {}", e)))?;

        if bytes.len() > MAX_RESPONSE_SIZE {
            return Err(ToolError::ExecutionError(
                "Response too large (exceeds 5MB limit)".to_string(),
            ));
        }

        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let title = format!("{} ({})", url, content_type);

        let is_image = mime.starts_with("image/")
            && mime != "image/svg+xml"
            && mime != "image/vnd.fastbidsheet";

        if is_image {
            let base64_content =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
            let data_url = format!("data:{};base64,{}", mime, base64_content);
            let output = format!(
                "Image fetched successfully.\n\n<attachment type=\"image\" mimeType=\"{}\" url=\"{}\" size=\"{}\" data=\"{}\" />",
                mime, url, bytes.len(), data_url
            );
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("url".to_string(), serde_json::json!(url));
            metadata.insert("mimeType".to_string(), serde_json::json!(mime));
            metadata.insert("size".to_string(), serde_json::json!(bytes.len()));
            metadata.insert("data".to_string(), serde_json::json!(data_url));
            metadata.insert(
                "attachment".to_string(),
                serde_json::json!({
                    "type": "image",
                    "mimeType": mime,
                    "url": url,
                    "size": bytes.len(),
                    "data": data_url
                }),
            );
            return Ok(ToolResult {
                title,
                output,
                metadata,
                truncated: false,
            });
        }

        let content = String::from_utf8_lossy(&bytes).to_string();

        let output = match input.format.as_str() {
            "markdown" => {
                if content_type.contains("text/html") {
                    convert_html_to_markdown(&content)
                } else {
                    content
                }
            }
            "text" => {
                if content_type.contains("text/html") {
                    strip_html(&content)
                } else {
                    content
                }
            }
            "html" | _ => content,
        };

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("url".to_string(), serde_json::json!(url));
        metadata.insert("format".to_string(), serde_json::json!(input.format));
        metadata.insert("mimeType".to_string(), serde_json::json!(mime));
        metadata.insert("size".to_string(), serde_json::json!(output.len()));

        Ok(ToolResult {
            title,
            output,
            metadata,
            truncated: false,
        })
    }
}

impl WebFetchTool {
    async fn fetch_with_retry(
        &self,
        url: &str,
        accept_header: &str,
        _timeout_secs: u64,
    ) -> Result<reqwest::Response, ToolError> {
        let response = self
            .client
            .get(url)
            .header("Accept", accept_header)
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to fetch URL: {}", e)))?;

        if response.status() == 403 {
            let cf_mitigated = response
                .headers()
                .get("cf-mitigated")
                .and_then(|v| v.to_str().ok());

            if cf_mitigated == Some("challenge") {
                return self
                    .client
                    .get(url)
                    .header("Accept", accept_header)
                    .header("User-Agent", "opencode")
                    .send()
                    .await
                    .map_err(|e| ToolError::ExecutionError(format!("Failed to fetch URL: {}", e)));
            }
        }

        if !response.status().is_success() {
            return Err(ToolError::ExecutionError(format!(
                "Request failed with status code: {}",
                response.status()
            )));
        }

        Ok(response)
    }
}

fn convert_html_to_markdown(html: &str) -> String {
    html2md::parse_html(html)
}

fn strip_html(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];

        if c == '<' {
            if i + 7 <= len {
                let tag: String = chars[i..i + 7].iter().collect();
                let tag_lower = tag.to_lowercase();
                if tag_lower.starts_with("<script") {
                    in_script = true;
                } else if tag_lower.starts_with("<style") {
                    in_style = true;
                }
            }
            in_tag = true;
            i += 1;
            continue;
        }

        if c == '>' {
            if in_script {
                if i >= 8 {
                    let end_tag: String = chars[i - 8..=i].iter().collect();
                    if end_tag.to_lowercase() == "</script>" {
                        in_script = false;
                    }
                }
            } else if in_style {
                if i >= 7 {
                    let end_tag: String = chars[i - 7..=i].iter().collect();
                    if end_tag.to_lowercase() == "</style>" {
                        in_style = false;
                    }
                }
            }
            in_tag = false;
            i += 1;
            continue;
        }

        if !in_tag && !in_script && !in_style {
            if c == '&' {
                if i + 4 <= len {
                    let entity: String = chars[i..i + 4].iter().collect();
                    match entity.as_str() {
                        "&lt;" => {
                            result.push('<');
                            i += 4;
                            continue;
                        }
                        "&gt;" => {
                            result.push('>');
                            i += 4;
                            continue;
                        }
                        "&amp;" => {
                            result.push('&');
                            i += 5;
                            continue;
                        }
                        _ => {}
                    }
                }
                if i + 6 <= len {
                    let entity: String = chars[i..i + 6].iter().collect();
                    if entity == "&nbsp;" {
                        result.push(' ');
                        i += 6;
                        continue;
                    }
                }
            }
            result.push(c);
        }

        i += 1;
    }

    let result = result
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    result
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}
