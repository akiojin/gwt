//! Contract tests for [`HttpIssueClient`] via the injected [`FakeTransport`]
//! (SPEC-12 tdd.md Layer 5).
//!
//! These tests pin down the exact request shape emitted by the HTTP client:
//! URLs, headers, HTTP methods, and JSON bodies. They also verify that the
//! parsing path handles GraphQL / REST responses correctly and that status
//! code mapping matches [`ApiError`].

use gwt_github::client::{
    http::{FakeTransport, HttpIssueClient, HttpMethod, HttpResponse},
    ApiError, CommentId, FetchResult, IssueClient, IssueNumber, IssueState, SpecListFilter,
    UpdatedAt,
};

fn client_with(transport: FakeTransport) -> HttpIssueClient<FakeTransport> {
    HttpIssueClient::with_transport(transport, "test-token".to_string(), "octo", "gwt")
}

fn ok_body(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: body.to_string(),
    }
}

fn created(body: &str) -> HttpResponse {
    HttpResponse {
        status: 201,
        body: body.to_string(),
    }
}

// -----------------------------------------------------------------------
// RED-50: fetch issues a POST to the GraphQL endpoint with Bearer auth
// -----------------------------------------------------------------------
#[test]
fn red_50_fetch_posts_graphql_with_auth() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        r#"{"data":{"repository":{"issue":{
            "number":2001,"title":"T","body":"B","state":"OPEN","updatedAt":"2026-04-08T00:00:00Z",
            "labels":{"nodes":[{"name":"gwt-spec"},{"name":"phase/review"}]},
            "comments":{"nodes":[]}
        }}}}"#,
    ));
    let client = client_with(transport);
    let res = client.fetch(IssueNumber(2001), None).unwrap();

    let reqs = client.transport().recorded();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    assert_eq!(req.method, HttpMethod::Post);
    assert_eq!(req.url, "https://api.github.com/graphql");
    assert!(req
        .headers
        .iter()
        .any(|(k, v)| k == "Authorization" && v == "Bearer test-token"));
    assert!(req
        .headers
        .iter()
        .any(|(k, v)| k == "Content-Type" && v == "application/json"));
    // Body is a JSON payload carrying query + variables.
    let body = req.body.as_deref().unwrap_or("");
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert!(payload.get("query").is_some());
    assert_eq!(payload["variables"]["owner"], "octo");
    assert_eq!(payload["variables"]["repo"], "gwt");
    assert_eq!(payload["variables"]["number"], 2001);

    match res {
        FetchResult::Updated(snap) => {
            assert_eq!(snap.number, IssueNumber(2001));
            assert_eq!(snap.title, "T");
            assert_eq!(snap.state, IssueState::Open);
            assert_eq!(snap.updated_at, UpdatedAt::new("2026-04-08T00:00:00Z"));
            assert_eq!(snap.labels, vec!["gwt-spec", "phase/review"]);
        }
        _ => panic!("expected Updated"),
    }
}

// -----------------------------------------------------------------------
// RED-51: fetch returns NotModified when updatedAt matches
// -----------------------------------------------------------------------
#[test]
fn red_51_fetch_returns_not_modified_on_match() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        r#"{"data":{"repository":{"issue":{
            "number":1,"title":"t","body":"b","state":"OPEN","updatedAt":"T1",
            "labels":{"nodes":[]},
            "comments":{"nodes":[]}
        }}}}"#,
    ));
    let client = client_with(transport);
    let res = client
        .fetch(IssueNumber(1), Some(&UpdatedAt::new("T1")))
        .unwrap();
    assert!(matches!(res, FetchResult::NotModified));
}

// -----------------------------------------------------------------------
// RED-52: fetch returns NotFound when issue is null in GraphQL response
// -----------------------------------------------------------------------
#[test]
fn red_52_fetch_returns_not_found() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(r#"{"data":{"repository":{"issue":null}}}"#));
    let client = client_with(transport);
    let err = client.fetch(IssueNumber(9999), None).unwrap_err();
    assert!(matches!(err, ApiError::NotFound(IssueNumber(9999))));
}

