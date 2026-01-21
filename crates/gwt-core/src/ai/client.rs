//! OpenAI-compatible API client

use crate::config::ResolvedAISettings;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_OUTPUT_TOKENS: u32 = 400;
const TEMPERATURE: f32 = 0.3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Error)]
pub enum AIError {
    /// API key invalid or missing
    #[error("Unauthorized")]
    Unauthorized,
    /// Rate limited
    #[error("Rate limited")]
    RateLimited { retry_after: Option<u64> },
    /// Server error
    #[error("Server error: {0}")]
    ServerError(String),
    /// Network error
    #[error("Network error: {0}")]
    NetworkError(String),
    /// Response parse error
    #[error("Parse error: {0}")]
    ParseError(String),
    /// Configuration error
    #[error("Config error: {0}")]
    ConfigError(String),
}

#[derive(Debug, Serialize)]
struct ResponsesRequest<'a> {
    model: &'a str,
    input: Vec<ResponseInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    max_output_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct ResponsesResponse {
    output: Vec<ResponseOutputItem>,
}

#[derive(Debug, Deserialize)]
struct ResponseOutputItem {
    #[serde(rename = "type")]
    item_type: String,
    role: Option<String>,
    content: Option<Vec<ResponseOutputContent>>,
}

#[derive(Debug, Deserialize)]
struct ResponseOutputContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, Serialize)]
struct ResponseInputItem {
    #[serde(rename = "type")]
    item_type: String,
    role: String,
    content: Vec<ResponseInputContent>,
}

#[derive(Debug, Serialize)]
struct ResponseInputContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

/// OpenAI-compatible API client (blocking)
pub struct AIClient {
    endpoint: String,
    api_key: String,
    model: String,
    client: Client,
}

impl AIClient {
    pub fn new(settings: ResolvedAISettings) -> Result<Self, AIError> {
        let endpoint = settings.endpoint.trim().to_string();
        let model = settings.model.trim().to_string();
        if endpoint.is_empty() {
            return Err(AIError::ConfigError("API endpoint is empty".to_string()));
        }
        if model.is_empty() {
            return Err(AIError::ConfigError("Model is empty".to_string()));
        }

        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| AIError::NetworkError(e.to_string()))?;

        Ok(Self {
            endpoint,
            api_key: settings.api_key,
            model,
            client,
        })
    }

    pub fn create_response(&self, messages: Vec<ChatMessage>) -> Result<String, AIError> {
        let url = build_responses_url(&self.endpoint)?;
        let (instructions, input) = build_responses_input(messages);
        if input.is_empty() {
            return Err(AIError::ConfigError("No input messages".to_string()));
        }
        let request_body = ResponsesRequest {
            model: &self.model,
            input,
            instructions,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            temperature: TEMPERATURE,
        };

        let mut rate_retries = 0usize;
        let mut server_retries = 0usize;
        let mut network_retries = 0usize;

        loop {
            let mut headers = HeaderMap::new();
            if !self.api_key.trim().is_empty() {
                if is_azure_endpoint(&url) {
                    headers.insert(
                        "api-key",
                        HeaderValue::from_str(self.api_key.trim())
                            .map_err(|e| AIError::ConfigError(e.to_string()))?,
                    );
                } else {
                    let value = format!("Bearer {}", self.api_key.trim());
                    headers.insert(
                        AUTHORIZATION,
                        HeaderValue::from_str(&value)
                            .map_err(|e| AIError::ConfigError(e.to_string()))?,
                    );
                }
            }

            let response = self
                .client
                .post(url.clone())
                .headers(headers)
                .json(&request_body)
                .send();

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    let headers = resp.headers().clone();
                    let body = resp.text().unwrap_or_default();
                    if status == StatusCode::OK {
                        return parse_response(&body);
                    }
                    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                        return Err(AIError::Unauthorized);
                    }
                    if status == StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = parse_retry_after(&body, &headers);
                        if rate_retries < 3 {
                            let delay = retry_after.unwrap_or(1 << rate_retries);
                            std::thread::sleep(Duration::from_secs(delay));
                            rate_retries += 1;
                            continue;
                        }
                        return Err(AIError::RateLimited { retry_after });
                    }
                    if status.is_server_error() {
                        let message = extract_error_message(&body)
                            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                        if server_retries < 2 {
                            std::thread::sleep(Duration::from_secs(1));
                            server_retries += 1;
                            continue;
                        }
                        return Err(AIError::ServerError(message));
                    }

                    let message = extract_error_message(&body)
                        .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                    return Err(AIError::ServerError(message));
                }
                Err(err) => {
                    let message = err.to_string();
                    if (err.is_timeout() || err.is_connect()) && network_retries < 2 {
                        std::thread::sleep(Duration::from_secs(1));
                        network_retries += 1;
                        continue;
                    }
                    return Err(AIError::NetworkError(message));
                }
            }
        }
    }
}

