//! OpenAI-compatible API client

use crate::config::ResolvedAISettings;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

const MAX_OUTPUT_TOKENS: u32 = 1024;
const TEMPERATURE: f32 = 0.3;

const MAX_RETRIES: usize = 5;
const BACKOFF_BASE_SECS: u64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
    #[serde(default)]
    pub call_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AIResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage_tokens: Option<u64>,
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
    /// Summary response incomplete (missing required sections)
    #[error("Incomplete summary")]
    IncompleteSummary,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDefinition>,
    max_output_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct ResponsesResponse {
    output: Vec<ResponseOutputItem>,
    #[serde(default)]
    usage: Option<UsageInfo>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct UsageInfo {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
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

#[derive(Debug, Serialize)]
struct ChatCompletionsRequest<'a> {
    model: &'a str,
    messages: Vec<ChatCompletionsInputMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ChatCompletionsReasoning<'a>>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionsReasoning<'a> {
    effort: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct ChatCompletionsInputMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    choices: Vec<ChatCompletionsChoice>,
    #[serde(default)]
    usage: Option<UsageInfo>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsChoice {
    message: ChatCompletionsOutputMessage,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsOutputMessage {
    content: Option<Value>,
}

/// OpenAI-compatible API client (blocking)
pub struct AIClient {
    endpoint: String,
    api_key: String,
    model: String,
    client: Client,
    cumulative_tokens: AtomicU64,
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
            cumulative_tokens: AtomicU64::new(0),
        })
    }

    /// Returns the cumulative token count across all API calls
    pub fn cumulative_tokens(&self) -> u64 {
        self.cumulative_tokens.load(Ordering::Relaxed)
    }

    pub fn create_response(&self, messages: Vec<ChatMessage>) -> Result<String, AIError> {
        if should_prefer_chat_completions(&self.endpoint, &self.model) {
            return self.create_chat_completion(&messages);
        }

        let url = build_responses_url(&self.endpoint)?;
        let (instructions, input) = build_responses_input(&messages);
        if input.is_empty() {
            return Err(AIError::ConfigError("No input messages".to_string()));
        }
        let request_body = ResponsesRequest {
            model: &self.model,
            input,
            instructions,
            tools: Vec::new(),
            max_output_tokens: MAX_OUTPUT_TOKENS,
            temperature: TEMPERATURE,
        };

        let mut retries = 0usize;

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
                    let resp_headers = resp.headers().clone();
                    let body = resp.text().unwrap_or_default();
                    if status == StatusCode::OK {
                        let (text, usage) = parse_response_with_usage(&body)?;
                        if let Some(tokens) = usage {
                            self.cumulative_tokens.fetch_add(tokens, Ordering::Relaxed);
                        }
                        return Ok(text);
                    }
                    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                        return Err(AIError::Unauthorized);
                    }
                    let error = if status == StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = parse_retry_after(&body, &resp_headers);
                        AIError::RateLimited { retry_after }
                    } else if status.is_server_error() {
                        let message = extract_error_message(&body)
                            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                        AIError::ServerError(message)
                    } else {
                        let message = extract_error_message(&body)
                            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                        if should_fallback_to_chat_completions(status, &message) {
                            warn!(
                                status = status.as_u16(),
                                reason = %message,
                                "Responses API unavailable; falling back to chat completions"
                            );
                            return self.create_chat_completion(&messages);
                        }
                        return Err(AIError::ServerError(message));
                    };

                    if let AIError::ServerError(message) = &error {
                        if should_fallback_to_chat_completions(status, message) {
                            warn!(
                                status = status.as_u16(),
                                reason = %message,
                                "Responses API unavailable; falling back to chat completions"
                            );
                            return self.create_chat_completion(&messages);
                        }
                    }

                    if is_retryable(&error) && retries < MAX_RETRIES {
                        let delay = backoff_delay(retries);
                        warn!(
                            retry = retries + 1,
                            max_retries = MAX_RETRIES,
                            delay_secs = delay.as_secs(),
                            error = %error,
                            "Retrying API request"
                        );
                        std::thread::sleep(delay);
                        retries += 1;
                        continue;
                    }
                    return Err(error);
                }
                Err(err) => {
                    if should_fallback_on_transport_error(&err) {
                        warn!(
                            error = %err,
                            "Responses transport failed; falling back to chat completions"
                        );
                        return self.create_chat_completion(&messages);
                    }
                    return Err(AIError::NetworkError(err.to_string()));
                }
            }
        }
    }

    pub fn create_response_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<AIResponse, AIError> {
        let url = build_responses_url(&self.endpoint)?;
        let (instructions, input) = build_responses_input(&messages);
        if input.is_empty() {
            return Err(AIError::ConfigError("No input messages".to_string()));
        }
        let request_body = ResponsesRequest {
            model: &self.model,
            input,
            instructions,
            tools,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            temperature: TEMPERATURE,
        };

        let mut retries = 0usize;

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
                    let resp_headers = resp.headers().clone();
                    let body = resp.text().unwrap_or_default();
                    if status == StatusCode::OK {
                        let (text, tool_calls, usage) = parse_response_with_tools(&body)?;
                        if let Some(tokens) = usage {
                            self.cumulative_tokens.fetch_add(tokens, Ordering::Relaxed);
                        }
                        return Ok(AIResponse {
                            text,
                            tool_calls,
                            usage_tokens: usage,
                        });
                    }
                    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                        return Err(AIError::Unauthorized);
                    }
                    let error = if status == StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = parse_retry_after(&body, &resp_headers);
                        AIError::RateLimited { retry_after }
                    } else if status.is_server_error() {
                        let message = extract_error_message(&body)
                            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                        AIError::ServerError(message)
                    } else {
                        let message = extract_error_message(&body)
                            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                        return Err(AIError::ServerError(message));
                    };

                    if !is_retryable(&error) || retries >= MAX_RETRIES {
                        return Err(error);
                    }

                    let delay = backoff_delay(retries);
                    retries += 1;
                    std::thread::sleep(delay);
                }
                Err(e) => {
                    if retries >= MAX_RETRIES {
                        return Err(AIError::NetworkError(e.to_string()));
                    }
                    let delay = backoff_delay(retries);
                    retries += 1;
                    std::thread::sleep(delay);
                }
            }
        }
    }

    fn create_chat_completion(&self, messages: &[ChatMessage]) -> Result<String, AIError> {
        let url = build_chat_completions_url(&self.endpoint)?;
        let body_messages = build_chat_completions_messages(messages);
        if body_messages.is_empty() {
            return Err(AIError::ConfigError("No input messages".to_string()));
        }

        let mut attempts: Vec<Option<&str>> = vec![None];
        if let Some(effort) = preferred_chat_completion_reasoning_effort(&self.endpoint, &self.model) {
            attempts.insert(0, Some(effort));
        }

        let mut last_error: Option<AIError> = None;

        for reasoning in attempts {
            let request_body = build_chat_completions_request(
                &body_messages,
                &self.model,
                reasoning,
            );

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
                    let resp_headers = resp.headers().clone();
                    let body = resp.text().unwrap_or_default();
                    if status == StatusCode::OK {
                        match parse_chat_completion_with_usage(&body) {
                            Ok((text, usage)) => {
                                if let Some(tokens) = usage {
                                    self.cumulative_tokens.fetch_add(tokens, Ordering::Relaxed);
                                }
                                return Ok(text);
                            }
                            Err(error) => {
                                if reasoning.is_some()
                                    && should_retry_chat_completion_without_reasoning(&error)
                                {
                                    last_error = Some(error);
                                    continue;
                                }
                                return Err(error);
                            }
                        }
                    }
                    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                        return Err(AIError::Unauthorized);
                    }
                    if status == StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = parse_retry_after(&body, &resp_headers);
                        return Err(AIError::RateLimited { retry_after });
                    }
                    let message = extract_error_message(&body)
                        .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
                    if reasoning.is_some()
                        && should_retry_chat_completion_without_reasoning_message(status, &message)
                    {
                        last_error = Some(AIError::ServerError(message));
                        continue;
                    }
                    return Err(AIError::ServerError(message));
                }
                Err(err) => {
                    return Err(AIError::NetworkError(err.to_string()));
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| AIError::ServerError("Chat completion request failed".to_string())))
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