// -----------------------------------------------------------------------
// RED-53: patch_body issues a PATCH /repos/.../issues/:n with body field
// -----------------------------------------------------------------------
#[test]
fn red_53_patch_body_issues_rest_patch() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        r#"{"number":2001,"title":"T","body":"new body","state":"open","updated_at":"t","labels":[]}"#,
    ));
    let client = client_with(transport);
    let snap = client.patch_body(IssueNumber(2001), "new body").unwrap();

    let reqs = client.transport().recorded();
    let req = &reqs[0];
    assert_eq!(req.method, HttpMethod::Patch);
    assert_eq!(req.url, "https://api.github.com/repos/octo/gwt/issues/2001");
    let payload: serde_json::Value = serde_json::from_str(req.body.as_deref().unwrap()).unwrap();
    assert_eq!(payload["body"], "new body");
    assert_eq!(snap.body, "new body");
}

// -----------------------------------------------------------------------
// RED-54: patch_comment hits the comment endpoint and returns snapshot
// -----------------------------------------------------------------------
#[test]
fn red_54_patch_comment_uses_comment_endpoint() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(r#"{"id":777,"body":"new","updated_at":"t"}"#));
    let client = client_with(transport);
    let snap = client.patch_comment(CommentId(777), "new").unwrap();

    let reqs = client.transport().recorded();
    assert_eq!(reqs[0].method, HttpMethod::Patch);
    assert_eq!(
        reqs[0].url,
        "https://api.github.com/repos/octo/gwt/issues/comments/777"
    );
    assert_eq!(snap.id, CommentId(777));
    assert_eq!(snap.body, "new");
}

// -----------------------------------------------------------------------
// RED-55: create_comment POSTs to /repos/.../issues/:n/comments
// -----------------------------------------------------------------------
#[test]
fn red_55_create_comment_posts_to_issue_comments() {
    let transport = FakeTransport::new();
    transport.enqueue(created(r#"{"id":4500,"body":"hello","updated_at":"t"}"#));
    let client = client_with(transport);
    let snap = client.create_comment(IssueNumber(10), "hello").unwrap();

    let reqs = client.transport().recorded();
    assert_eq!(reqs[0].method, HttpMethod::Post);
    assert_eq!(
        reqs[0].url,
        "https://api.github.com/repos/octo/gwt/issues/10/comments"
    );
    assert_eq!(snap.id, CommentId(4500));
    assert_eq!(snap.body, "hello");
}

// -----------------------------------------------------------------------
// RED-56: create_issue POSTs to /repos/.../issues with title + body + labels
// -----------------------------------------------------------------------
#[test]
fn red_56_create_issue_posts_with_labels() {
    let transport = FakeTransport::new();
    transport.enqueue(created(
        r#"{"number":99,"title":"T","body":"B","state":"open","updated_at":"t","labels":[{"name":"gwt-spec"},{"name":"phase/draft"}]}"#,
    ));
    let client = client_with(transport);
    let labels = vec!["gwt-spec".to_string(), "phase/draft".to_string()];
    let snap = client.create_issue("T", "B", &labels).unwrap();

    let reqs = client.transport().recorded();
    assert_eq!(reqs[0].method, HttpMethod::Post);
    assert_eq!(reqs[0].url, "https://api.github.com/repos/octo/gwt/issues");
    let payload: serde_json::Value =
        serde_json::from_str(reqs[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(payload["title"], "T");
    assert_eq!(payload["body"], "B");
    assert_eq!(payload["labels"][0], "gwt-spec");
    assert_eq!(payload["labels"][1], "phase/draft");
    assert_eq!(snap.number, IssueNumber(99));
    assert_eq!(snap.labels, labels);
}

// -----------------------------------------------------------------------
// RED-57: set_labels PATCHes the issue with the labels field
// -----------------------------------------------------------------------
#[test]
fn red_57_set_labels_patches_labels_only() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        r#"{"number":5,"title":"T","body":"B","state":"open","updated_at":"t","labels":[{"name":"gwt-spec"},{"name":"phase/done"}]}"#,
    ));
    let client = client_with(transport);
    let labels = vec!["gwt-spec".to_string(), "phase/done".to_string()];
    let snap = client.set_labels(IssueNumber(5), &labels).unwrap();

    let reqs = client.transport().recorded();
    assert_eq!(reqs[0].method, HttpMethod::Patch);
    let payload: serde_json::Value =
        serde_json::from_str(reqs[0].body.as_deref().unwrap()).unwrap();
    assert!(payload.get("body").is_none());
    assert_eq!(payload["labels"][0], "gwt-spec");
    assert_eq!(snap.labels, labels);
}

// -----------------------------------------------------------------------
// RED-58: set_state sends state: "closed"
// -----------------------------------------------------------------------
#[test]
fn red_58_set_state_closed_sends_state_field() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        r#"{"number":5,"title":"T","body":"B","state":"closed","updated_at":"t","labels":[]}"#,
    ));
    let client = client_with(transport);
    let snap = client
        .set_state(IssueNumber(5), IssueState::Closed)
        .unwrap();

    let reqs = client.transport().recorded();
    let payload: serde_json::Value =
        serde_json::from_str(reqs[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(payload["state"], "closed");
    assert_eq!(snap.state, IssueState::Closed);
}

// -----------------------------------------------------------------------
// RED-59: list_spec_issues issues one GraphQL call and filters locally
// -----------------------------------------------------------------------
#[test]
fn red_59_list_spec_issues_graphql_single_call() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        r#"{"data":{"repository":{"issues":{"nodes":[
            {"number":3,"title":"c","state":"OPEN","updatedAt":"t3","labels":{"nodes":[{"name":"gwt-spec"},{"name":"phase/implementation"}]}},
            {"number":2,"title":"b","state":"OPEN","updatedAt":"t2","labels":{"nodes":[{"name":"gwt-spec"},{"name":"phase/done"}]}},
            {"number":1,"title":"a","state":"OPEN","updatedAt":"t1","labels":{"nodes":[{"name":"gwt-spec"},{"name":"phase/draft"}]}}
        ],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#,
    ));
    let client = client_with(transport);
    let list = client
        .list_spec_issues(&SpecListFilter {
            phase: Some("implementation".to_string()),
            state: None,
        })
        .unwrap();

    let reqs = client.transport().recorded();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].url, "https://api.github.com/graphql");

    assert_eq!(list.len(), 1);
    assert_eq!(list[0].number, IssueNumber(3));
    assert_eq!(list[0].title, "c");
}