fn build_responses_url(endpoint: &str) -> Result<Url, AIError> {
    let mut url = Url::parse(endpoint)
        .map_err(|e| AIError::ConfigError(format!("Invalid endpoint: {}", e)))?;
    let mut path = url.path().trim_end_matches('/').to_string();
    if !path.ends_with("/responses") {
        if path.is_empty() {
            path = "/responses".to_string();
        } else {
            path = format!("{}/responses", path);
        }
        url.set_path(&path);
    }
    Ok(url)
}

fn is_azure_endpoint(url: &Url) -> bool {
    url.host_str()
        .map(|host| host.contains("openai.azure.com"))
        .unwrap_or(false)
}

fn build_responses_input(messages: Vec<ChatMessage>) -> (Option<String>, Vec<ResponseInputItem>) {
    let mut instructions_parts = Vec::new();
    let mut items = Vec::new();
    for message in messages {
        if message.role == "system" {
            if !message.content.trim().is_empty() {
                instructions_parts.push(message.content);
            }
            continue;
        }
        items.push(ResponseInputItem {
            item_type: "message".to_string(),
            role: message.role,
            content: vec![ResponseInputContent {
                content_type: "input_text".to_string(),
                text: message.content,
            }],
        });
    }
    let instructions = if instructions_parts.is_empty() {
        None
    } else {
        Some(instructions_parts.join("\n"))
    };
    (instructions, items)
}

fn parse_response(body: &str) -> Result<String, AIError> {
    let parsed: ResponsesResponse = serde_json::from_str(body)
        .map_err(|e| AIError::ParseError(format!("Invalid response: {}", e)))?;
    let mut texts = Vec::new();
    for item in parsed.output {
        if item.item_type != "message" {
            continue;
        }
        if item.role.as_deref() != Some("assistant") {
            continue;
        }
        if let Some(contents) = item.content {
            for content in contents {
                if content.content_type == "output_text" {
                    if let Some(text) = content.text {
                        texts.push(text);
                    }
                }
            }
        }
    }
    if texts.is_empty() {
        return Err(AIError::ParseError(
            "No assistant output_text in response".to_string(),
        ));
    }
    Ok(texts.join(""))
}

fn extract_error_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(|message| message.as_str())
        .map(|s| s.to_string())
}

fn parse_retry_after(body: &str, headers: &reqwest::header::HeaderMap) -> Option<u64> {
    if let Some(value) = headers.get("retry-after") {
        if let Ok(text) = value.to_str() {
            if let Ok(seconds) = text.parse::<u64>() {
                return Some(seconds);
            }
        }
    }
    extract_error_message(body).and_then(|message| message.parse::<u64>().ok())
}

// ============================================================================
// Model List API (GET /models) - AI設定疎通チェック用
// ============================================================================

const LIST_MODELS_TIMEOUT: Duration = Duration::from_secs(10);

/// Model information from GET /models API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model ID (e.g., "gpt-4o-mini", "gpt-4")
    pub id: String,
    /// Creation timestamp (Unix epoch)
    #[serde(default)]
    pub created: i64,
    /// Owner (e.g., "openai", "system")
    #[serde(default)]
    pub owned_by: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

