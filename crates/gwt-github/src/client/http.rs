//! Real HTTPS-backed [`IssueClient`] implementation.
//!
//! The implementation is split into two layers so we can keep TDD fast and
//! predictable while still shipping a real network client:
//!
//! - [`HttpTransport`] is an abstract request executor. A single method
//!   `execute` takes an [`HttpRequest`] and returns an [`HttpResponse`]. This
//!   is the seam that tests inject with a [`FakeTransport`] that records the
//!   request shape and returns canned responses.
//! - [`ReqwestTransport`] is the production implementation backed by the
//!   blocking `reqwest` client with rustls.
//!
//! [`HttpIssueClient`] composes a transport with repo coordinates (owner,
//! name) and an authentication token to implement every method of
//! [`IssueClient`]. Fetches and list queries go through the GraphQL endpoint
//! so one network round-trip covers "issue body + every comment"; mutations
//! go through the REST endpoints.

use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::client::{
    ApiError, CollectionGeneration, CommentId, CommentSnapshot, CommitComparison,
    CommitComparisonStatus, CompleteCollection, CreateRepositoryIssue, FetchResult, IssueClient,
    IssueNumber, IssueSnapshot, IssueState, MergedPullRequest, OwnerMutationError,
    OwnerMutationResult, OwnerRepositoryClient, RepositoryActorType, RepositoryAuthorAssociation,
    RepositoryComment, RepositoryIdentity, RepositoryIssue, RepositoryIssueKind, RepositoryRelease,
    ResolutionDeadline, SpecListFilter, SpecSummary, UpdatedAt,
};

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Patch,
    Delete,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
        }
    }
}

/// Outbound HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// Inbound HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Transport errors surfaced by [`HttpTransport::execute`].
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("transport error before request submission: {0}")]
    PreSubmitTransport(String),
    #[error("transport timeout before request submission: {0}")]
    PreSubmitTimeout(String),
    #[error("transport timeout: {0}")]
    Timeout(String),
}

/// Abstract HTTP executor used by [`HttpIssueClient`].
pub trait HttpTransport: Send + Sync {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError>;

    fn execute_with_deadline(
        &self,
        request: HttpRequest,
        deadline: &ResolutionDeadline,
    ) -> Result<HttpResponse, HttpError>;
}

// ---------------------------------------------------------------------------
// FakeTransport: records requests, returns canned responses in order
// ---------------------------------------------------------------------------

/// A deterministic fake [`HttpTransport`] used in tests.
///
/// The fake records every executed [`HttpRequest`] in insertion order and
/// returns pre-seeded [`HttpResponse`]s one at a time from a queue. Tests can
/// then assert both the exact request shape and the number of calls.
pub struct FakeTransport {
    state: Mutex<FakeState>,
}

struct FakeState {
    recorded: Vec<HttpRequest>,
    recorded_deadlines: Vec<Instant>,
    canned: std::collections::VecDeque<HttpResponse>,
}

impl FakeTransport {
    pub fn new() -> Self {
        FakeTransport {
            state: Mutex::new(FakeState {
                recorded: Vec::new(),
                recorded_deadlines: Vec::new(),
                canned: std::collections::VecDeque::new(),
            }),
        }
    }

    /// Queue a response to be returned by the next `execute` call.
    pub fn enqueue(&self, response: HttpResponse) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .canned
            .push_back(response);
    }

    /// Snapshot of every recorded request so far.
    pub fn recorded(&self) -> Vec<HttpRequest> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recorded
            .clone()
    }

    pub fn recorded_deadlines(&self) -> Vec<Instant> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recorded_deadlines
            .clone()
    }
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for FakeTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.recorded.push(request);
        state
            .canned
            .pop_front()
            .ok_or_else(|| HttpError::Transport("no canned response available".into()))
    }

    fn execute_with_deadline(
        &self,
        request: HttpRequest,
        deadline: &ResolutionDeadline,
    ) -> Result<HttpResponse, HttpError> {
        deadline
            .remaining("github http request")
            .map_err(|error| HttpError::PreSubmitTimeout(error.to_string()))?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.recorded.push(request);
        state.recorded_deadlines.push(deadline.expires_at());
        state
            .canned
            .pop_front()
            .ok_or_else(|| HttpError::Transport("no canned response available".into()))
    }
}

// ---------------------------------------------------------------------------
// ReqwestTransport: production backend
// ---------------------------------------------------------------------------

/// Production [`HttpTransport`] backed by `reqwest` blocking.
pub struct ReqwestTransport {
    standard_client: Arc<reqwest::blocking::Client>,
    strict_client: Arc<reqwest::blocking::Client>,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, HttpError> {
        Ok(Self {
            standard_client: Arc::new(build_reqwest_client(Duration::from_secs(5))?),
            strict_client: Arc::new(build_reqwest_client(Duration::from_secs(3))?),
        })
    }

    fn deadline_client(
        &self,
        deadline: &ResolutionDeadline,
    ) -> Result<Arc<reqwest::blocking::Client>, HttpError> {
        let connect_timeout = deadline
            .connect_timeout("github http connect")
            .map_err(|error| HttpError::PreSubmitTimeout(error.to_string()))?;
        if connect_timeout == Duration::from_secs(3) {
            Ok(Arc::clone(&self.strict_client))
        } else if connect_timeout == Duration::from_secs(5) {
            Ok(Arc::clone(&self.standard_client))
        } else {
            build_reqwest_client(connect_timeout)
                .map(Arc::new)
                .map_err(|error| HttpError::PreSubmitTransport(error.to_string()))
        }
    }
}

fn build_reqwest_client(connect_timeout: Duration) -> Result<reqwest::blocking::Client, HttpError> {
    reqwest::blocking::Client::builder()
        .user_agent("gwt-github/0.1")
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(connect_timeout)
        .build()
        .map_err(|error| HttpError::Transport(error.to_string()))
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new().expect("default reqwest client")
    }
}

impl HttpTransport for ReqwestTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        execute_reqwest(self.standard_client.as_ref(), request, None)
    }

    fn execute_with_deadline(
        &self,
        request: HttpRequest,
        deadline: &ResolutionDeadline,
    ) -> Result<HttpResponse, HttpError> {
        deadline
            .remaining("github http request")
            .map_err(|error| HttpError::PreSubmitTimeout(error.to_string()))?;
        let client = self.deadline_client(deadline)?;
        let remaining = deadline
            .remaining("github http request")
            .map_err(|error| HttpError::PreSubmitTimeout(error.to_string()))?;
        execute_reqwest(client.as_ref(), request, Some(remaining))
    }
}