fn build_chat_completions_url(endpoint: &str) -> Result<Url, AIError> {
    let mut url = Url::parse(endpoint)
        .map_err(|e| AIError::ConfigError(format!("Invalid endpoint: {}", e)))?;
    let mut path = url.path().trim_end_matches('/').to_string();
    if path.ends_with("/responses") {
        path = path.trim_end_matches("/responses").to_string();
    }
    if !path.ends_with("/chat/completions") {
        if path.is_empty() {
            path = "/chat/completions".to_string();
        } else {
            path = format!("{}/chat/completions", path);
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

fn build_responses_input(messages: &[ChatMessage]) -> (Option<String>, Vec<ResponseInputItem>) {
    let mut instructions_parts = Vec::new();
    let mut items = Vec::new();
    for message in messages {
        if message.role == "system" {
            if !message.content.trim().is_empty() {
                instructions_parts.push(message.content.clone());
            }
            continue;
        }
        items.push(ResponseInputItem {
            item_type: "message".to_string(),
            role: message.role.clone(),
            content: vec![ResponseInputContent {
                content_type: "input_text".to_string(),
                text: message.content.clone(),
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

fn build_chat_completions_messages<'a>(
    messages: &'a [ChatMessage],
) -> Vec<ChatCompletionsInputMessage<'a>> {
    messages
        .iter()
        .filter(|message| !message.content.trim().is_empty())
        .map(|message| ChatCompletionsInputMessage {
            role: message.role.as_str(),
            content: message.content.as_str(),
        })
        .collect()
}

fn build_chat_completions_request<'a>(
    messages: &'a [ChatCompletionsInputMessage<'a>],
    model: &'a str,
    reasoning_effort: Option<&'a str>,
) -> ChatCompletionsRequest<'a> {
    ChatCompletionsRequest {
        model,
        messages: messages.to_vec(),
        max_tokens: MAX_OUTPUT_TOKENS,
        temperature: TEMPERATURE,
        reasoning: reasoning_effort.map(|effort| ChatCompletionsReasoning { effort }),
    }
}

fn preferred_chat_completion_reasoning_effort(endpoint: &str, model: &str) -> Option<&'static str> {
    if !model.contains(':') {
        return None;
    }

    let Ok(url) = Url::parse(endpoint) else {
        return None;
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == "api.openai.com" {
        return None;
    }

    Some("low")
}

fn should_retry_chat_completion_without_reasoning(error: &AIError) -> bool {
    matches!(
        error,
        AIError::ParseError(message)
            if message.contains("No message content in chat completion response")
                || message.contains("No text content in chat completion response")
    )
}

fn should_retry_chat_completion_without_reasoning_message(
    status: StatusCode,
    message: &str,
) -> bool {
    if !status.is_server_error() && status != StatusCode::BAD_REQUEST {
        return false;
    }

    let lower = message.to_ascii_lowercase();
    lower.contains("reasoning")
        && (lower.contains("cannot unmarshal")
            || lower.contains("invalid request")
            || lower.contains("unsupported")
            || lower.contains("not support"))
}

fn parse_response_with_usage(body: &str) -> Result<(String, Option<u64>), AIError> {
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
    let usage_tokens = parsed.usage.map(|u| u.total_tokens);
    Ok((texts.join(""), usage_tokens))
}

fn parse_response_with_tools(body: &str) -> Result<(String, Vec<ToolCall>, Option<u64>), AIError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| AIError::ParseError(format!("Invalid response: {}", e)))?;
    let output = value
        .get("output")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AIError::ParseError("Missing output array".to_string()))?;

    let mut texts = Vec::new();
    let mut tool_calls = Vec::new();

    for item in output {
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if item_type == "message" {
            if item.get("role").and_then(|v| v.as_str()).unwrap_or("") != "assistant" {
                continue;
            }
            if let Some(contents) = item.get("content").and_then(|v| v.as_array()) {
                for content in contents {
                    let content_type = content.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if content_type == "output_text" || content_type == "text" {
                        if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
                            texts.push(text.to_string());
                        }
                    }
                }
            }

            if let Some(calls) = item.get("tool_calls").and_then(|v| v.as_array()) {
                for call in calls {
                    if let Some(parsed) = parse_tool_call(call) {
                        tool_calls.push(parsed);
                    }
                }
            }
            continue;
        }

        if item_type == "tool_call" || item_type == "function_call" {
            if let Some(parsed) = parse_tool_call(item) {
                tool_calls.push(parsed);
            }
        }
    }

    if texts.is_empty() && tool_calls.is_empty() {
        return Err(AIError::ParseError(
            "No assistant output_text or tool calls in response".to_string(),
        ));
    }

    let usage_tokens = value
        .get("usage")
        .and_then(|u| u.get("total_tokens"))
        .and_then(|v| v.as_u64());
    Ok((texts.join(""), tool_calls, usage_tokens))
}