impl AIClient {
    /// Create a new AIClient for list_models only (without model validation)
    pub fn new_for_list_models(endpoint: &str, api_key: &str) -> Result<Self, AIError> {
        let endpoint = endpoint.trim().to_string();
        if endpoint.is_empty() {
            return Err(AIError::ConfigError("API endpoint is empty".to_string()));
        }

        let client = Client::builder()
            .connect_timeout(LIST_MODELS_TIMEOUT)
            .timeout(LIST_MODELS_TIMEOUT)
            .build()
            .map_err(|e| AIError::NetworkError(e.to_string()))?;

        Ok(Self {
            endpoint,
            api_key: api_key.to_string(),
            model: String::new(), // not used for list_models
            client,
        })
    }

    /// List available models from the API (GET /models)
    /// Used for connection check and model selection in AI settings wizard
    pub fn list_models(&self) -> Result<Vec<ModelInfo>, AIError> {
        let url = build_models_url(&self.endpoint)?;

        let mut headers = HeaderMap::new();
        if !self.api_key.trim().is_empty() {
            if is_azure_endpoint(&url) {
                headers.insert(
                    "api-key",
                    HeaderValue::from_str(self.api_key.trim())
                        .map_err(|e| AIError::ConfigError(e.to_string()))?,
                );
            } else {
                let value = format!("Bearer {}", self.api_key.trim());
                headers.insert(
                    AUTHORIZATION,
                    HeaderValue::from_str(&value)
                        .map_err(|e| AIError::ConfigError(e.to_string()))?,
                );
            }
        }

        let response = self.client.get(url.clone()).headers(headers).send();

        match response {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().unwrap_or_default();

                if status == StatusCode::OK {
                    return parse_models_response(&body);
                }
                if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                    return Err(AIError::Unauthorized);
                }
                if status == StatusCode::TOO_MANY_REQUESTS {
                    return Err(AIError::RateLimited { retry_after: None });
                }

                let message = extract_error_message(&body)
                    .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                Err(AIError::ServerError(message))
            }
            Err(err) => {
                if err.is_timeout() {
                    return Err(AIError::NetworkError("Connection timed out".to_string()));
                }
                if err.is_connect() {
                    return Err(AIError::NetworkError("Connection refused".to_string()));
                }
                Err(AIError::NetworkError(err.to_string()))
            }
        }
    }
}

fn build_models_url(endpoint: &str) -> Result<Url, AIError> {
    let mut url = Url::parse(endpoint)
        .map_err(|e| AIError::ConfigError(format!("Invalid endpoint: {}", e)))?;
    let mut path = url.path().trim_end_matches('/').to_string();
    if !path.ends_with("/models") {
        if path.is_empty() {
            path = "/models".to_string();
        } else {
            path = format!("{}/models", path);
        }
        url.set_path(&path);
    }
    Ok(url)
}

fn parse_models_response(body: &str) -> Result<Vec<ModelInfo>, AIError> {
    let parsed: ModelsResponse = serde_json::from_str(body)
        .map_err(|e| AIError::ParseError(format!("Invalid models response: {}", e)))?;
    Ok(parsed.data)
}

