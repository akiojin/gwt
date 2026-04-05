//! OpenAI Responses API client with retry and rate-limit handling.

use std::time::Duration;

use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::error::AIError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT_TOKENS: u32 = 1024;
const TEMPERATURE: f32 = 0.3;

const MAX_RETRIES: usize = 3;
const BACKOFF_BASE_MS: u64 = 500;

/// A single chat message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    /// One of `"system"`, `"user"`, or `"assistant"`.
    pub role: String,
    /// The text content of the message.
    pub content: String,
}

// ── Request / Response wire types ──────────────────────────────────────

#[derive(Debug, Serialize)]
struct ResponsesRequest<'a> {
    model: &'a str,
    input: Vec<serde_json::Value>,
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
    content: Option<Vec<ResponseOutputContent>>,
}

#[derive(Debug, Deserialize)]
struct ResponseOutputContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

// ── AIClient ───────────────────────────────────────────────────────────

/// Blocking client for the OpenAI Responses API.
#[derive(Debug)]
pub struct AIClient {
    endpoint: String,
    api_key: String,
    model: String,
    client: Client,
}

impl AIClient {
    /// Create a new client from explicit parameters.
    ///
    /// # Errors
    /// Returns [`AIError::ConfigError`] if `endpoint` or `model` is empty.
    pub fn new(endpoint: &str, api_key: &str, model: &str) -> Result<Self, AIError> {
        let endpoint = endpoint.trim().to_string();
        let model = model.trim().to_string();
        if endpoint.is_empty() {
            return Err(AIError::ConfigError("API endpoint is empty".into()));
        }
        if model.is_empty() {
            return Err(AIError::ConfigError("Model is empty".into()));
        }

        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| AIError::NetworkError(e.to_string()))?;

        Ok(Self {
            endpoint,
            api_key: api_key.to_string(),
            model,
            client,
        })
    }

    /// Send chat messages via the Responses API and return the assistant's text.
    ///
    /// Retries transient failures (429, 5xx) with exponential back-off up to
    /// [`MAX_RETRIES`] times.
    pub fn create_response(&self, messages: Vec<ChatMessage>) -> Result<String, AIError> {
        if messages.is_empty() {
            return Err(AIError::ConfigError("No input messages".into()));
        }

        let url = build_responses_url(&self.endpoint)?;
        let (instructions, input) = build_input(&messages);
        let body = ResponsesRequest {
            model: &self.model,
            input,
            instructions,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            temperature: TEMPERATURE,
        };

        let raw = self.send_with_retry(&url, &body)?;
        parse_text(&raw)
    }

    // ── internal helpers ───────────────────────────────────────────────

    fn send_with_retry(&self, url: &str, body: &impl Serialize) -> Result<String, AIError> {
        let mut retries = 0usize;

        loop {
            let headers = self.auth_headers();
            let result = self.client.post(url).headers(headers).json(body).send();

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    let resp_headers = resp.headers().clone();
                    let text = resp.text().unwrap_or_default();

                    if status == StatusCode::OK {
                        return Ok(text);
                    }

                    if status == StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = resp_headers
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok());

                        if retries >= MAX_RETRIES {
                            return Err(AIError::RateLimited {
                                retry_after_secs: retry_after,
                            });
                        }
                        let wait = retry_after.unwrap_or(backoff_ms(retries) / 1000 + 1);
                        std::thread::sleep(Duration::from_secs(wait));
                        retries += 1;
                        continue;
                    }

                    if status.is_server_error() {
                        if retries >= MAX_RETRIES {
                            return Err(AIError::ServerError(format!("{status}: {text}")));
                        }
                        std::thread::sleep(Duration::from_millis(backoff_ms(retries)));
                        retries += 1;
                        continue;
                    }

                    // Non-retryable client error
                    return Err(AIError::ServerError(format!("{status}: {text}")));
                }
                Err(e) => {
                    if e.is_timeout() {
                        return Err(AIError::Timeout(e.to_string()));
                    }
                    if retries >= MAX_RETRIES {
                        return Err(AIError::NetworkError(e.to_string()));
                    }
                    std::thread::sleep(Duration::from_millis(backoff_ms(retries)));
                    retries += 1;
                }
            }
        }
    }

    fn auth_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if !self.api_key.is_empty() {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
                headers.insert(AUTHORIZATION, val);
            }
        }
        headers
    }
}

// ── Free helpers ───────────────────────────────────────────────────────