fn execute_reqwest(
    client: &reqwest::blocking::Client,
    request: HttpRequest,
    timeout: Option<Duration>,
) -> Result<HttpResponse, HttpError> {
    let method = match request.method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
    };
    let mut builder = client.request(method, &request.url);
    for (k, v) in &request.headers {
        builder = builder.header(k, v);
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(body) = request.body {
        builder = builder.body(body);
    }
    let resp = builder.send().map_err(|error| {
        if error.is_builder() {
            HttpError::PreSubmitTransport(error.to_string())
        } else if error.is_connect() && error.is_timeout() {
            HttpError::PreSubmitTimeout(error.to_string())
        } else if error.is_connect() {
            HttpError::PreSubmitTransport(error.to_string())
        } else if error.is_timeout() {
            HttpError::Timeout(error.to_string())
        } else {
            HttpError::Transport(error.to_string())
        }
    })?;
    let status = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = resp.text().map_err(|error| {
        if error.is_timeout() {
            HttpError::Timeout(error.to_string())
        } else {
            HttpError::Transport(error.to_string())
        }
    })?;
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

// ---------------------------------------------------------------------------
// HttpIssueClient
// ---------------------------------------------------------------------------

/// Real [`IssueClient`] that talks to GitHub via an [`HttpTransport`].
pub struct HttpIssueClient<T: HttpTransport = ReqwestTransport> {
    transport: T,
    token: String,
    owner: String,
    repo: String,
    rest_base: String,
    graphql_url: String,
}

impl HttpIssueClient<ReqwestTransport> {
    /// Construct an [`HttpIssueClient`] using `gh auth token` for credentials
    /// and the default [`ReqwestTransport`].
    pub fn from_gh_auth(owner: &str, repo: &str) -> Result<Self, ApiError> {
        let token = resolve_gh_token()?;
        let transport = ReqwestTransport::new().map_err(|e| ApiError::Network(e.to_string()))?;
        Ok(Self::with_transport(transport, token, owner, repo))
    }

    pub fn from_gh_auth_with_deadline(
        owner: &str,
        repo: &str,
        deadline: &ResolutionDeadline,
    ) -> Result<Self, ApiError> {
        deadline.remaining("owner client construction")?;
        let token = resolve_gh_token_with_deadline(deadline)?;
        let transport =
            ReqwestTransport::new().map_err(|error| ApiError::Network(error.to_string()))?;
        deadline.remaining("owner client construction")?;
        Ok(Self::with_transport(transport, token, owner, repo))
    }

    pub fn from_owner_environment_with_deadline(
        owner: &str,
        repo: &str,
        deadline: &ResolutionDeadline,
    ) -> Result<Self, ApiError> {
        deadline.remaining("owner client construction")?;
        const MODE: &str = "GWT_OWNER_GITHUB_TEST_MODE";
        const REST: &str = "GWT_OWNER_GITHUB_REST_BASE";
        const GRAPHQL: &str = "GWT_OWNER_GITHUB_GRAPHQL_URL";
        const TOKEN: &str = "GWT_OWNER_GITHUB_TOKEN";
        let values = [MODE, REST, GRAPHQL, TOKEN].map(std::env::var);
        if values.iter().all(Result::is_err) {
            return Self::from_gh_auth_with_deadline(owner, repo, deadline);
        }
        let [mode, rest_base, graphql_url, token] = values.map(|value| {
            value.map_err(|_| ApiError::TestOverrideRejected {
                reason: "complete owner GitHub test override is required".to_string(),
            })
        });
        let mode = mode?;
        let rest_base = rest_base?;
        let graphql_url = graphql_url?;
        let token = token?;
        if !cfg!(debug_assertions) || mode != "loopback-v1" || token.trim().is_empty() {
            return Err(ApiError::TestOverrideRejected {
                reason: "explicit debug loopback-v1 mode and non-empty token are required"
                    .to_string(),
            });
        }
        let transport =
            ReqwestTransport::new().map_err(|error| ApiError::Network(error.to_string()))?;
        let client = Self::with_transport(transport, token, owner, repo).with_test_endpoints(
            rest_base,
            graphql_url,
            true,
        )?;
        deadline.remaining("owner client construction")?;
        Ok(client)
    }
}

impl<T: HttpTransport> HttpIssueClient<T> {
    /// Construct with an explicit transport (useful for tests).
    pub fn with_transport(transport: T, token: String, owner: &str, repo: &str) -> Self {
        HttpIssueClient {
            transport,
            token,
            owner: owner.to_string(),
            repo: repo.to_string(),
            rest_base: "https://api.github.com".to_string(),
            graphql_url: "https://api.github.com/graphql".to_string(),
        }
    }

    /// Point an explicitly test-mode debug client at loopback endpoints.
    pub fn with_test_endpoints(
        mut self,
        rest_base: impl Into<String>,
        graphql_url: impl Into<String>,
        explicit_test_mode: bool,
    ) -> Result<Self, ApiError> {
        if !explicit_test_mode || !cfg!(debug_assertions) {
            return Err(ApiError::TestOverrideRejected {
                reason: "explicit debug test mode is required".to_string(),
            });
        }
        let rest_base = rest_base.into();
        let graphql_url = graphql_url.into();
        validate_loopback_endpoint(&rest_base)?;
        validate_loopback_endpoint(&graphql_url)?;
        self.rest_base = rest_base.trim_end_matches('/').to_string();
        self.graphql_url = graphql_url;
        Ok(self)
    }

    /// Accessor for the inner transport (tests).
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        vec![
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.token),
            ),
            (
                "Accept".to_string(),
                "application/vnd.github+json".to_string(),
            ),
            ("X-GitHub-Api-Version".to_string(), "2022-11-28".to_string()),
            ("User-Agent".to_string(), "gwt-github/0.1".to_string()),
        ]
    }

    fn rest_patch(&self, path: &str, body: Value) -> Result<HttpResponse, ApiError> {
        let mut headers = self.auth_headers();
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
        let resp = self
            .transport
            .execute(HttpRequest {
                method: HttpMethod::Patch,
                url: format!("{}{}", self.rest_base, path),
                headers,
                body: Some(body.to_string()),
            })
            .map_err(|e| ApiError::Network(e.to_string()))?;
        check_status(&resp)?;
        Ok(resp)
    }

    fn rest_post(&self, path: &str, body: Value) -> Result<HttpResponse, ApiError> {
        let mut headers = self.auth_headers();
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
        let resp = self
            .transport
            .execute(HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}{}", self.rest_base, path),
                headers,
                body: Some(body.to_string()),
            })
            .map_err(|e| ApiError::Network(e.to_string()))?;
        check_status(&resp)?;
        Ok(resp)
    }

    fn rest_delete(&self, path: &str) -> Result<HttpResponse, ApiError> {
        let resp = self
            .transport
            .execute(HttpRequest {
                method: HttpMethod::Delete,
                url: format!("{}{}", self.rest_base, path),
                headers: self.auth_headers(),
                body: None,
            })
            .map_err(|e| ApiError::Network(e.to_string()))?;
        check_status(&resp)?;
        Ok(resp)
    }

    fn graphql(&self, query: &str, variables: Value) -> Result<Value, ApiError> {
        let mut headers = self.auth_headers();
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
        let payload = json!({
            "query": query,
            "variables": variables,
        });
        let resp = self
            .transport
            .execute(HttpRequest {
                method: HttpMethod::Post,
                url: self.graphql_url.clone(),
                headers,
                body: Some(payload.to_string()),
            })
            .map_err(|e| ApiError::Network(e.to_string()))?;
        check_status(&resp)?;
        let value: Value = serde_json::from_str(&resp.body)
            .map_err(|e| ApiError::Unexpected(format!("graphql json: {e}")))?;
        if let Some(errs) = value.get("errors") {
            return Err(ApiError::Unexpected(format!("graphql errors: {errs}")));
        }
        Ok(value)
    }

    fn execute_owner_request(
        &self,
        request: HttpRequest,
        deadline: &ResolutionDeadline,
        operation: &str,
    ) -> Result<HttpResponse, ApiError> {
        deadline.remaining(operation)?;
        let response = self
            .transport
            .execute_with_deadline(request, deadline)
            .map_err(|error| match error {
                HttpError::PreSubmitTimeout(_) | HttpError::Timeout(_) => ApiError::Timeout {
                    operation: operation.to_string(),
                },
                HttpError::PreSubmitTransport(message) | HttpError::Transport(message) => {
                    ApiError::Network(message)
                }
            })?;
        deadline.remaining(operation)?;
        Ok(response)
    }

    fn owner_graphql(
        &self,
        query: &str,
        variables: Value,
        deadline: &ResolutionDeadline,
        operation: &str,
    ) -> Result<Value, ApiError> {
        let mut headers = self.auth_headers();
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
        let response = self.execute_owner_request(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.graphql_url.clone(),
                headers,
                body: Some(json!({ "query": query, "variables": variables }).to_string()),
            },
            deadline,
            operation,
        )?;
        check_status(&response)?;
        let value =
            serde_json::from_str::<Value>(&response.body).map_err(|error| ApiError::Parse {
                operation: operation.to_string(),
                message: error.to_string(),
            })?;
        deadline.remaining(operation)?;
        if let Some(errors) = value.get("errors").filter(|errors| !errors.is_null()) {
            return Err(classify_graphql_errors(errors, operation));
        }
        Ok(value)
    }

    fn owner_rest_mutation(
        &self,
        method: HttpMethod,
        path: &str,
        body: Value,
        deadline: &ResolutionDeadline,
        operation: &str,
    ) -> OwnerMutationResult<HttpResponse> {
        deadline
            .remaining(operation)
            .map_err(OwnerMutationError::PreSubmit)?;
        let mut headers = self.auth_headers();
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
        let response = self
            .transport
            .execute_with_deadline(
                HttpRequest {
                    method,
                    url: format!("{}{}", self.rest_base, path),
                    headers,
                    body: Some(body.to_string()),
                },
                deadline,
            )
            .map_err(|error| match error {
                HttpError::PreSubmitTimeout(_) => {
                    OwnerMutationError::PreSubmit(ApiError::Timeout {
                        operation: operation.to_string(),
                    })
                }
                HttpError::PreSubmitTransport(message) => {
                    OwnerMutationError::PreSubmit(ApiError::Network(message))
                }
                HttpError::Timeout(_) => {
                    OwnerMutationError::RemoteOutcomeUnknown(ApiError::Timeout {
                        operation: operation.to_string(),
                    })
                }
                HttpError::Transport(message) => {
                    OwnerMutationError::RemoteOutcomeUnknown(ApiError::Network(message))
                }
            })?;
        deadline
            .remaining(operation)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        check_status(&response).map_err(|error| {
            if response.status >= 500 {
                OwnerMutationError::RemoteOutcomeUnknown(error)
            } else {
                OwnerMutationError::PreSubmit(error)
            }
        })?;
        Ok(response)
    }

    fn owner_rest_optional(
        &self,
        path: &str,
        deadline: &ResolutionDeadline,
        operation: &str,
    ) -> Result<Option<HttpResponse>, ApiError> {
        let response = self.execute_owner_request(
            HttpRequest {
                method: HttpMethod::Get,
                url: format!("{}{}", self.rest_base, path),
                headers: self.auth_headers(),
                body: None,
            },
            deadline,
            operation,
        )?;
        if response.status == 404 {
            return Ok(None);
        }
        check_status(&response)?;
        Ok(Some(response))
    }
}

