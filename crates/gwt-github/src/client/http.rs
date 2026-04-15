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

use std::process::Command;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::client::{
    ApiError, CommentId, CommentSnapshot, FetchResult, IssueClient, IssueNumber, IssueSnapshot,
    IssueState, SpecListFilter, SpecSummary, UpdatedAt,
};

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Patch,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Patch => "PATCH",
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
    pub body: String,
}

/// Transport errors surfaced by [`HttpTransport::execute`].
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("transport error: {0}")]
    Transport(String),
}

/// Abstract HTTP executor used by [`HttpIssueClient`].
pub trait HttpTransport: Send + Sync {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError>;
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
    canned: std::collections::VecDeque<HttpResponse>,
}

impl FakeTransport {
    pub fn new() -> Self {
        FakeTransport {
            state: Mutex::new(FakeState {
                recorded: Vec::new(),
                canned: std::collections::VecDeque::new(),
            }),
        }
    }

    /// Queue a response to be returned by the next `execute` call.
    pub fn enqueue(&self, response: HttpResponse) {
        self.state.lock().unwrap().canned.push_back(response);
    }

    /// Snapshot of every recorded request so far.
    pub fn recorded(&self) -> Vec<HttpRequest> {
        self.state.lock().unwrap().recorded.clone()
    }
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for FakeTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut state = self.state.lock().unwrap();
        state.recorded.push(request);
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
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, HttpError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("gwt-github/0.1")
            .build()
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        Ok(ReqwestTransport { client })
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new().expect("default reqwest client")
    }
}

impl HttpTransport for ReqwestTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Patch => reqwest::Method::PATCH,
        };
        let mut builder = self.client.request(method, &request.url);
        for (k, v) in &request.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let resp = builder
            .send()
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .text()
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        Ok(HttpResponse { status, body })
    }
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

    /// Override API endpoints (useful when pointing at a test double server).
    pub fn with_endpoints(
        mut self,
        rest_base: impl Into<String>,
        graphql_url: impl Into<String>,
    ) -> Self {
        self.rest_base = rest_base.into();
        self.graphql_url = graphql_url.into();
        self
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
}

// ---------------------------------------------------------------------------
// Status / parsing helpers
// ---------------------------------------------------------------------------

fn check_status(resp: &HttpResponse) -> Result<(), ApiError> {
    match resp.status {
        200..=299 => Ok(()),
        401 | 403 if resp.body.contains("rate limit") => {
            Err(ApiError::RateLimited { retry_after: None })
        }
        401 => Err(ApiError::Unauthorized),
        403 => Err(ApiError::RateLimited { retry_after: None }),
        404 => Err(ApiError::Unexpected("not found".into())),
        422 if resp.body.contains("is too long") || resp.body.contains("body is too long") => {
            Err(ApiError::BodyTooLarge)
        }
        422 => Err(ApiError::Unexpected(format!("422: {}", resp.body))),
        status => Err(ApiError::Unexpected(format!(
            "HTTP {status}: {}",
            resp.body
        ))),
    }
}

fn resolve_gh_token() -> Result<String, ApiError> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|e| ApiError::Network(format!("gh auth token: {e}")))?;
    if !output.status.success() {
        return Err(ApiError::Unauthorized);
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(ApiError::Unauthorized);
    }
    Ok(token)
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
        .and_then(|v| v.as_u64())
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
                    let id = v.get("databaseId").and_then(|i| i.as_u64())?;
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
        .and_then(|v| v.as_u64())
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
        .and_then(|v| v.as_u64())
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
                let number = v.get("number").and_then(|n| n.as_u64())?;
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
