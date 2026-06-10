//! SPEC-3014 FR-005: integration tests for gwt-ai response handling against
//! a local HTTP mock (std `TcpListener`, no extra dependencies).
//!
//! Covered surfaces: `AIClient::create_response` (Responses API), issue
//! classification, branch suggestion, and the `/v1/models` probe — each with
//! a happy path plus malformed-JSON / empty-or-missing-field / invalid-token
//! edge cases. `AIClient::new` already accepts an injectable endpoint, so no
//! production change is needed to point the client at the mock.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;

use gwt_ai::{
    classify_issue, list_models_blocking, suggest_branch_name, AIClient, AIError, ChatMessage,
    ProbeError,
};

struct MockResponse {
    /// Full HTTP status line tail, e.g. `"200 OK"`.
    status: &'static str,
    body: String,
}

impl MockResponse {
    fn ok(body: impl Into<String>) -> Self {
        Self {
            status: "200 OK",
            body: body.into(),
        }
    }
}

/// Serve `responses` sequentially on an ephemeral port. Returns the base URL
/// and a handle whose join yields the raw captured requests (status line +
/// headers + body) in arrival order.
fn serve(responses: Vec<MockResponse>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("local addr");
    let handle = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().expect("accept connection");
            captured.push(read_http_request(&mut stream));
            let reply = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status,
                response.body.len(),
                response.body
            );
            stream.write_all(reply.as_bytes()).expect("write response");
        }
        captured
    });
    (format!("http://{addr}"), handle)
}

/// Read one HTTP/1.1 request: headers plus a `Content-Length` body (if any).
fn read_http_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(header_end) = find_subsequence(&buf, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.trim().eq_ignore_ascii_case("content-length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            if buf.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Wrap `text` in a minimal OpenAI Responses API success payload.
fn responses_body(text: &str) -> String {
    serde_json::json!({
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": text }]
        }]
    })
    .to_string()
}

fn user_message(content: &str) -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: "user".to_string(),
        content: content.to_string(),
    }]
}

// ── AIClient::create_response ───────────────────────────────────────────

#[test]
fn create_response_returns_assistant_text_and_sends_auth_header() {
    let (base, server) = serve(vec![MockResponse::ok(responses_body("Hello back!"))]);
    let client = AIClient::new(&base, "test-key", "test-model").expect("client");

    let text = client
        .create_response(user_message("Hello there"))
        .expect("create_response");
    assert_eq!(text, "Hello back!");

    let captured = server.join().expect("server thread");
    assert_eq!(captured.len(), 1, "exactly one request, no retries");
    let request = captured[0].to_lowercase();
    assert!(
        request.starts_with("post /responses http/1.1"),
        "got: {}",
        &captured[0][..captured[0].len().min(120)]
    );
    assert!(
        request.contains("authorization: bearer test-key"),
        "Authorization header must carry the API key"
    );
    assert!(request.contains("test-model"), "model must be in the body");
    assert!(
        request.contains("hello there"),
        "user message must be in the body"
    );
}

#[test]
fn create_response_malformed_json_body_is_parse_error() {
    let (base, _server) = serve(vec![MockResponse::ok("this is {not json")]);
    let client = AIClient::new(&base, "k", "m").expect("client");

    let err = client.create_response(user_message("hi")).unwrap_err();
    assert!(matches!(err, AIError::ParseError(_)), "got: {err:?}");
}