// -----------------------------------------------------------------------
// RED-60: 422 with "body is too long" maps to BodyTooLarge
// -----------------------------------------------------------------------
#[test]
fn red_60_422_body_too_long_maps_to_body_too_large() {
    let transport = FakeTransport::new();
    transport.enqueue(HttpResponse {
        status: 422,
        body: r#"{"message":"Validation Failed","errors":[{"resource":"Issue","code":"custom","message":"body is too long"}]}"#
            .to_string(),
    });
    let client = client_with(transport);
    let err = client.patch_body(IssueNumber(1), "huge").unwrap_err();
    assert!(matches!(err, ApiError::BodyTooLarge));
}

// -----------------------------------------------------------------------
// RED-61: 403 with "rate limit" maps to RateLimited
// -----------------------------------------------------------------------
#[test]
fn red_61_403_rate_limit_maps_to_rate_limited() {
    let transport = FakeTransport::new();
    transport.enqueue(HttpResponse {
        status: 403,
        body: r#"{"message":"API rate limit exceeded"}"#.to_string(),
    });
    let client = client_with(transport);
    let err = client.patch_body(IssueNumber(1), "x").unwrap_err();
    assert!(matches!(err, ApiError::RateLimited { .. }));
}

// -----------------------------------------------------------------------
// RED-62: 401 maps to Unauthorized
// -----------------------------------------------------------------------
#[test]
fn red_62_401_maps_to_unauthorized() {
    let transport = FakeTransport::new();
    transport.enqueue(HttpResponse {
        status: 401,
        body: r#"{"message":"Bad credentials"}"#.to_string(),
    });
    let client = client_with(transport);
    let err = client.patch_body(IssueNumber(1), "x").unwrap_err();
    assert!(matches!(err, ApiError::Unauthorized));
}

// -----------------------------------------------------------------------
// RED-63: transport error propagates as Network
// -----------------------------------------------------------------------
#[test]
fn red_63_transport_error_is_network() {
    let transport = FakeTransport::new();
    // No canned response -> FakeTransport returns HttpError::Transport.
    let client = client_with(transport);
    let err = client.patch_body(IssueNumber(1), "x").unwrap_err();
    assert!(matches!(err, ApiError::Network(_)));
}