fn parse_tool_call(value: &Value) -> Option<ToolCall> {
    let call_id = value
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("call_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

    let (name, args) = if let Some(func) = value.get("function") {
        let name = func
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?;
        let args = func.get("arguments").cloned().unwrap_or(Value::Null);
        (name, args)
    } else {
        let name = value
            .get("name")
            .or_else(|| value.get("tool_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?;
        let args = value.get("arguments").cloned().unwrap_or(Value::Null);
        (name, args)
    };

    let arguments = if let Some(text) = args.as_str() {
        serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
    } else {
        args
    };

    Some(ToolCall {
        name,
        arguments,
        call_id,
    })
}
fn parse_chat_completion_with_usage(body: &str) -> Result<(String, Option<u64>), AIError> {
    let parsed: ChatCompletionsResponse = serde_json::from_str(body)
        .map_err(|e| AIError::ParseError(format!("Invalid chat completion response: {}", e)))?;

    let choice =
        parsed.choices.into_iter().next().ok_or_else(|| {
            AIError::ParseError("No choices in chat completion response".to_string())
        })?;

    let content = choice.message.content.ok_or_else(|| {
        AIError::ParseError("No message content in chat completion response".to_string())
    })?;

    let text = extract_chat_completion_text(content).ok_or_else(|| {
        AIError::ParseError("No text content in chat completion response".to_string())
    })?;

    let usage_tokens = parsed.usage.map(|u| u.total_tokens);
    Ok((text, usage_tokens))
}

fn extract_chat_completion_text(content: Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text),
        Value::Array(items) => {
            let mut texts = Vec::new();
            for item in items {
                match item {
                    Value::String(text) if !text.is_empty() => texts.push(text),
                    Value::Object(map) => {
                        if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                texts.push(text.to_string());
                            }
                            continue;
                        }
                        if let Some(text) = map.get("content").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                texts.push(text.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            if texts.is_empty() {
                None
            } else {
                Some(texts.join(""))
            }
        }
        Value::Object(map) => map
            .get("text")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .or_else(|| {
                map.get("content")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string())
            }),
        _ => None,
    }
}

fn should_fallback_to_chat_completions(status: StatusCode, message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    if status == StatusCode::NOT_IMPLEMENTED {
        return true;
    }
    if status == StatusCode::METHOD_NOT_ALLOWED {
        return true;
    }
    if status == StatusCode::NOT_FOUND && (m.contains("/responses") || m.contains("responses")) {
        return true;
    }
    if m.contains("does not support the responses api") {
        return true;
    }
    if m.contains("responses api") && m.contains("not implemented") {
        return true;
    }
    if m.contains("/responses") && m.contains("not found") {
        return true;
    }
    false
}

fn should_prefer_chat_completions(endpoint: &str, model: &str) -> bool {
    if model.contains(':') {
        return true;
    }

    let Ok(url) = Url::parse(endpoint) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    host != "api.openai.com"
}

fn should_fallback_on_transport_error(err: &reqwest::Error) -> bool {
    if err.is_timeout() || err.is_connect() {
        return false;
    }
    should_fallback_on_transport_message(&err.to_string())
}

fn should_fallback_on_transport_message(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("empty reply from server")
        || m.contains("incomplete message")
        || m.contains("unexpected eof")
        || m.contains("connection reset")
        || m.contains("connection closed")
        || m.contains("connection was closed")
}

/// Returns true if the error is retryable (rate limited or server error)
fn is_retryable(error: &AIError) -> bool {
    match error {
        AIError::RateLimited { .. } => true,
        AIError::ServerError(message) => !is_permanent_server_error(message),
        _ => false,
    }
}

fn is_permanent_server_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    // Some OpenAI-compatible backends return 501 for /responses. Retrying just adds long delays.
    if m.contains("http 501") {
        return true;
    }
    if m.contains("not implemented") {
        return true;
    }
    if m.contains("does not support the responses api") {
        return true;
    }
    false
}

/// Compute exponential backoff delay: 1s, 2s, 4s, 8s, 16s
fn backoff_delay(retry: usize) -> Duration {
    Duration::from_secs(BACKOFF_BASE_SECS << retry)
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
            cumulative_tokens: AtomicU64::new(0),
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
        AIError::IncompleteSummary => "Incomplete summary - retrying".to_string(),
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

    #[test]
    fn test_build_chat_completions_url_appends_path() {
        let url = build_chat_completions_url("https://api.openai.com/v1").unwrap();
        assert_eq!(url.as_str(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn test_build_chat_completions_url_already_has_path() {
        let url = build_chat_completions_url("https://api.openai.com/v1/chat/completions").unwrap();
        assert_eq!(url.as_str(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn test_build_chat_completions_url_strips_responses_path() {
        let url = build_chat_completions_url("https://api.openai.com/v1/responses").unwrap();
        assert_eq!(url.as_str(), "https://api.openai.com/v1/chat/completions");
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

    #[test]
    fn test_parse_chat_completion_with_usage_string_content() {
        let body = r#"{
            "choices": [
                {
                    "message": {
                        "content": "{\"suggestions\":[\"feature/a\",\"bugfix/b\",\"hotfix/c\"]}"
                    }
                }
            ],
            "usage": {"total_tokens": 12}
        }"#;
        let (text, usage) = parse_chat_completion_with_usage(body).unwrap();
        assert!(text.contains("\"suggestions\""));
        assert_eq!(usage, Some(12));
    }

    #[test]
    fn test_parse_chat_completion_with_usage_array_content() {
        let body = r#"{
            "choices": [
                {
                    "message": {
                        "content": [
                            {"type": "text", "text": "{\"suggestions\":["},
                            {"type": "text", "text": "\"feature/a\"]}"}
                        ]
                    }
                }
            ]
        }"#;
        let (text, usage) = parse_chat_completion_with_usage(body).unwrap();
        assert_eq!(text, "{\"suggestions\":[\"feature/a\"]}");
        assert_eq!(usage, None);
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
    // Timeout Constants Tests
    // ========================================

    #[test]
    fn test_request_timeout_constant() {
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(600));
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

    // ========================================
    // Retryable Error Tests
    // ========================================

    #[test]
    fn test_is_retryable_rate_limited() {
        let error = AIError::RateLimited {
            retry_after: Some(5),
        };
        assert!(is_retryable(&error));
    }

    #[test]
    fn test_is_retryable_rate_limited_no_retry_after() {
        let error = AIError::RateLimited { retry_after: None };
        assert!(is_retryable(&error));
    }

    #[test]
    fn test_is_retryable_server_error() {
        let error = AIError::ServerError("Internal Server Error".to_string());
        assert!(is_retryable(&error));
    }

    #[test]
    fn test_is_not_retryable_server_error_not_implemented() {
        let error = AIError::ServerError("Not Implemented".to_string());
        assert!(!is_retryable(&error));
    }

    #[test]
    fn test_is_not_retryable_server_error_responses_api_unsupported() {
        let error = AIError::ServerError(
            "Not Implemented: The backend for model 'qwen3-coder:30b' does not support the Responses API"
                .to_string(),
        );
        assert!(!is_retryable(&error));
    }

    #[test]
    fn test_is_not_retryable_server_error_http_501() {
        let error = AIError::ServerError("HTTP 501".to_string());
        assert!(!is_retryable(&error));
    }

    #[test]
    fn test_should_fallback_to_chat_completions_for_not_implemented() {
        assert!(should_fallback_to_chat_completions(
            StatusCode::NOT_IMPLEMENTED,
            "Not Implemented"
        ));
    }

    #[test]
    fn test_should_fallback_to_chat_completions_for_responses_api_error() {
        assert!(should_fallback_to_chat_completions(
            StatusCode::BAD_REQUEST,
            "The backend does not support the Responses API"
        ));
    }

    #[test]
    fn test_should_not_fallback_to_chat_completions_for_unrelated_error() {
        assert!(!should_fallback_to_chat_completions(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error"
        ));
    }

    #[test]
    fn test_should_fallback_on_transport_message_for_connection_closed() {
        assert!(should_fallback_on_transport_message(
            "error sending request for url: connection closed before message completed"
        ));
    }

    #[test]
    fn test_should_not_fallback_on_transport_message_for_timeout() {
        assert!(!should_fallback_on_transport_message("operation timed out"));
    }

    #[test]
    fn test_should_prefer_chat_completions_for_local_endpoint() {
        assert!(should_prefer_chat_completions(
            "http://localhost:11434/v1",
            "gpt-oss:20b"
        ));
    }

    #[test]
    fn test_should_prefer_chat_completions_for_non_openai_host() {
        assert!(should_prefer_chat_completions(
            "https://openrouter.ai/api/v1",
            "gpt-4o-mini"
        ));
    }

    #[test]
    fn test_should_not_prefer_chat_completions_for_official_openai() {
        assert!(!should_prefer_chat_completions(
            "https://api.openai.com/v1",
            "gpt-4o-mini"
        ));
    }

    #[test]
    fn test_should_not_prefer_chat_completions_for_invalid_endpoint() {
        assert!(!should_prefer_chat_completions("not-a-url", "gpt-4o-mini"));
    }

    #[test]
    fn test_preferred_chat_completion_reasoning_effort_for_local_model() {
        assert_eq!(
            preferred_chat_completion_reasoning_effort("http://192.168.100.235:32768/v1", "gpt-oss:20b"),
            Some("low")
        );
    }

    #[test]
    fn test_preferred_chat_completion_reasoning_effort_none_for_openai() {
        assert_eq!(
            preferred_chat_completion_reasoning_effort("https://api.openai.com/v1", "gpt-oss:20b"),
            None
        );
    }

    #[test]
    fn test_preferred_chat_completion_reasoning_effort_none_for_official_model_name() {
        assert_eq!(
            preferred_chat_completion_reasoning_effort("https://api.openai.com/v1", "gpt-4o-mini"),
            None
        );
    }

    #[test]
    fn test_should_retry_chat_completion_without_reasoning_parse_error() {
        assert!(should_retry_chat_completion_without_reasoning(
            &AIError::ParseError("No message content in chat completion response".to_string())
        ));
        assert!(!should_retry_chat_completion_without_reasoning(&AIError::ParseError(
            "bad json".to_string()
        )));
    }

    #[test]
    fn test_is_not_retryable_unauthorized() {
        let error = AIError::Unauthorized;
        assert!(!is_retryable(&error));
    }

    #[test]
    fn test_is_not_retryable_parse_error() {
        let error = AIError::ParseError("bad json".to_string());
        assert!(!is_retryable(&error));
    }

    #[test]
    fn test_is_not_retryable_network_error() {
        let error = AIError::NetworkError("timeout".to_string());
        assert!(!is_retryable(&error));
    }

    #[test]
    fn test_is_not_retryable_config_error() {
        let error = AIError::ConfigError("missing key".to_string());
        assert!(!is_retryable(&error));
    }

    #[test]
    fn test_is_not_retryable_incomplete_summary() {
        let error = AIError::IncompleteSummary;
        assert!(!is_retryable(&error));
    }

    // ========================================
    // Backoff Delay Tests
    // ========================================

    #[test]
    fn test_backoff_delay_sequence() {
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(2), Duration::from_secs(4));
        assert_eq!(backoff_delay(3), Duration::from_secs(8));
        assert_eq!(backoff_delay(4), Duration::from_secs(16));
    }

    // ========================================
    // Usage Parsing Tests
    // ========================================

    #[test]
    fn test_parse_response_with_usage_tokens() {
        let body = r#"{
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello"}]
            }],
            "usage": {"total_tokens": 42}
        }"#;
        let (text, usage) = parse_response_with_usage(body).unwrap();
        assert_eq!(text, "Hello");
        assert_eq!(usage, Some(42));
    }

    #[test]
    fn test_parse_response_with_no_usage() {
        let body = r#"{
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hi"}]
            }]
        }"#;
        let (text, usage) = parse_response_with_usage(body).unwrap();
        assert_eq!(text, "Hi");
        assert_eq!(usage, None);
    }

    #[test]
    fn test_parse_response_with_tools() {
        let body = r#"{
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Starting."}],
                "tool_calls": [{
                    "id": "call_1",
                    "name": "send_keys_to_pane",
                    "arguments": "{\"pane_id\":\"pane-1\",\"text\":\"ls\\n\"}"
                }]
            }],
            "usage": {"total_tokens": 42}
        }"#;
        let (text, calls, usage) = parse_response_with_tools(body).unwrap();
        assert_eq!(text, "Starting.");
        assert_eq!(usage, Some(42));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "send_keys_to_pane");
        assert_eq!(
            calls[0].arguments.get("pane_id").and_then(|v| v.as_str()),
            Some("pane-1")
        );
    }

    // ========================================
    // Cumulative Tokens Tests
    // ========================================

    #[test]
    fn test_cumulative_tokens_initial_value() {
        let client = AIClient::new_for_list_models("https://api.openai.com/v1", "key").unwrap();
        assert_eq!(client.cumulative_tokens(), 0);
    }
}
