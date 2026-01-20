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