fn backoff_ms(retry: usize) -> u64 {
    BACKOFF_BASE_MS * 2u64.pow(retry as u32)
}

fn build_responses_url(endpoint: &str) -> Result<String, AIError> {
    let base = endpoint.trim_end_matches('/');
    Ok(format!("{base}/responses"))
}

/// Split messages into (optional system instruction, input array).
fn build_input(messages: &[ChatMessage]) -> (Option<String>, Vec<serde_json::Value>) {
    let mut instructions: Option<String> = None;
    let mut input = Vec::new();

    for msg in messages {
        if msg.role == "system" {
            instructions = Some(msg.content.clone());
        } else {
            input.push(serde_json::json!({
                "role": msg.role,
                "content": msg.content,
            }));
        }
    }

    (instructions, input)
}

fn parse_text(raw: &str) -> Result<String, AIError> {
    let resp: ResponsesResponse = serde_json::from_str(raw)
        .map_err(|e| AIError::ParseError(format!("Invalid JSON response: {e}")))?;

    for item in &resp.output {
        if item.item_type == "message" {
            if let Some(contents) = &item.content {
                for c in contents {
                    if c.content_type == "output_text" {
                        if let Some(text) = &c.text {
                            return Ok(text.clone());
                        }
                    }
                }
            }
        }
    }

    Err(AIError::ParseError(
        "No output text found in response".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── constructor ────────────────────────────────────────────────────

    #[test]
    fn new_rejects_empty_endpoint() {
        let err = AIClient::new("", "key", "model").unwrap_err();
        assert!(matches!(err, AIError::ConfigError(_)));
    }

    #[test]
    fn new_rejects_empty_model() {
        let err = AIClient::new("https://api.example.com", "key", "").unwrap_err();
        assert!(matches!(err, AIError::ConfigError(_)));
    }

    #[test]
    fn new_accepts_empty_api_key() {
        // Some endpoints may not require authentication.
        let client = AIClient::new("https://api.example.com", "", "gpt-4o").unwrap();
        assert_eq!(client.model, "gpt-4o");
    }

    // ── create_response validation ─────────────────────────────────────

    #[test]
    fn create_response_rejects_empty_messages() {
        let client = AIClient::new("https://api.example.com", "k", "m").unwrap();
        let err = client.create_response(vec![]).unwrap_err();
        assert!(matches!(err, AIError::ConfigError(_)));
    }

    // ── build_input ────────────────────────────────────────────────────

    #[test]
    fn build_input_extracts_system_instruction() {
        let msgs = vec![
            ChatMessage {
                role: "system".into(),
                content: "Be concise.".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "Hello".into(),
            },
        ];
        let (instr, input) = build_input(&msgs);
        assert_eq!(instr.unwrap(), "Be concise.");
        assert_eq!(input.len(), 1);
    }

    #[test]
    fn build_input_without_system() {
        let msgs = vec![ChatMessage {
            role: "user".into(),
            content: "Hi".into(),
        }];
        let (instr, input) = build_input(&msgs);
        assert!(instr.is_none());
        assert_eq!(input.len(), 1);
    }

    // ── parse_text ─────────────────────────────────────────────────────

    #[test]
    fn parse_text_extracts_output() {
        let raw = r#"{
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }]
        }"#;
        assert_eq!(parse_text(raw).unwrap(), "Hello!");
    }

    #[test]
    fn parse_text_fails_on_empty_output() {
        let raw = r#"{"output": []}"#;
        let err = parse_text(raw).unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    #[test]
    fn parse_text_fails_on_invalid_json() {
        let err = parse_text("not json").unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    // ── build_responses_url ────────────────────────────────────────────

    #[test]
    fn build_responses_url_appends_path() {
        let url = build_responses_url("https://api.openai.com/v1").unwrap();
        assert_eq!(url, "https://api.openai.com/v1/responses");
    }

    #[test]
    fn build_responses_url_strips_trailing_slash() {
        let url = build_responses_url("https://api.openai.com/v1/").unwrap();
        assert_eq!(url, "https://api.openai.com/v1/responses");
    }

    // ── backoff_ms ─────────────────────────────────────────────────────

    #[test]
    fn backoff_ms_grows_exponentially() {
        assert_eq!(backoff_ms(0), 500);
        assert_eq!(backoff_ms(1), 1000);
        assert_eq!(backoff_ms(2), 2000);
    }
}