// ---------------------------------------------------------------------------
// Status / parsing helpers
// ---------------------------------------------------------------------------

fn check_status(resp: &HttpResponse) -> Result<(), ApiError> {
    let lowercase_body = resp.body.to_ascii_lowercase();
    let retry_after = response_header(resp, "retry-after").and_then(|value| value.parse().ok());
    match resp.status {
        200..=299 => Ok(()),
        _ if response_header(resp, "x-ratelimit-remaining") == Some("0") => {
            Err(ApiError::RateLimited { retry_after })
        }
        429 => Err(ApiError::RateLimited { retry_after }),
        401 | 403 if lowercase_body.contains("rate limit") => {
            Err(ApiError::RateLimited { retry_after })
        }
        401 => Err(ApiError::Unauthorized),
        // SPEC-3214 FR-011: keep the GitHub reason (e.g. personal repo
        // restrictions) instead of flattening every 403 into RateLimited.
        403 => Err(ApiError::PermissionDenied {
            message: github_error_message(&resp.body),
        }),
        404 => Err(ApiError::Unexpected(format!(
            "not found: {}",
            github_error_message(&resp.body)
        ))),
        422 if resp.body.contains("is too long") || resp.body.contains("body is too long") => {
            Err(ApiError::BodyTooLarge)
        }
        422 => Err(ApiError::Unexpected(format!("422: {}", resp.body))),
        status @ 500..=599 => Err(ApiError::Network(format!(
            "GitHub service returned HTTP {status}"
        ))),
        status => Err(ApiError::Unexpected(format!(
            "HTTP {status}: {}",
            resp.body
        ))),
    }
}

fn response_header<'a>(response: &'a HttpResponse, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn classify_graphql_errors(errors: &Value, operation: &str) -> ApiError {
    let error_entries = errors.as_array().map(Vec::as_slice).unwrap_or_default();
    let codes = error_entries
        .iter()
        .flat_map(|error| {
            [
                error.pointer("/extensions/type"),
                error.pointer("/extensions/code"),
                error.get("type"),
                error.get("code"),
            ]
        })
        .flatten()
        .filter_map(Value::as_str)
        .map(|code| code.to_ascii_uppercase())
        .collect::<Vec<_>>();
    let messages = error_entries
        .iter()
        .filter_map(|error| error.get("message").and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if codes
        .iter()
        .any(|code| matches!(code.as_str(), "RATE_LIMITED" | "RATE_LIMIT"))
        || messages
            .iter()
            .any(|message| message.contains("rate limit"))
    {
        return ApiError::RateLimited { retry_after: None };
    }
    if codes
        .iter()
        .any(|code| matches!(code.as_str(), "UNAUTHORIZED" | "UNAUTHENTICATED"))
    {
        return ApiError::Unauthorized;
    }
    if codes.iter().any(|code| code == "FORBIDDEN") {
        return ApiError::PermissionDenied {
            message: "GitHub GraphQL access forbidden".to_string(),
        };
    }
    ApiError::Parse {
        operation: operation.to_string(),
        message: "GitHub GraphQL returned an unclassified error".to_string(),
    }
}

/// Extract the `message` field from a GitHub error body, falling back to the
/// raw body so no reason is ever dropped.
fn github_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string())
}

fn validate_loopback_endpoint(endpoint: &str) -> Result<(), ApiError> {
    let parsed = reqwest::Url::parse(endpoint).map_err(|error| ApiError::TestOverrideRejected {
        reason: format!("invalid endpoint: {error}"),
    })?;
    let valid_scheme = matches!(parsed.scheme(), "http" | "https");
    let valid_host = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    let no_credentials = parsed.username().is_empty() && parsed.password().is_none();
    if !valid_scheme
        || !valid_host
        || !no_credentials
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ApiError::TestOverrideRejected {
            reason: "endpoint must be credential-free loopback HTTP(S) without query or fragment"
                .to_string(),
        });
    }
    Ok(())
}

fn resolve_gh_token() -> Result<String, ApiError> {
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &["auth", "token"],
        gwt_core::process_console::SpawnOptions::new("gh auth token").forward_output(false),
    )
    .map_err(|e| ApiError::Network(format!("gh auth token: {e}")))?;
    if !output.success() {
        return Err(ApiError::Unauthorized);
    }
    let token = output.stdout.trim().to_string();
    if token.is_empty() {
        return Err(ApiError::Unauthorized);
    }
    Ok(token)
}

fn resolve_gh_token_with_deadline(deadline: &ResolutionDeadline) -> Result<String, ApiError> {
    deadline.remaining("gh auth token")?;
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking_with_deadline(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &["auth", "token"],
        gwt_core::process_console::SpawnOptions::new("gh auth token").forward_output(false),
        deadline.expires_at(),
    )
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            ApiError::Timeout {
                operation: "gh auth token".to_string(),
            }
        } else {
            ApiError::Network(format!("gh auth token: {error}"))
        }
    })?;
    if !output.success() {
        return Err(ApiError::Unauthorized);
    }
    let token = output.stdout.trim().to_string();
    if token.is_empty() {
        return Err(ApiError::Unauthorized);
    }
    Ok(token)
}

#[cfg(test)]
mod transport_tests {
    #[cfg(unix)]
    use std::{ffi::OsString, sync::Mutex};
    use std::{sync::Arc, time::Duration};

    use super::{
        issue_generation, HttpError, HttpMethod, HttpRequest, HttpTransport, ReqwestTransport,
        ResolutionDeadline,
    };
    #[cfg(unix)]
    use super::{resolve_gh_token, resolve_gh_token_with_deadline};