#[test]
fn create_response_empty_output_array_is_parse_error() {
    let (base, _server) = serve(vec![MockResponse::ok(r#"{"output":[]}"#)]);
    let client = AIClient::new(&base, "k", "m").expect("client");

    let err = client.create_response(user_message("hi")).unwrap_err();
    match err {
        AIError::ParseError(message) => {
            assert!(message.contains("No output text"), "got: {message}")
        }
        other => panic!("expected ParseError, got {other:?}"),
    }
}

#[test]
fn create_response_client_error_status_fails_without_retry() {
    let (base, server) = serve(vec![MockResponse {
        status: "404 Not Found",
        body: r#"{"error":"no such route"}"#.to_string(),
    }]);
    let client = AIClient::new(&base, "k", "m").expect("client");

    let err = client.create_response(user_message("hi")).unwrap_err();
    match err {
        AIError::ServerError(message) => assert!(message.contains("404"), "got: {message}"),
        other => panic!("expected ServerError, got {other:?}"),
    }

    let captured = server.join().expect("server thread");
    assert_eq!(captured.len(), 1, "4xx must not be retried");
}

// ── issue classification ────────────────────────────────────────────────

#[test]
fn classify_issue_maps_model_reply_to_branch_prefix() {
    let (base, _server) = serve(vec![MockResponse::ok(responses_body("bugfix"))]);
    let client = AIClient::new(&base, "k", "m").expect("client");

    let prefix = classify_issue(&client, "App crashes on startup", "stack trace ...")
        .expect("classify issue");
    assert_eq!(prefix, "bugfix");
}

#[test]
fn classify_issue_rejects_invalid_classification_token() {
    let (base, _server) = serve(vec![MockResponse::ok(responses_body("banana"))]);
    let client = AIClient::new(&base, "k", "m").expect("client");

    let err = client_classify_err(&client);
    assert!(matches!(err, AIError::ParseError(_)), "got: {err:?}");
}

fn client_classify_err(client: &AIClient) -> AIError {
    classify_issue(client, "Some title", "Some body").unwrap_err()
}

// ── branch suggestion ───────────────────────────────────────────────────

#[test]
fn suggest_branch_name_returns_validated_candidates() {
    let suggestions = r#"{"suggestions": ["feature/add-auth", "bad name!", "bugfix/fix-crash", "hotfix/patch-1"]}"#;
    let (base, _server) = serve(vec![MockResponse::ok(responses_body(suggestions))]);
    let client = AIClient::new(&base, "k", "m").expect("client");

    let names = suggest_branch_name(&client, "Add authentication to the API")
        .expect("suggest branch names");
    assert_eq!(
        names,
        vec!["feature/add-auth", "bugfix/fix-crash", "hotfix/patch-1"],
        "invalid candidates must be filtered out"
    );
}

#[test]
fn suggest_branch_name_fails_when_reply_has_no_json() {
    let (base, _server) = serve(vec![MockResponse::ok(responses_body(
        "sorry, no suggestions today",
    ))]);
    let client = AIClient::new(&base, "k", "m").expect("client");

    let err = suggest_branch_name(&client, "context").unwrap_err();
    assert!(matches!(err, AIError::ParseError(_)), "got: {err:?}");
}

// ── /v1/models probe ────────────────────────────────────────────────────

#[test]
fn models_probe_lists_model_ids_from_data_array() {
    let (base, server) = serve(vec![MockResponse::ok(
        r#"{"object":"list","data":[{"id":"openai/gpt-oss-20b"},{"id":"openai/gpt-oss-120b"}]}"#,
    )]);

    let models = list_models_blocking(&base, "probe-key").expect("probe models");
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["openai/gpt-oss-20b", "openai/gpt-oss-120b"]);

    let captured = server.join().expect("server thread");
    let request = captured[0].to_lowercase();
    assert!(
        request.starts_with("get /v1/models http/1.1"),
        "got: {}",
        &captured[0][..captured[0].len().min(120)]
    );
    assert!(request.contains("authorization: bearer probe-key"));
}

#[test]
fn models_probe_missing_data_field_is_missing_data_error() {
    let (base, _server) = serve(vec![MockResponse::ok(r#"{"object":"list"}"#)]);

    let err = list_models_blocking(&base, "").unwrap_err();
    assert_eq!(err, ProbeError::MissingData);
}

#[test]
fn models_probe_malformed_json_body_is_invalid_json_error() {
    let (base, _server) = serve(vec![MockResponse::ok("<html><body>oops</body></html>")]);

    let err = list_models_blocking(&base, "").unwrap_err();
    assert!(matches!(err, ProbeError::InvalidJson(_)), "got: {err:?}");
}

#[test]
fn models_probe_http_error_status_carries_code_and_body() {
    let (base, _server) = serve(vec![MockResponse {
        status: "401 Unauthorized",
        body: r#"{"error":"bad key"}"#.to_string(),
    }]);

    let err = list_models_blocking(&base, "wrong-key").unwrap_err();
    match err {
        ProbeError::HttpStatus { code, body } => {
            assert_eq!(code, 401);
            assert!(body.contains("bad key"), "got: {body}");
        }
        other => panic!("expected HttpStatus, got {other:?}"),
    }
}
