//! `/v1/models` probe client for OpenAI-compatible upstreams (including
//! Anthropic Messages API compatible proxies that expose the OpenAI model
//! listing endpoint).
//!
//! Supports SPEC-1921 FR-061: Settings > Custom Agents > Add from preset >
//! "Claude Code (OpenAI-compat backend)" saves only after a
//! `GET {base_url}/v1/models` call returns HTTP 200 with parseable JSON
//! containing `data[].id`.

use std::time::Duration;

use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
};
use serde::{Deserialize, Serialize};

/// Validate that `base_url` uses the `http://` or `https://` scheme.
/// SPEC-1921 FR-060. Accepts leading/trailing whitespace and is case-insensitive
/// on the scheme portion only.
pub fn is_valid_base_url(base_url: &str) -> bool {
    let lower = base_url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Connect + read timeout for the `/v1/models` probe. SPEC-1921 FR-061.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// A single model entry returned by `/v1/models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model ID. Callers should treat this string as opaque.
    pub id: String,
}

/// Structured error taxonomy for the `/v1/models` probe.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProbeError {
    /// `base_url` could not be parsed or has a non-http(s) scheme.
    #[error("invalid base_url: {0}")]
    InvalidUrl(String),

    /// Connect or read timed out (>= `PROBE_TIMEOUT`).
    #[error("probe timed out after {0:?}")]
    Timeout(Duration),

    /// Upstream returned a non-2xx status.
    #[error("http status {code}: {body}")]
    HttpStatus {
        /// HTTP status code.
        code: u16,
        /// Truncated body string for diagnostic use.
        body: String,
    },

    /// Response body was not valid JSON.
    #[error("invalid JSON: {0}")]
    InvalidJson(String),

    /// JSON did not contain a `data` array, or an entry was missing `id`.
    #[error("response missing `data[].id`")]
    MissingData,

    /// Low-level transport failure (DNS, TLS, connection refused, etc.).
    #[error("transport error: {0}")]
    Transport(String),
}

/// Internal wire type for `{ "data": [ { "id": "..." }, ... ] }`.
#[derive(Debug, Deserialize)]
struct ModelsResponseWire {
    data: Option<Vec<ModelEntryWire>>,
}

#[derive(Debug, Deserialize)]
struct ModelEntryWire {
    id: Option<String>,
}

/// Parse a `/v1/models` response body.
///
/// Returns all discovered `data[].id` entries in document order. An empty
/// `data` array yields an empty `Vec`. An entry without `id` (or a payload
/// without `data`) yields `ProbeError::MissingData`.
pub fn parse_models_response(body: &str) -> Result<Vec<ModelInfo>, ProbeError> {
    let parsed: ModelsResponseWire =
        serde_json::from_str(body).map_err(|e| ProbeError::InvalidJson(e.to_string()))?;
    let Some(data) = parsed.data else {
        return Err(ProbeError::MissingData);
    };
    let mut out = Vec::with_capacity(data.len());
    for entry in data {
        let Some(id) = entry.id else {
            return Err(ProbeError::MissingData);
        };
        out.push(ModelInfo { id });
    }
    Ok(out)
}

/// Validate the `base_url` scheme and return the structured probe error on
/// failure. Thin wrapper over [`is_valid_base_url`].
fn validate_base_url(base_url: &str) -> Result<(), ProbeError> {
    if is_valid_base_url(base_url) {
        Ok(())
    } else {
        Err(ProbeError::InvalidUrl(format!(
            "base_url must start with http:// or https://, got: {base_url}"
        )))
    }
}

/// Build the `/v1/models` URL from `base_url`.
fn build_models_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    format!("{trimmed}/v1/models")
}

/// Blocking `GET {base_url}/v1/models` call.
///
/// Uses a 3-second connect+read timeout (FR-061) and no retry. Returns the
/// parsed list of models, or a structured `ProbeError`. Callers in the
/// Settings UI use the return value to populate the default_model dropdown
/// and to gate the form's Save button.
pub fn list_models_blocking(base_url: &str, api_key: &str) -> Result<Vec<ModelInfo>, ProbeError> {
    validate_base_url(base_url)?;
    let url = build_models_url(base_url);

    let mut headers = HeaderMap::new();
    if !api_key.is_empty() {
        let header_value = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|e| ProbeError::InvalidUrl(format!("invalid api_key header: {e}")))?;
        headers.insert(AUTHORIZATION, header_value);
    }

    let client = Client::builder()
        .timeout(PROBE_TIMEOUT)
        .connect_timeout(PROBE_TIMEOUT)
        .default_headers(headers)
        .build()
        .map_err(|e| ProbeError::Transport(e.to_string()))?;

    let response = client.get(&url).send().map_err(map_reqwest_error)?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|e| ProbeError::Transport(e.to_string()))?;

    if !status.is_success() {
        return Err(ProbeError::HttpStatus {
            code: status.as_u16(),
            body: truncate_for_diagnostic(&body),
        });
    }

    parse_models_response(&body)
}

fn map_reqwest_error(err: reqwest::Error) -> ProbeError {
    if err.is_timeout() {
        return ProbeError::Timeout(PROBE_TIMEOUT);
    }
    if let Some(status) = err.status() {
        return ProbeError::HttpStatus {
            code: status.as_u16(),
            body: err.to_string(),
        };
    }
    if err.is_connect() || err.is_request() {
        return ProbeError::Transport(err.to_string());
    }
    ProbeError::Transport(err.to_string())
}