/// Format a detailed error message for display in the wizard
pub fn format_error_for_display(error: &AIError) -> String {
    match error {
        AIError::Unauthorized => "401 Unauthorized - Check your API key".to_string(),
        AIError::RateLimited { retry_after } => match retry_after {
            Some(secs) => format!("429 Rate Limited - Retry after {} seconds", secs),
            None => "429 Rate Limited - Please try again later".to_string(),
        },
        AIError::ServerError(msg) => format!("Server error: {}", msg),
        AIError::NetworkError(msg) => {
            if msg.contains("timed out") {
                "Connection timed out - Check the endpoint URL".to_string()
            } else if msg.contains("refused") {
                "Connection refused - Check the endpoint URL".to_string()
            } else {
                format!("Network error: {}", msg)
            }
        }
        AIError::ParseError(msg) => format!("Parse error: {}", msg),
        AIError::ConfigError(msg) => format!("Configuration error: {}", msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================
    // URL Building Tests
    // ========================================

    #[test]
    fn test_build_models_url_appends_models_path() {
        let url = build_models_url("https://api.openai.com/v1").unwrap();
        assert_eq!(url.as_str(), "https://api.openai.com/v1/models");
    }

    #[test]
    fn test_build_models_url_with_trailing_slash() {
        let url = build_models_url("https://api.openai.com/v1/").unwrap();
        assert_eq!(url.as_str(), "https://api.openai.com/v1/models");
    }

    #[test]
    fn test_build_models_url_already_has_models() {
        let url = build_models_url("https://api.openai.com/v1/models").unwrap();
        assert_eq!(url.as_str(), "https://api.openai.com/v1/models");
    }

    #[test]
    fn test_build_models_url_local_llm() {
        let url = build_models_url("http://localhost:11434/v1").unwrap();
        assert_eq!(url.as_str(), "http://localhost:11434/v1/models");
    }

    #[test]
    fn test_build_models_url_invalid() {
        let result = build_models_url("not-a-url");
        assert!(result.is_err());
    }

    // ========================================
    // Response Parsing Tests
    // ========================================

    #[test]
    fn test_parse_models_response_openai_format() {
        let body = r#"{
            "object": "list",
            "data": [
                {"id": "gpt-4o-mini", "created": 1715367049, "owned_by": "system"},
                {"id": "gpt-4", "created": 1678604602, "owned_by": "openai"}
            ]
        }"#;
        let models = parse_models_response(body).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4o-mini");
        assert_eq!(models[1].id, "gpt-4");
    }

    #[test]
    fn test_parse_models_response_ollama_format() {
        // Ollama uses the same OpenAI-compatible format
        let body = r#"{
            "data": [
                {"id": "llama3.2:latest", "created": 0, "owned_by": "library"},
                {"id": "codellama:7b", "created": 0, "owned_by": "library"}
            ]
        }"#;
        let models = parse_models_response(body).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "llama3.2:latest");
    }

    #[test]
    fn test_parse_models_response_empty() {
        let body = r#"{"data": []}"#;
        let models = parse_models_response(body).unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn test_parse_models_response_invalid_json() {
        let body = "not json";
        let result = parse_models_response(body);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_models_response_missing_data() {
        let body = r#"{"models": []}"#;
        let result = parse_models_response(body);
        assert!(result.is_err());
    }

    // ========================================
    // Error Formatting Tests
    // ========================================

    #[test]
    fn test_format_error_unauthorized() {
        let error = AIError::Unauthorized;
        let msg = format_error_for_display(&error);
        assert!(msg.contains("401"));
        assert!(msg.contains("Unauthorized"));
    }

    #[test]
    fn test_format_error_rate_limited() {
        let error = AIError::RateLimited {
            retry_after: Some(60),
        };
        let msg = format_error_for_display(&error);
        assert!(msg.contains("429"));
        assert!(msg.contains("60"));
    }

    #[test]
    fn test_format_error_timeout() {
        let error = AIError::NetworkError("Connection timed out".to_string());
        let msg = format_error_for_display(&error);
        assert!(msg.contains("timed out"));
    }

    #[test]
    fn test_format_error_connection_refused() {
        let error = AIError::NetworkError("Connection refused".to_string());
        let msg = format_error_for_display(&error);
        assert!(msg.contains("refused"));
    }

    // ========================================
    // Client Creation Tests
    // ========================================

    #[test]
    fn test_new_for_list_models_valid() {
        let result = AIClient::new_for_list_models("https://api.openai.com/v1", "test-key");
        assert!(result.is_ok());
    }

    #[test]
    fn test_new_for_list_models_empty_endpoint() {
        let result = AIClient::new_for_list_models("", "test-key");
        assert!(result.is_err());
    }

    #[test]
    fn test_new_for_list_models_empty_api_key_allowed() {
        // Empty API key should be allowed for local LLMs
        let result = AIClient::new_for_list_models("http://localhost:11434/v1", "");
        assert!(result.is_ok());
    }
}