    #[cfg(unix)]
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(unix)]
    struct ScopedEnvVar {
        name: &'static str,
        previous: Option<OsString>,
    }

    #[cfg(unix)]
    impl ScopedEnvVar {
        fn set(name: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(name);
            std::env::set_var(name, value);
            Self { name, previous }
        }
    }

    #[cfg(unix)]
    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    #[test]
    fn reqwest_transport_reuses_prebuilt_clients_by_connect_timeout_profile() {
        let transport = ReqwestTransport::new().expect("transport");
        let standard = ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(30));
        let strict = ResolutionDeadline::new(Duration::from_secs(3), Duration::from_secs(30));

        let first = transport
            .deadline_client(&standard)
            .expect("standard client");
        let second = transport
            .deadline_client(&standard)
            .expect("reused standard client");
        let strict = transport.deadline_client(&strict).expect("strict client");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &strict));
    }

    #[test]
    fn reqwest_transport_does_not_expand_noncanonical_connect_caps() {
        let transport = ReqwestTransport::new().expect("transport");
        let one_second = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(30));
        let strict = ResolutionDeadline::new(Duration::from_secs(3), Duration::from_secs(30));

        let one_second = transport
            .deadline_client(&one_second)
            .expect("one-second client");
        let strict = transport.deadline_client(&strict).expect("strict client");

        assert!(
            !Arc::ptr_eq(&one_second, &strict),
            "a one-second cap must not reuse the three-second client"
        );
    }

    #[test]
    fn reqwest_builder_failures_are_known_pre_submit() {
        let transport = ReqwestTransport::new().expect("transport");
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: "://invalid-url".to_string(),
            headers: Vec::new(),
            body: None,
        };

        let error = transport
            .execute_with_deadline(request, &deadline)
            .expect_err("invalid URL must fail before submission");

        assert!(
            matches!(error, HttpError::PreSubmitTransport(_)),
            "{error:?}"
        );
    }

    #[test]
    fn corpus_generation_rejects_an_expired_absolute_deadline() {
        let deadline = ResolutionDeadline::at(
            std::time::Instant::now() - Duration::from_millis(1),
            Duration::from_secs(1),
        );

        let error = issue_generation(&[], &deadline, "owner corpus generation")
            .expect_err("expired generation must stop");

        assert!(matches!(error, super::ApiError::Timeout { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn gh_auth_token_stdout_is_capture_only_for_both_paths() {
        use std::os::unix::fs::PermissionsExt;

        let _env_lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fake_bin = tempfile::tempdir().expect("fake bin");
        let fake_gh = fake_bin.path().join("gh");
        std::fs::write(&fake_gh, "#!/bin/sh\nprintf '%s\\n' \"$GWT_TEST_SECRET\"\n")
            .expect("write fake gh");
        std::fs::set_permissions(&fake_gh, std::fs::Permissions::from_mode(0o755))
            .expect("make fake gh executable");
        let secret = "unstructured-secret-value-without-known-prefix-92731";
        let _path = ScopedEnvVar::set("PATH", fake_bin.path());
        let _secret = ScopedEnvVar::set("GWT_TEST_SECRET", secret);
        let installed_hub = gwt_core::process_console::ProcessConsoleHub::new();
        let _ = gwt_core::process_console::set_global(installed_hub);
        let hub = gwt_core::process_console::global();

        assert_eq!(resolve_gh_token().expect("normal token"), secret);
        let deadline = ResolutionDeadline::new(Duration::from_millis(100), Duration::from_secs(1));
        assert_eq!(
            resolve_gh_token_with_deadline(&deadline).expect("deadline token"),
            secret
        );

        let leaked = hub
            .snapshot_kind(gwt_core::process_console::ProcessKind::Gh)
            .into_iter()
            .any(|line| line.message.contains(secret));
        assert!(!leaked, "gh auth token stdout reached Process Console");
    }
}

fn parse_issue_state(s: &str) -> IssueState {
    match s {
        "CLOSED" | "closed" => IssueState::Closed,
        _ => IssueState::Open,
    }
}

fn parse_graphql_issue(issue: &Value) -> Result<IssueSnapshot, ApiError> {
    let number = issue
        .get("number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ApiError::Unexpected("issue.number missing".into()))?;
    let title = issue
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let body = issue
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let state_raw = issue
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("OPEN");
    let state = parse_issue_state(state_raw);
    let updated_at = issue
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let labels: Vec<String> = issue
        .get("labels")
        .and_then(|l| l.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let comments: Vec<CommentSnapshot> = issue
        .get("comments")
        .and_then(|c| c.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let id = v.get("databaseId").and_then(serde_json::Value::as_u64)?;
                    let body = v
                        .get("body")
                        .and_then(|b| b.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let updated = v
                        .get("updatedAt")
                        .and_then(|u| u.as_str())
                        .unwrap_or_default()
                        .to_string();
                    Some(CommentSnapshot {
                        id: CommentId(id),
                        body,
                        updated_at: UpdatedAt(updated),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(IssueSnapshot {
        number: IssueNumber(number),
        title,
        body,
        labels,
        state,
        updated_at: UpdatedAt(updated_at),
        comments,
    })
}

fn parse_rest_issue(json: &Value) -> Result<IssueSnapshot, ApiError> {
    let number = json
        .get("number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ApiError::Unexpected("issue.number missing".into()))?;
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let body = json
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let state = json
        .get("state")
        .and_then(|v| v.as_str())
        .map(parse_issue_state)
        .unwrap_or(IssueState::Open);
    let updated_at = json
        .get("updated_at")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let labels: Vec<String> = json
        .get("labels")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(IssueSnapshot {
        number: IssueNumber(number),
        title,
        body,
        labels,
        state,
        updated_at: UpdatedAt(updated_at),
        comments: Vec::new(),
    })
}

fn parse_rest_comment(json: &Value) -> Result<CommentSnapshot, ApiError> {
    let id = json
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ApiError::Unexpected("comment.id missing".into()))?;
    let body = json
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let updated_at = json
        .get("updated_at")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(CommentSnapshot {
        id: CommentId(id),
        body,
        updated_at: UpdatedAt(updated_at),
    })
}

fn parse_owner_issue(
    value: &Value,
    repository: &RepositoryIdentity,
    operation: &str,
    rest_shape: bool,
) -> Result<RepositoryIssue, ApiError> {
    let parse = |message: &str| ApiError::Parse {
        operation: operation.to_string(),
        message: message.to_string(),
    };
    let number = value
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| parse("issue.number missing"))?;
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| parse("issue.title missing"))?
        .to_string();
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| parse("issue.body missing"))?
        .to_string();
    let state_value = value
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| parse("issue.state missing"))?;
    let state = match state_value {
        "OPEN" | "open" => IssueState::Open,
        "CLOSED" | "closed" => IssueState::Closed,
        _ => return Err(parse("issue.state invalid")),
    };
    let updated_key = if rest_shape {
        "updated_at"
    } else {
        "updatedAt"
    };
    let updated_at = value
        .get(updated_key)
        .and_then(Value::as_str)
        .ok_or_else(|| parse("issue.updated_at missing"))?
        .to_string();
    let labels_value = value
        .get("labels")
        .ok_or_else(|| parse("issue.labels missing"))?;
    let label_nodes = if rest_shape {
        labels_value.as_array()
    } else {
        labels_value.get("nodes").and_then(Value::as_array)
    }
    .ok_or_else(|| parse("issue.labels invalid"))?;
    if !rest_shape {
        let total_count = labels_value
            .get("totalCount")
            .and_then(Value::as_u64)
            .ok_or_else(|| ApiError::PartialPage {
                operation: operation.to_string(),
                completed_pages: 0,
            })?;
        if total_count != label_nodes.len() as u64 {
            return Err(ApiError::PartialPage {
                operation: operation.to_string(),
                completed_pages: 0,
            });
        }
    }
    let mut labels = Vec::with_capacity(label_nodes.len());
    for label in label_nodes {
        labels.push(
            label
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| parse("issue label name missing"))?
                .to_string(),
        );
    }
    labels.sort();
    let kind = if labels.iter().any(|label| label == "gwt-spec") {
        RepositoryIssueKind::Spec
    } else {
        RepositoryIssueKind::Plain
    };
    Ok(RepositoryIssue {
        repository: repository.clone(),
        number: IssueNumber(number),
        title,
        body,
        labels,
        state,
        kind,
        updated_at: UpdatedAt(updated_at),
    })
}

fn parse_owner_comment(value: &Value, operation: &str) -> Result<RepositoryComment, ApiError> {
    let parse = |message: &str| ApiError::Parse {
        operation: operation.to_string(),
        message: message.to_string(),
    };
    let id = value
        .get("databaseId")
        .or_else(|| value.get("id"))
        .and_then(Value::as_u64)
        .ok_or_else(|| parse("comment id missing"))?;
    if id == 0 {
        return Err(parse("comment id must be positive"));
    }
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| parse("comment body missing"))?
        .to_string();
    let updated_at = value
        .get("updatedAt")
        .or_else(|| value.get("updated_at"))
        .and_then(Value::as_str)
        .ok_or_else(|| parse("comment updated_at missing"))?
        .to_string();
    if chrono::DateTime::parse_from_rfc3339(&updated_at).is_err() {
        return Err(parse("comment updated_at is not RFC3339"));
    }
    let author = value
        .get("author")
        .or_else(|| value.get("user"))
        .and_then(Value::as_object);
    let author_login = author
        .and_then(|author| author.get("login"))
        .and_then(Value::as_str)
        .filter(|login| !login.is_empty())
        .map(str::to_string);
    let author_type = author
        .and_then(|author| author.get("__typename").or_else(|| author.get("type")))
        .and_then(Value::as_str)
        .map(|actor_type| match actor_type {
            "User" => RepositoryActorType::User,
            "Bot" => RepositoryActorType::Bot,
            "Organization" => RepositoryActorType::Organization,
            "Mannequin" => RepositoryActorType::Mannequin,
            "EnterpriseUserAccount" => RepositoryActorType::EnterpriseUserAccount,
            other => RepositoryActorType::Unknown(other.to_string()),
        });
    let author_association = value
        .get("authorAssociation")
        .or_else(|| value.get("author_association"))
        .and_then(Value::as_str)
        .map(|association| match association {
            "OWNER" => RepositoryAuthorAssociation::Owner,
            "MEMBER" => RepositoryAuthorAssociation::Member,
            "COLLABORATOR" => RepositoryAuthorAssociation::Collaborator,
            "CONTRIBUTOR" => RepositoryAuthorAssociation::Contributor,
            "FIRST_TIMER" => RepositoryAuthorAssociation::FirstTimer,
            "FIRST_TIME_CONTRIBUTOR" => RepositoryAuthorAssociation::FirstTimeContributor,
            "MANNEQUIN" => RepositoryAuthorAssociation::Mannequin,
            "NONE" => RepositoryAuthorAssociation::None,
            other => RepositoryAuthorAssociation::Unknown(other.to_string()),
        });
    Ok(RepositoryComment {
        id: CommentId(id),
        body,
        updated_at: UpdatedAt(updated_at),
        author_login,
        author_type,
        author_association,
    })
}

fn complete_generation(
    domain: &str,
    rows: Vec<String>,
    deadline: &ResolutionDeadline,
    operation: &str,
) -> Result<CollectionGeneration, ApiError> {
    let mut sorted_rows = BTreeMap::<String, usize>::new();
    for row in rows {
        deadline.remaining(operation)?;
        *sorted_rows.entry(row).or_default() += 1;
    }
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain.as_bytes());
    for (value, count) in sorted_rows {
        for _ in 0..count {
            deadline.remaining(operation)?;
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
    }
    deadline.remaining(operation)?;
    Ok(CollectionGeneration::new(format!(
        "gen:v1:{}",
        hex::encode(digest.finalize())
    )))
}

fn issue_generation(
    issues: &[RepositoryIssue],
    deadline: &ResolutionDeadline,
    operation: &str,
) -> Result<CollectionGeneration, ApiError> {
    let mut rows = Vec::with_capacity(issues.len());
    for issue in issues {
        deadline.remaining(operation)?;
        rows.push(format!(
            "{}\0{}\0{}\0{}\0{:?}\0{:?}\0{}\0{}",
            issue.repository,
            issue.number.0,
            issue.title,
            issue.body,
            issue.state,
            issue.kind,
            issue.labels.join("\u{1f}"),
            issue.updated_at.0,
        ));
    }
    complete_generation("gwt.owner-corpus.issues.v1", rows, deadline, operation)
}

fn comment_generation(
    comments: &[RepositoryComment],
    deadline: &ResolutionDeadline,
    operation: &str,
) -> Result<CollectionGeneration, ApiError> {
    let mut rows = Vec::with_capacity(comments.len());
    for comment in comments {
        deadline.remaining(operation)?;
        rows.push(format!(
            "{}\0{}\0{}\0{:?}\0{:?}\0{:?}",
            comment.id.0,
            comment.body,
            comment.updated_at.0,
            comment.author_login,
            comment.author_type,
            comment.author_association,
        ));
    }
    complete_generation("gwt.owner-corpus.comments.v1", rows, deadline, operation)
}

fn sort_owner_collection_by_key<T, K: Ord>(
    values: &mut Vec<T>,
    deadline: &ResolutionDeadline,
    operation: &str,
    key: impl Fn(&T) -> K,
) -> Result<(), ApiError> {
    let mut buckets = BTreeMap::<K, Vec<T>>::new();
    for value in std::mem::take(values) {
        deadline.remaining(operation)?;
        buckets.entry(key(&value)).or_default().push(value);
    }
    for (_, mut bucket) in buckets {
        for value in bucket.drain(..) {
            deadline.remaining(operation)?;
            values.push(value);
        }
    }
    Ok(())
}

fn page_connection<'a>(
    value: &'a Value,
    path: &[&str],
    operation: &str,
) -> Result<(&'a [Value], bool, Option<&'a str>), ApiError> {
    let mut current = value;
    for key in path {
        current = current.get(*key).ok_or_else(|| ApiError::Parse {
            operation: operation.to_string(),
            message: format!("{key} missing"),
        })?;
    }
    let nodes = current
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Parse {
            operation: operation.to_string(),
            message: "nodes missing".to_string(),
        })?;
    let page_info = current
        .get("pageInfo")
        .ok_or_else(|| ApiError::PartialPage {
            operation: operation.to_string(),
            completed_pages: 0,
        })?;
    let has_next = page_info
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| ApiError::PartialPage {
            operation: operation.to_string(),
            completed_pages: 0,
        })?;
    let cursor = page_info.get("endCursor").and_then(Value::as_str);
    Ok((nodes, has_next, cursor))
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

// ---------------------------------------------------------------------------
// IssueClient implementation for HttpIssueClient
// ---------------------------------------------------------------------------

const FETCH_ISSUE_QUERY: &str = r#"
query($owner:String!,$repo:String!,$number:Int!){
  repository(owner:$owner, name:$repo){
    issue(number:$number){
      number title body state updatedAt
      labels(first:50){nodes{name}}
      comments(first:100){nodes{databaseId body updatedAt}}
    }
  }
}
"#;

const LIST_SPEC_ISSUES_QUERY: &str = r#"
query($owner:String!,$repo:String!,$after:String){
  repository(owner:$owner, name:$repo){
    issues(labels:["gwt-spec"], first:100, after:$after, orderBy:{field:UPDATED_AT,direction:DESC}){
      nodes{ number title state updatedAt labels(first:20){nodes{name}} }
      pageInfo{ hasNextPage endCursor }
    }
  }
}
"#;

const LIST_OWNER_ISSUES_QUERY: &str = r#"
query($owner:String!,$repo:String!,$after:String){
  repository(owner:$owner, name:$repo){
    issues(states:[OPEN,CLOSED], first:100, after:$after, orderBy:{field:CREATED_AT,direction:ASC}){
      nodes{ number title body state updatedAt labels(first:100){totalCount nodes{name}} }
      pageInfo{ hasNextPage endCursor }
    }
  }
}
"#;

const LIST_OWNER_COMMENTS_QUERY: &str = r#"
query($owner:String!,$repo:String!,$number:Int!,$after:String){
  repository(owner:$owner, name:$repo){
    issue(number:$number){
      comments(first:100, after:$after){
        nodes{
          databaseId body updatedAt authorAssociation
          author{ login __typename }
        }
        pageInfo{ hasNextPage endCursor }
      }
    }
  }
}
"#;

const FETCH_OWNER_ISSUE_QUERY: &str = r#"
query($owner:String!,$repo:String!,$number:Int!){
  repository(owner:$owner, name:$repo){
    issue(number:$number){
      number title body state updatedAt labels(first:100){totalCount nodes{name}}
    }
  }
}
"#;

impl<T: HttpTransport> IssueClient for HttpIssueClient<T> {
    fn fetch(
        &self,
        number: IssueNumber,
        since: Option<&UpdatedAt>,
    ) -> Result<FetchResult, ApiError> {
        let value = self.graphql(
            FETCH_ISSUE_QUERY,
            json!({
                "owner": self.owner,
                "repo": self.repo,
                "number": number.0,
            }),
        )?;
        let issue = value
            .get("data")
            .and_then(|d| d.get("repository"))
            .and_then(|r| r.get("issue"))
            .ok_or(ApiError::NotFound(number))?;
        if issue.is_null() {
            return Err(ApiError::NotFound(number));
        }
        let snapshot = parse_graphql_issue(issue)?;
        if let Some(prev) = since {
            if *prev == snapshot.updated_at {
                return Ok(FetchResult::NotModified);
            }
        }
        Ok(FetchResult::Updated(snapshot))
    }

    fn patch_body(&self, number: IssueNumber, new_body: &str) -> Result<IssueSnapshot, ApiError> {
        let path = format!("/repos/{}/{}/issues/{}", self.owner, self.repo, number.0);
        let resp = self.rest_patch(&path, json!({ "body": new_body }))?;
        let value: Value = serde_json::from_str(&resp.body)
            .map_err(|e| ApiError::Unexpected(format!("patch_body json: {e}")))?;
        parse_rest_issue(&value)
    }

    fn patch_title(&self, number: IssueNumber, new_title: &str) -> Result<IssueSnapshot, ApiError> {
        let path = format!("/repos/{}/{}/issues/{}", self.owner, self.repo, number.0);
        let resp = self.rest_patch(&path, json!({ "title": new_title }))?;
        let value: Value = serde_json::from_str(&resp.body)
            .map_err(|e| ApiError::Unexpected(format!("patch_title json: {e}")))?;
        parse_rest_issue(&value)
    }

    fn patch_comment(
        &self,
        comment_id: CommentId,
        new_body: &str,
    ) -> Result<CommentSnapshot, ApiError> {
        let path = format!(
            "/repos/{}/{}/issues/comments/{}",
            self.owner, self.repo, comment_id.0
        );
        let resp = self.rest_patch(&path, json!({ "body": new_body }))?;
        let value: Value = serde_json::from_str(&resp.body)
            .map_err(|e| ApiError::Unexpected(format!("patch_comment json: {e}")))?;
        parse_rest_comment(&value)
    }

    fn create_comment(&self, number: IssueNumber, body: &str) -> Result<CommentSnapshot, ApiError> {
        let path = format!(
            "/repos/{}/{}/issues/{}/comments",
            self.owner, self.repo, number.0
        );
        let resp = self.rest_post(&path, json!({ "body": body }))?;
        let value: Value = serde_json::from_str(&resp.body)
            .map_err(|e| ApiError::Unexpected(format!("create_comment json: {e}")))?;
        parse_rest_comment(&value)
    }

    fn delete_comment(&self, comment_id: CommentId) -> Result<(), ApiError> {
        let path = format!(
            "/repos/{}/{}/issues/comments/{}",
            self.owner, self.repo, comment_id.0
        );
        // GitHub answers 204 No Content on success; 404 maps through
        // check_status like every other comment operation.
        let _resp = self.rest_delete(&path)?;
        Ok(())
    }

    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        let path = format!("/repos/{}/{}/issues", self.owner, self.repo);
        let resp = self.rest_post(
            &path,
            json!({
                "title": title,
                "body": body,
                "labels": labels,
            }),
        )?;
        let value: Value = serde_json::from_str(&resp.body)
            .map_err(|e| ApiError::Unexpected(format!("create_issue json: {e}")))?;
        parse_rest_issue(&value)
    }

    fn set_labels(
        &self,
        number: IssueNumber,
        labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        let path = format!("/repos/{}/{}/issues/{}", self.owner, self.repo, number.0);
        let resp = self.rest_patch(&path, json!({ "labels": labels }))?;
        let value: Value = serde_json::from_str(&resp.body)
            .map_err(|e| ApiError::Unexpected(format!("set_labels json: {e}")))?;
        parse_rest_issue(&value)
    }

    fn set_state(&self, number: IssueNumber, state: IssueState) -> Result<IssueSnapshot, ApiError> {
        let path = format!("/repos/{}/{}/issues/{}", self.owner, self.repo, number.0);
        let state_str = match state {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
        };
        let resp = self.rest_patch(&path, json!({ "state": state_str }))?;
        let value: Value = serde_json::from_str(&resp.body)
            .map_err(|e| ApiError::Unexpected(format!("set_state json: {e}")))?;
        parse_rest_issue(&value)
    }

    fn list_spec_issues(&self, filter: &SpecListFilter) -> Result<Vec<SpecSummary>, ApiError> {
        let value = self.graphql(
            LIST_SPEC_ISSUES_QUERY,
            json!({
                "owner": self.owner,
                "repo": self.repo,
                "after": serde_json::Value::Null,
            }),
        )?;
        let nodes = value
            .get("data")
            .and_then(|d| d.get("repository"))
            .and_then(|r| r.get("issues"))
            .and_then(|i| i.get("nodes"))
            .and_then(|n| n.as_array())
            .cloned()
            .unwrap_or_default();

        let mut out: Vec<SpecSummary> = nodes
            .into_iter()
            .filter_map(|v| {
                let number = v.get("number").and_then(serde_json::Value::as_u64)?;
                let title = v
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .to_string();
                let state = v
                    .get("state")
                    .and_then(|s| s.as_str())
                    .map(parse_issue_state)
                    .unwrap_or(IssueState::Open);
                let updated_at = v
                    .get("updatedAt")
                    .and_then(|u| u.as_str())
                    .unwrap_or_default()
                    .to_string();
                let labels: Vec<String> = v
                    .get("labels")
                    .and_then(|l| l.get("nodes"))
                    .and_then(|n| n.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| {
                                x.get("name").and_then(|n| n.as_str()).map(String::from)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(SpecSummary {
                    number: IssueNumber(number),
                    title,
                    state,
                    labels,
                    updated_at: UpdatedAt(updated_at),
                })
            })
            .collect();

        // Apply filters.
        if let Some(phase) = &filter.phase {
            out.retain(|s| {
                s.labels
                    .iter()
                    .any(|l| l == &format!("phase/{phase}") || l == phase.as_str())
            });
        }
        if let Some(state) = filter.state {
            out.retain(|s| s.state == state);
        }

        Ok(out)
    }
}

impl<T: HttpTransport> OwnerRepositoryClient for HttpIssueClient<T> {
    fn list_issues(
        &self,
        repository: &RepositoryIdentity,
        deadline: &ResolutionDeadline,
    ) -> Result<CompleteCollection<RepositoryIssue>, ApiError> {
        let operation = "list owner issues";
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut completed_pages = 0;
        let mut issues = Vec::new();
        loop {
            let value = self.owner_graphql(
                LIST_OWNER_ISSUES_QUERY,
                json!({
                    "owner": repository.owner(),
                    "repo": repository.name(),
                    "after": cursor,
                }),
                deadline,
                operation,
            )?;
            let (nodes, has_next, next_cursor) =
                page_connection(&value, &["data", "repository", "issues"], operation)
                    .map_err(|error| remap_partial_page(error, completed_pages))?;
            for node in nodes {
                deadline.remaining(operation)?;
                let issue = parse_owner_issue(node, repository, operation, false)
                    .map_err(|error| remap_partial_page(error, completed_pages))?;
                issues.push(issue);
            }
            deadline.remaining(operation)?;
            completed_pages += 1;
            if !has_next {
                break;
            }
            let next_cursor = next_cursor
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ApiError::PartialPage {
                    operation: operation.to_string(),
                    completed_pages,
                })?;
            if !seen_cursors.insert(next_cursor.to_string()) {
                return Err(ApiError::PartialPage {
                    operation: operation.to_string(),
                    completed_pages,
                });
            }
            cursor = Some(next_cursor.to_string());
        }
        sort_owner_collection_by_key(&mut issues, deadline, operation, |issue| issue.number)?;
        let generation = issue_generation(&issues, deadline, operation)?;
        deadline.remaining(operation)?;
        Ok(CompleteCollection::from_complete(issues, generation))
    }

    fn list_comments(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<CompleteCollection<RepositoryComment>, ApiError> {
        let operation = "list owner comments";
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut completed_pages = 0;
        let mut comments = Vec::new();
        loop {
            let value = self.owner_graphql(
                LIST_OWNER_COMMENTS_QUERY,
                json!({
                    "owner": repository.owner(),
                    "repo": repository.name(),
                    "number": number.0,
                    "after": cursor,
                }),
                deadline,
                operation,
            )?;
            if value
                .pointer("/data/repository/issue")
                .is_some_and(Value::is_null)
            {
                return Err(ApiError::NotFound(number));
            }
            let (nodes, has_next, next_cursor) = page_connection(
                &value,
                &["data", "repository", "issue", "comments"],
                operation,
            )
            .map_err(|error| remap_partial_page(error, completed_pages))?;
            for node in nodes {
                deadline.remaining(operation)?;
                comments.push(parse_owner_comment(node, operation)?);
            }
            deadline.remaining(operation)?;
            completed_pages += 1;
            if !has_next {
                break;
            }
            let next_cursor = next_cursor
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ApiError::PartialPage {
                    operation: operation.to_string(),
                    completed_pages,
                })?;
            if !seen_cursors.insert(next_cursor.to_string()) {
                return Err(ApiError::PartialPage {
                    operation: operation.to_string(),
                    completed_pages,
                });
            }
            cursor = Some(next_cursor.to_string());
        }
        sort_owner_collection_by_key(&mut comments, deadline, operation, |comment| comment.id)?;
        let generation = comment_generation(&comments, deadline, operation)?;
        deadline.remaining(operation)?;
        Ok(CompleteCollection::from_complete(comments, generation))
    }

    fn fetch_issue(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<RepositoryIssue, ApiError> {
        let operation = "fetch owner issue";
        let value = self.owner_graphql(
            FETCH_OWNER_ISSUE_QUERY,
            json!({
                "owner": repository.owner(),
                "repo": repository.name(),
                "number": number.0,
            }),
            deadline,
            operation,
        )?;
        let issue = value
            .pointer("/data/repository/issue")
            .ok_or_else(|| ApiError::Parse {
                operation: operation.to_string(),
                message: "issue missing".to_string(),
            })?;
        if issue.is_null() {
            return Err(ApiError::NotFound(number));
        }
        let issue = parse_owner_issue(issue, repository, operation, false)?;
        deadline.remaining(operation)?;
        Ok(issue)
    }

    fn create_owner_comment(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        body: &str,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryComment> {
        let operation = "create owner comment";
        let response = self.owner_rest_mutation(
            HttpMethod::Post,
            &format!(
                "/repos/{}/{}/issues/{}/comments",
                repository.owner(),
                repository.name(),
                number.0
            ),
            json!({ "body": body }),
            deadline,
            operation,
        )?;
        let value = parse_owner_json(&response.body, operation)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        let submitted = parse_owner_comment(&value, operation)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        let readback = self
            .list_comments(repository, number, deadline)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        let comment = readback
            .items()
            .iter()
            .find(|comment| comment.id == submitted.id && comment.body == body)
            .cloned()
            .ok_or_else(|| {
                OwnerMutationError::RemoteOutcomeUnknown(ApiError::Parse {
                    operation: "read back owner comment".to_string(),
                    message: "created comment was absent or changed during readback".to_string(),
                })
            })?;
        deadline
            .remaining(operation)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        Ok(comment)
    }

    fn create_owner_issue(
        &self,
        repository: &RepositoryIdentity,
        input: &CreateRepositoryIssue,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryIssue> {
        let operation = "create owner issue";
        let response = self.owner_rest_mutation(
            HttpMethod::Post,
            &format!("/repos/{}/{}/issues", repository.owner(), repository.name()),
            json!({
                "title": input.title,
                "body": input.body,
                "labels": input.labels,
            }),
            deadline,
            operation,
        )?;
        let value = parse_owner_json(&response.body, operation)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        let submitted = parse_owner_issue(&value, repository, operation, true)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        let readback = self
            .fetch_issue(repository, submitted.number, deadline)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        let mut expected_labels = input.labels.clone();
        expected_labels.sort();
        if readback.title != input.title
            || readback.body != input.body
            || readback.labels != expected_labels
            || readback.state != IssueState::Open
        {
            return Err(OwnerMutationError::RemoteOutcomeUnknown(ApiError::Parse {
                operation: "read back owner issue".to_string(),
                message: "created Issue did not match the submitted payload".to_string(),
            }));
        }
        deadline
            .remaining(operation)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        Ok(readback)
    }

    fn close_issue_verified(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryIssue> {
        let operation = "close duplicate owner issue";
        self.owner_rest_mutation(
            HttpMethod::Patch,
            &format!(
                "/repos/{}/{}/issues/{}",
                repository.owner(),
                repository.name(),
                number.0
            ),
            json!({ "state": "closed" }),
            deadline,
            operation,
        )?;
        let issue = self
            .fetch_issue(repository, number, deadline)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        if issue.state != IssueState::Closed {
            return Err(OwnerMutationError::RemoteOutcomeUnknown(
                ApiError::Unexpected("duplicate owner close readback remained open".to_string()),
            ));
        }
        deadline
            .remaining(operation)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        Ok(issue)
    }

    fn fetch_merged_pull_request(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<Option<MergedPullRequest>, ApiError> {
        let operation = "fetch merged pull request";
        let Some(response) = self.owner_rest_optional(
            &format!(
                "/repos/{}/{}/pulls/{}",
                repository.owner(),
                repository.name(),
                number.0
            ),
            deadline,
            operation,
        )?
        else {
            return Ok(None);
        };
        let value = parse_owner_json(&response.body, operation)?;
        let merged = value
            .get("merged")
            .and_then(Value::as_bool)
            .ok_or_else(|| ApiError::Parse {
                operation: operation.to_string(),
                message: "merged missing".to_string(),
            })?;
        let returned_number = IssueNumber(required_u64(&value, "number", operation)?);
        if returned_number != number {
            return Err(ApiError::Parse {
                operation: operation.to_string(),
                message: format!(
                    "pull request number mismatch: requested {}, received {}",
                    number.0, returned_number.0
                ),
            });
        }
        if !merged {
            deadline.remaining(operation)?;
            return Ok(None);
        }
        let pull_request = MergedPullRequest {
            number: returned_number,
            merge_commit_sha: required_string(&value, "merge_commit_sha", operation)?,
            merged_at: required_string(&value, "merged_at", operation)?,
        };
        deadline.remaining(operation)?;
        Ok(Some(pull_request))
    }

    fn fetch_release_by_tag(
        &self,
        repository: &RepositoryIdentity,
        tag: &str,
        deadline: &ResolutionDeadline,
    ) -> Result<Option<RepositoryRelease>, ApiError> {
        let operation = "fetch release by tag";
        let Some(response) = self.owner_rest_optional(
            &format!(
                "/repos/{}/{}/releases/tags/{}",
                repository.owner(),
                repository.name(),
                encode_path_segment(tag)
            ),
            deadline,
            operation,
        )?
        else {
            return Ok(None);
        };
        let value = parse_owner_json(&response.body, operation)?;
        let returned_tag = required_string(&value, "tag_name", operation)?;
        if returned_tag != tag {
            return Err(ApiError::Parse {
                operation: operation.to_string(),
                message: format!("release tag mismatch: requested {tag}, received {returned_tag}"),
            });
        }
        let release = RepositoryRelease {
            tag_name: returned_tag,
            target_commitish: required_string(&value, "target_commitish", operation)?,
            published_at: required_string(&value, "published_at", operation)?,
        };
        deadline.remaining(operation)?;
        Ok(Some(release))
    }

    fn compare_commits(
        &self,
        repository: &RepositoryIdentity,
        base: &str,
        head: &str,
        deadline: &ResolutionDeadline,
    ) -> Result<CommitComparison, ApiError> {
        let operation = "compare commits";
        let path = format!(
            "/repos/{}/{}/compare/{}...{}",
            repository.owner(),
            repository.name(),
            encode_path_segment(base),
            encode_path_segment(head)
        );
        let response = self
            .owner_rest_optional(&path, deadline, operation)?
            .ok_or_else(|| ApiError::Parse {
                operation: operation.to_string(),
                message: "comparison not found".to_string(),
            })?;
        let value = parse_owner_json(&response.body, operation)?;
        let status = match required_string(&value, "status", operation)?.as_str() {
            "ahead" => CommitComparisonStatus::Ahead,
            "behind" => CommitComparisonStatus::Behind,
            "identical" => CommitComparisonStatus::Identical,
            "diverged" => CommitComparisonStatus::Diverged,
            _ => {
                return Err(ApiError::Parse {
                    operation: operation.to_string(),
                    message: "comparison status invalid".to_string(),
                })
            }
        };
        let base_commit_sha = value
            .pointer("/base_commit/sha")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ApiError::Parse {
                operation: operation.to_string(),
                message: "base_commit.sha missing".to_string(),
            })?;
        let merge_base_commit_sha = value
            .pointer("/merge_base_commit/sha")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ApiError::Parse {
                operation: operation.to_string(),
                message: "merge_base_commit.sha missing".to_string(),
            })?;
        let head_commit_sha = match status {
            CommitComparisonStatus::Identical => base_commit_sha.clone(),
            CommitComparisonStatus::Behind => merge_base_commit_sha.clone(),
            CommitComparisonStatus::Ahead | CommitComparisonStatus::Diverged => value
                .get("commits")
                .and_then(Value::as_array)
                .and_then(|commits| commits.last())
                .and_then(|commit| commit.get("sha"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| ApiError::Parse {
                    operation: operation.to_string(),
                    message: "final commits[].sha missing".to_string(),
                })?,
        };
        let comparison = CommitComparison {
            base: base.to_string(),
            head: head.to_string(),
            base_commit_sha,
            merge_base_commit_sha,
            head_commit_sha,
            status,
            ahead_by: required_u64(&value, "ahead_by", operation)?,
            behind_by: required_u64(&value, "behind_by", operation)?,
        };
        comparison.validate_response(operation)?;
        deadline.remaining(operation)?;
        Ok(comparison)
    }
}

fn remap_partial_page(error: ApiError, completed_pages: usize) -> ApiError {
    match error {
        ApiError::PartialPage { operation, .. } => ApiError::PartialPage {
            operation,
            completed_pages,
        },
        other => other,
    }
}

fn parse_owner_json(body: &str, operation: &str) -> Result<Value, ApiError> {
    serde_json::from_str(body).map_err(|error| ApiError::Parse {
        operation: operation.to_string(),
        message: error.to_string(),
    })
}

fn required_string(value: &Value, field: &str, operation: &str) -> Result<String, ApiError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ApiError::Parse {
            operation: operation.to_string(),
            message: format!("{field} missing"),
        })
}

fn required_u64(value: &Value, field: &str, operation: &str) -> Result<u64, ApiError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ApiError::Parse {
            operation: operation.to_string(),
            message: format!("{field} missing"),
        })
}

#[cfg(test)]
mod check_status_tests {
    use super::{check_status, classify_graphql_errors, HttpResponse};
    use crate::client::ApiError;

    /// SPEC-3214 T-016 / FR-011: a non-rate-limit 403 must surface the
    /// GitHub-provided reason (e.g. personal repo restrictions) instead of
    /// being flattened into a rate-limit error.
    #[test]
    fn permission_denied_403_preserves_github_message() {
        let resp = HttpResponse {
            status: 403,
            headers: Vec::new(),
            body: r#"{"message":"Issues are disabled for this repo","documentation_url":"https://docs.github.com"}"#.to_string(),
        };
        let error = check_status(&resp).expect_err("403 must be an error");
        match error {
            ApiError::PermissionDenied { message } => {
                assert!(
                    message.contains("Issues are disabled for this repo"),
                    "GitHub reason must be preserved, got: {message}"
                );
            }
            other => panic!("403 must map to PermissionDenied, got: {other:?}"),
        }
    }

    #[test]
    fn rate_limited_403_still_maps_to_rate_limited() {
        let resp = HttpResponse {
            status: 403,
            headers: Vec::new(),
            body: r#"{"message":"API rate limit exceeded for user"}"#.to_string(),
        };
        assert!(matches!(
            check_status(&resp).expect_err("403 must be an error"),
            ApiError::RateLimited { .. }
        ));
    }

    #[test]
    fn exhausted_rate_limit_headers_preserve_retry_after() {
        let resp = HttpResponse {
            status: 403,
            headers: vec![
                ("x-ratelimit-remaining".to_string(), "0".to_string()),
                ("retry-after".to_string(), "17".to_string()),
            ],
            body: r#"{"message":"secondary limit"}"#.to_string(),
        };
        assert!(matches!(
            check_status(&resp).expect_err("exhausted limit must be an error"),
            ApiError::RateLimited {
                retry_after: Some(17)
            }
        ));
    }

    #[test]
    fn graphql_rate_limit_variants_map_to_typed_failure() {
        let variants = [
            serde_json::json!([{"type": "RATE_LIMITED"}]),
            serde_json::json!([{"code": "RATE_LIMIT"}]),
            serde_json::json!([{"message": "API rate limit exceeded"}]),
        ];
        for errors in variants {
            assert!(matches!(
                classify_graphql_errors(&errors, "owner query"),
                ApiError::RateLimited { .. }
            ));
        }
    }

    /// SPEC-3214 R-7: 404 responses also keep the server message instead of a
    /// bare "not found".
    #[test]
    fn not_found_404_preserves_github_message() {
        let resp = HttpResponse {
            status: 404,
            headers: Vec::new(),
            body: r#"{"message":"Not Found: repository archived"}"#.to_string(),
        };
        match check_status(&resp).expect_err("404 must be an error") {
            ApiError::Unexpected(message) => {
                assert!(
                    message.contains("repository archived"),
                    "server message must be preserved, got: {message}"
                );
            }
            other => panic!("404 must map to Unexpected with message, got: {other:?}"),
        }
    }
}