fn truncate_for_diagnostic(body: &str) -> String {
    const MAX: usize = 512;
    if body.len() <= MAX {
        return body.to_string();
    }
    // Walk backwards from MAX to find the nearest char boundary so that
    // multi-byte UTF-8 upstream bodies (e.g. Japanese error messages) do
    // not cause `body[..MAX]` to panic at a mid-scalar byte split.
    let mut cut = MAX;
    while cut > 0 && !body.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + 16);
    out.push_str(&body[..cut]);
    out.push_str("...<truncated>");
    out
}

/// Shorthand that lists models and returns only the `id` strings. Useful for
/// wiring directly into UI dropdowns.
pub fn list_model_ids_blocking(base_url: &str, api_key: &str) -> Result<Vec<String>, ProbeError> {
    list_models_blocking(base_url, api_key).map(|ms| ms.into_iter().map(|m| m.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_response_returns_ids_in_order() {
        let body = r#"{"data":[{"id":"openai/gpt-oss-20b"},{"id":"openai/gpt-oss-120b"}]}"#;
        let models = parse_models_response(body).expect("should parse");
        assert_eq!(
            models,
            vec![
                ModelInfo {
                    id: "openai/gpt-oss-20b".to_string(),
                },
                ModelInfo {
                    id: "openai/gpt-oss-120b".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_empty_data_array_returns_empty_vec() {
        let body = r#"{"data":[]}"#;
        let models = parse_models_response(body).expect("should parse");
        assert!(models.is_empty());
    }

    #[test]
    fn parse_missing_data_field_is_missing_data_error() {
        let body = r#"{"object":"list"}"#;
        let err = parse_models_response(body).unwrap_err();
        assert_eq!(err, ProbeError::MissingData);
    }

    #[test]
    fn parse_entry_missing_id_is_missing_data_error() {
        let body = r#"{"data":[{"id":"a"},{"object":"model"}]}"#;
        let err = parse_models_response(body).unwrap_err();
        assert_eq!(err, ProbeError::MissingData);
    }

    #[test]
    fn parse_invalid_json_is_invalid_json_error() {
        let body = "<html><body>Not Found</body></html>";
        let err = parse_models_response(body).unwrap_err();
        assert!(matches!(err, ProbeError::InvalidJson(_)));
    }

    #[test]
    fn parse_ignores_extra_fields() {
        let body = r#"{"object":"list","data":[{"id":"m1","object":"model","created":123}]}"#;
        let models = parse_models_response(body).expect("should parse");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "m1");
    }

    #[test]
    fn validate_base_url_accepts_http_and_https() {
        assert!(validate_base_url("http://192.168.100.166:32768").is_ok());
        assert!(validate_base_url("https://api.example.com").is_ok());
        assert!(validate_base_url("HTTP://upper.case").is_ok());
        assert!(validate_base_url("  https://trimmed.com  ").is_ok());
    }

    #[test]
    fn validate_base_url_rejects_other_schemes() {
        assert!(matches!(
            validate_base_url("ws://example.com"),
            Err(ProbeError::InvalidUrl(_))
        ));
        assert!(matches!(
            validate_base_url("file:///etc/passwd"),
            Err(ProbeError::InvalidUrl(_))
        ));
        assert!(matches!(
            validate_base_url("no-scheme.example"),
            Err(ProbeError::InvalidUrl(_))
        ));
    }

    #[test]
    fn build_models_url_strips_trailing_slash() {
        assert_eq!(
            build_models_url("http://host:1234/"),
            "http://host:1234/v1/models"
        );
        assert_eq!(
            build_models_url("http://host:1234"),
            "http://host:1234/v1/models"
        );
        assert_eq!(
            build_models_url("https://a.b/c/"),
            "https://a.b/c/v1/models"
        );
    }

    #[test]
    fn list_models_blocking_rejects_invalid_scheme() {
        let err =
            list_models_blocking("ws://example.com", "k").expect_err("should reject ws scheme");
        assert!(matches!(err, ProbeError::InvalidUrl(_)));
    }

    #[test]
    fn truncate_for_diagnostic_caps_long_bodies() {
        let long = "a".repeat(1024);
        let truncated = truncate_for_diagnostic(&long);
        assert!(truncated.len() <= 540);
        assert!(truncated.ends_with("...<truncated>"));
    }

    #[test]
    fn truncate_for_diagnostic_leaves_short_bodies_alone() {
        let body = "short body";
        assert_eq!(truncate_for_diagnostic(body), "short body");
    }

    #[test]
    fn truncate_for_diagnostic_preserves_char_boundary_with_multibyte_utf8() {
        // 3-byte chars (Japanese) at byte ~512 would panic under
        // `body[..512]` if byte 512 lands mid-scalar. The helper must
        // round down to a char boundary.
        let body: String = "あ".repeat(1024);
        let truncated = truncate_for_diagnostic(&body);
        assert!(truncated.ends_with("...<truncated>"));
        // The prefix must still be a valid UTF-8 string (confirmed by
        // the fact that `truncated` was built via push_str of a &str slice).
        assert!(truncated.starts_with("あ"));
    }

    #[test]
    fn truncate_for_diagnostic_handles_mixed_width_utf8_at_cap() {
        // Force a scenario where MAX=512 lands in the middle of a 4-byte char.
        // 510 ASCII + U+1F600 (😀, 4 bytes) repeated — byte 512 is inside the
        // emoji.
        let mut body = String::with_capacity(600);
        for _ in 0..510 {
            body.push('a');
        }
        for _ in 0..20 {
            body.push('😀');
        }
        let truncated = truncate_for_diagnostic(&body);
        // Must not panic; must be valid UTF-8 (enforced by Rust type system
        // once the function returns String). Verify truncation happened.
        assert!(truncated.ends_with("...<truncated>"));
        assert!(truncated.len() < body.len() + 16);
    }
}
