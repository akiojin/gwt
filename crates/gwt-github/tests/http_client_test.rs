//! Contract tests for [`HttpIssueClient`] via the injected [`FakeTransport`]
//! (SPEC-12 tdd.md Layer 5).
//!
//! These tests pin down the exact request shape emitted by the HTTP client:
//! URLs, headers, HTTP methods, and JSON bodies. They also verify that the
//! parsing path handles GraphQL / REST responses correctly and that status
//! code mapping matches [`ApiError`].

use gwt_github::client::{
    http::{FakeTransport, HttpIssueClient, HttpMethod, HttpResponse},
    ApiError, CommentId, CommitComparisonStatus, CreateRepositoryIssue, FetchResult, IssueClient,
    IssueNumber, IssueState, OwnerMutationError, OwnerRepositoryClient, RepositoryActorType,
    RepositoryAuthorAssociation, RepositoryIdentity, RepositoryIssueKind, ResolutionDeadline,
    SpecListFilter, UpdatedAt,
};
use std::{
    ffi::{OsStr, OsString},
    sync::Mutex,
    time::{Duration, Instant},
};

#[cfg(unix)]
static PATH_ENV_LOCK: Mutex<()> = Mutex::new(());
static OWNER_ENV_LOCK: Mutex<()> = Mutex::new(());

const OWNER_ENV_KEYS: [&str; 4] = [
    "GWT_OWNER_GITHUB_TEST_MODE",
    "GWT_OWNER_GITHUB_REST_BASE",
    "GWT_OWNER_GITHUB_GRAPHQL_URL",
    "GWT_OWNER_GITHUB_TOKEN",
];

struct ScopedEnv {
    original: Vec<(&'static str, Option<OsString>)>,
}

impl ScopedEnv {
    fn cleared(keys: &[&'static str]) -> Self {
        let original = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for key in keys {
            std::env::remove_var(key);
        }
        Self { original }
    }

    fn set(&self, key: &str, value: impl AsRef<OsStr>) {
        std::env::set_var(key, value);
    }
}

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        for (key, value) in &self.original {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn client_with(transport: FakeTransport) -> HttpIssueClient<FakeTransport> {
    HttpIssueClient::with_transport(transport, "test-token".to_string(), "octo", "gwt")
}

fn ok_body(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: body.to_string(),
    }
}

fn created(body: &str) -> HttpResponse {
    HttpResponse {
        status: 201,
        headers: Vec::new(),
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
        headers: Vec::new(),
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
        headers: Vec::new(),
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
        headers: Vec::new(),
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

fn owner_deadline() -> ResolutionDeadline {
    ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5))
}

fn owner_issue(number: u64, labels: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": format!("Owner {number}"),
        "body": format!("body {number}"),
        "state": if number.is_multiple_of(2) { "CLOSED" } else { "OPEN" },
        "updatedAt": format!("2026-07-14T00:00:{:02}Z", number % 60),
        "labels": {
            "totalCount": labels.len(),
            "nodes": labels.iter().map(|name| serde_json::json!({"name": name})).collect::<Vec<_>>()
        }
    })
}

fn owner_page(nodes: Vec<serde_json::Value>, has_next: bool, cursor: Option<&str>) -> HttpResponse {
    ok_body(
        &serde_json::json!({
            "data": { "repository": { "issues": {
                "nodes": nodes,
                "pageInfo": { "hasNextPage": has_next, "endCursor": cursor }
            }}}
        })
        .to_string(),
    )
}

#[test]
fn owner_list_issues_reads_every_page_and_uses_issue_only_connection() {
    let transport = FakeTransport::new();
    transport.enqueue(owner_page(
        (1..=100).map(|number| owner_issue(number, &[])).collect(),
        true,
        Some("cursor-100"),
    ));
    transport.enqueue(owner_page(
        vec![owner_issue(101, &["gwt-spec"])],
        false,
        None,
    ));
    let client = client_with(transport);
    let collection = client
        .list_issues(&RepositoryIdentity::gwt_upstream(), &owner_deadline())
        .expect("complete issue corpus");
    assert_eq!(collection.items().len(), 101);
    assert_eq!(collection.items()[100].kind, RepositoryIssueKind::Spec);
    assert!(!collection.generation().as_str().is_empty());

    let requests = client.transport().recorded();
    assert_eq!(requests.len(), 2);
    let first: serde_json::Value =
        serde_json::from_str(requests[0].body.as_deref().expect("first body")).unwrap();
    let second: serde_json::Value =
        serde_json::from_str(requests[1].body.as_deref().expect("second body")).unwrap();
    assert_eq!(first["variables"]["owner"], "akiojin");
    assert_eq!(first["variables"]["repo"], "gwt");
    assert!(first["variables"]["after"].is_null());
    assert_eq!(second["variables"]["after"], "cursor-100");
    let query = first["query"].as_str().expect("query");
    assert!(query.contains("issues("));
    assert!(query.contains("totalCount"));
    assert!(!query.contains("pullRequests"));
}

#[test]
fn owner_issue_generation_is_stable_and_tracks_corpus_content() {
    let baseline = owner_issue(1, &["gwt-spec"]);
    let identical = baseline.clone();
    let mut body_changed = baseline.clone();
    body_changed["body"] = serde_json::json!("changed body");
    let mut state_changed = baseline.clone();
    state_changed["state"] = serde_json::json!("CLOSED");
    let mut label_membership_changed = baseline.clone();
    label_membership_changed["labels"]["totalCount"] = serde_json::json!(2);
    label_membership_changed["labels"]["nodes"] =
        serde_json::json!([{"name":"gwt-spec"},{"name":"bug"}]);

    let transport = FakeTransport::new();
    for issue in [
        baseline,
        identical,
        body_changed,
        state_changed,
        label_membership_changed,
    ] {
        transport.enqueue(owner_page(vec![issue], false, None));
    }
    let client = client_with(transport);
    let generations = (0..5)
        .map(|_| {
            client
                .list_issues(&RepositoryIdentity::gwt_upstream(), &owner_deadline())
                .expect("complete issue corpus")
                .generation()
                .as_str()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(generations[0], generations[1]);
    for changed in &generations[2..] {
        assert_ne!(generations[0], *changed);
    }
}

#[test]
fn owner_issue_listing_rejects_truncated_nested_labels() {
    let transport = FakeTransport::new();
    let mut issue = owner_issue(1, &["gwt-spec"]);
    issue["labels"]["totalCount"] = serde_json::json!(101);
    transport.enqueue(owner_page(vec![issue], false, None));
    let client = client_with(transport);

    let error = client
        .list_issues(&RepositoryIdentity::gwt_upstream(), &owner_deadline())
        .expect_err("truncated labels must taint the owner corpus");

    assert!(matches!(error, ApiError::PartialPage { .. }));
}

#[test]
fn owner_list_comments_reads_more_than_one_hundred() {
    let transport = FakeTransport::new();
    let comments = (1..=100)
        .map(|id| {
            serde_json::json!({
                "databaseId": id,
                "body": format!("comment {id}"),
                "updatedAt": "2026-07-14T00:00:00Z",
                "author": {"login":"akiojin","__typename":"User"},
                "authorAssociation":"OWNER"
            })
        })
        .collect::<Vec<_>>();
    transport.enqueue(ok_body(
        &serde_json::json!({"data":{"repository":{"issue":{"comments":{
            "nodes": comments,
            "pageInfo":{"hasNextPage":true,"endCursor":"comment-100"}
        }}}}})
        .to_string(),
    ));
    transport.enqueue(ok_body(
        &serde_json::json!({"data":{"repository":{"issue":{"comments":{
            "nodes":[{
                "databaseId":101,
                "body":"comment 101",
                "updatedAt":"2026-07-14T00:00:01Z",
                "author":{"login":"akiojin","__typename":"User"},
                "authorAssociation":"OWNER"
            }],
            "pageInfo":{"hasNextPage":false,"endCursor":null}
        }}}}})
        .to_string(),
    ));
    let client = client_with(transport);
    let collection = client
        .list_comments(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(42),
            &owner_deadline(),
        )
        .expect("complete comment corpus");
    assert_eq!(collection.items().len(), 101);
    assert_eq!(client.transport().recorded().len(), 2);
}

#[test]
fn owner_list_comments_preserves_comment_actor_identity() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        &serde_json::json!({"data":{"repository":{"issue":{"comments":{
            "nodes":[{
                "databaseId":501,
                "body":"resolution marker",
                "updatedAt":"2026-07-14T00:00:01Z",
                "author":{"login":"akiojin","__typename":"User"},
                "authorAssociation":"OWNER"
            }],
            "pageInfo":{"hasNextPage":false,"endCursor":null}
        }}}}})
        .to_string(),
    ));
    let client = client_with(transport);

    let collection = client
        .list_comments(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(42),
            &owner_deadline(),
        )
        .expect("complete comment corpus");

    let comment = &collection.items()[0];
    assert_eq!(comment.author_login.as_deref(), Some("akiojin"));
    assert_eq!(comment.author_type, Some(RepositoryActorType::User));
    assert_eq!(
        comment.author_association,
        Some(RepositoryAuthorAssociation::Owner)
    );
    let request = &client.transport().recorded()[0];
    let request_body = request.body.as_deref().expect("GraphQL request body");
    assert!(request_body.contains("authorAssociation"));
    assert!(request_body.contains("__typename"));
    assert!(request_body.contains("login"));
}

#[test]
fn owner_list_comments_preserves_unattributed_or_unknown_actor_identity() {
    let cases = [
        (
            serde_json::json!({
            "databaseId":501,
            "body":"resolution marker",
            "updatedAt":"2026-07-14T00:00:01Z",
            "authorAssociation":"OWNER"
            }),
            None,
            Some(RepositoryAuthorAssociation::Owner),
        ),
        (
            serde_json::json!({
            "databaseId":501,
            "body":"resolution marker",
            "updatedAt":"2026-07-14T00:00:01Z",
            "author":{"login":"akiojin","__typename":"UnknownActor"},
            "authorAssociation":"OWNER"
            }),
            Some(RepositoryActorType::Unknown("UnknownActor".to_string())),
            Some(RepositoryAuthorAssociation::Owner),
        ),
        (
            serde_json::json!({
            "databaseId":501,
            "body":"resolution marker",
            "updatedAt":"2026-07-14T00:00:01Z",
            "author":{"login":"akiojin","__typename":"User"},
            "authorAssociation":"UNKNOWN"
            }),
            Some(RepositoryActorType::User),
            Some(RepositoryAuthorAssociation::Unknown("UNKNOWN".to_string())),
        ),
    ];

    for (node, expected_type, expected_association) in cases {
        let transport = FakeTransport::new();
        transport.enqueue(ok_body(
            &serde_json::json!({"data":{"repository":{"issue":{"comments":{
                "nodes":[node],
                "pageInfo":{"hasNextPage":false,"endCursor":null}
            }}}}})
            .to_string(),
        ));
        let comments = client_with(transport)
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(42),
                &owner_deadline(),
            )
            .expect("untrusted actor must not abort unrelated comments");
        assert_eq!(comments.items()[0].author_type, expected_type);
        assert_eq!(comments.items()[0].author_association, expected_association);
    }
}

#[test]
fn owner_pagination_rejects_missing_and_repeated_cursors_as_partial() {
    for cursor in [None, Some("same-cursor")] {
        let transport = FakeTransport::new();
        transport.enqueue(owner_page(vec![owner_issue(1, &[])], true, cursor));
        if cursor.is_some() {
            transport.enqueue(owner_page(
                vec![owner_issue(2, &[])],
                true,
                Some("same-cursor"),
            ));
        }
        let client = client_with(transport);
        let error = client
            .list_issues(&RepositoryIdentity::gwt_upstream(), &owner_deadline())
            .expect_err("invalid cursor must not produce a partial collection");
        assert!(matches!(error, ApiError::PartialPage { .. }));
    }
}

#[test]
fn owner_list_comments_rejects_invalid_required_identity_fields() {
    let cases = [(0, "2026-07-14T00:00:01Z"), (501, "not-rfc3339")];

    for (id, updated_at) in cases {
        let transport = FakeTransport::new();
        transport.enqueue(ok_body(
            &serde_json::json!({"data":{"repository":{"issue":{"comments":{
                "nodes":[{
                    "databaseId":id,
                    "body":"resolution marker",
                    "updatedAt":updated_at,
                    "author":{"login":"akiojin","__typename":"User"},
                    "authorAssociation":"OWNER"
                }],
                "pageInfo":{"hasNextPage":false,"endCursor":null}
            }}}}})
            .to_string(),
        ));

        let error = client_with(transport)
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(42),
                &owner_deadline(),
            )
            .expect_err("malformed required comment identity must fail closed");

        assert!(matches!(error, ApiError::Parse { .. }));
    }
}

#[test]
fn expired_owner_deadline_performs_no_http_request() {
    let client = client_with(FakeTransport::new());
    let deadline = ResolutionDeadline::at(
        Instant::now() - Duration::from_millis(1),
        Duration::from_secs(1),
    );
    let error = client
        .list_issues(&RepositoryIdentity::gwt_upstream(), &deadline)
        .expect_err("expired deadline");
    assert!(matches!(error, ApiError::Timeout { .. }));
    assert!(client.transport().recorded().is_empty());
}

#[test]
fn expired_owner_deadline_rejects_auth_before_spawning_gh() {
    let deadline = ResolutionDeadline::at(
        Instant::now() - Duration::from_millis(1),
        Duration::from_secs(1),
    );

    let error = match HttpIssueClient::from_gh_auth_with_deadline("akiojin", "gwt", &deadline) {
        Err(error) => error,
        Ok(_) => panic!("expired auth deadline must fail"),
    };

    assert!(matches!(error, ApiError::Timeout { .. }));
}

#[cfg(unix)]
#[test]
fn stalled_gh_auth_is_terminated_at_the_absolute_deadline() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = PATH_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().expect("fake gh directory");
    let executable = directory.path().join("gh");
    std::fs::write(
        &executable,
        "#!/bin/sh\n/bin/sleep 30\nprintf 'late-token\\n'\n",
    )
    .expect("fake gh script");
    let mut permissions = std::fs::metadata(&executable)
        .expect("fake gh metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&executable, permissions).expect("fake gh permissions");

    let path_env = ScopedEnv::cleared(&["PATH"]);
    path_env.set("PATH", directory.path());
    let started = Instant::now();
    let deadline = ResolutionDeadline::new(Duration::from_millis(100), Duration::from_millis(150));
    let result = HttpIssueClient::from_gh_auth_with_deadline("akiojin", "gwt", &deadline);

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("stalled auth must fail"),
    };
    assert!(matches!(error, ApiError::Timeout { .. }));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn owner_mutations_are_explicitly_targeted_and_close_is_read_back() {
    let transport = FakeTransport::new();
    transport.enqueue(created(
        r#"{"id":501,"body":"occurrence","updated_at":"2026-07-14T00:00:00Z","user":{"login":"akiojin","type":"User"},"author_association":"OWNER"}"#,
    ));
    transport.enqueue(ok_body(
        r#"{"data":{"repository":{"issue":{"comments":{"nodes":[{"databaseId":501,"body":"occurrence","updatedAt":"2026-07-14T00:00:01Z","author":{"login":"akiojin","__typename":"User"},"authorAssociation":"OWNER"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}"#,
    ));
    transport.enqueue(created(
        r#"{"number":88,"title":"Created","body":"Body","state":"open","updated_at":"2026-07-14T00:00:00Z","labels":[]}"#,
    ));
    transport.enqueue(ok_body(
        r#"{"data":{"repository":{"issue":{"number":88,"title":"Created","body":"Body","state":"OPEN","updatedAt":"2026-07-14T00:00:01Z","labels":{"totalCount":0,"nodes":[]}}}}}"#,
    ));
    transport.enqueue(ok_body(
        r#"{"number":88,"title":"Created","body":"Body","state":"closed","updated_at":"2026-07-14T00:00:01Z","labels":[]}"#,
    ));
    transport.enqueue(ok_body(
        r#"{"data":{"repository":{"issue":{"number":88,"title":"Created","body":"Body","state":"CLOSED","updatedAt":"2026-07-14T00:00:01Z","labels":{"totalCount":0,"nodes":[]}}}}}"#,
    ));
    let client = client_with(transport);
    let repository = RepositoryIdentity::gwt_upstream();
    client
        .create_owner_comment(
            &repository,
            IssueNumber(42),
            "occurrence",
            &owner_deadline(),
        )
        .expect("create comment");
    let created = client
        .create_owner_issue(
            &repository,
            &CreateRepositoryIssue {
                title: "Created".to_string(),
                body: "Body".to_string(),
                labels: Vec::new(),
            },
            &owner_deadline(),
        )
        .expect("create issue");
    assert_eq!(created.number, IssueNumber(88));
    let closed = client
        .close_issue_verified(&repository, IssueNumber(88), &owner_deadline())
        .expect("verified close");
    assert_eq!(closed.state, IssueState::Closed);

    let requests = client.transport().recorded();
    assert_eq!(requests.len(), 6);
    assert_eq!(
        requests[0].url,
        "https://api.github.com/repos/akiojin/gwt/issues/42/comments"
    );
    assert_eq!(
        requests[2].url,
        "https://api.github.com/repos/akiojin/gwt/issues"
    );
    assert_eq!(requests[1].url, "https://api.github.com/graphql");
    assert_eq!(requests[3].url, "https://api.github.com/graphql");
    assert_eq!(requests[4].method, HttpMethod::Patch);
    assert_eq!(requests[5].url, "https://api.github.com/graphql");
}

#[test]
fn owner_create_requires_authoritative_readback_match() {
    let transport = FakeTransport::new();
    transport.enqueue(created(
        r#"{"id":501,"body":"occurrence","updated_at":"2026-07-14T00:00:00Z","user":{"login":"akiojin","type":"User"},"author_association":"OWNER"}"#,
    ));
    transport.enqueue(ok_body(
        r#"{"data":{"repository":{"issue":{"comments":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}"#,
    ));
    let client = client_with(transport);

    let error = client
        .create_owner_comment(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(42),
            "occurrence",
            &owner_deadline(),
        )
        .expect_err("missing readback must fail closed");

    assert!(matches!(
        error,
        OwnerMutationError::RemoteOutcomeUnknown(ApiError::Parse { .. })
    ));
}

#[test]
fn owner_mutation_failures_expose_submission_certainty() {
    let input = CreateRepositoryIssue {
        title: "Created".to_string(),
        body: "Body".to_string(),
        labels: Vec::new(),
    };
    let cases = [
        (
            Some(HttpResponse {
                status: 422,
                headers: Vec::new(),
                body: r#"{"message":"body is too long"}"#.to_string(),
            }),
            "pre-submit",
        ),
        (
            Some(HttpResponse {
                status: 503,
                headers: Vec::new(),
                body: r#"{"message":"Service unavailable"}"#.to_string(),
            }),
            "unknown",
        ),
        (None, "unknown"),
    ];
    for (response, expected) in cases {
        let transport = FakeTransport::new();
        if let Some(response) = response {
            transport.enqueue(response);
        }
        let client = client_with(transport);
        let error = client
            .create_owner_issue(
                &RepositoryIdentity::gwt_upstream(),
                &input,
                &owner_deadline(),
            )
            .expect_err("mutation failure");
        match expected {
            "pre-submit" => assert!(matches!(
                error,
                OwnerMutationError::PreSubmit(ApiError::BodyTooLarge)
            )),
            "unknown" => assert!(matches!(
                error,
                OwnerMutationError::RemoteOutcomeUnknown(ApiError::Network(_))
            )),
            _ => unreachable!(),
        }
    }
}

#[test]
fn owner_graphql_and_http_failures_keep_typed_classification() {
    let cases = [
        (
            HttpResponse {
                status: 429,
                headers: Vec::new(),
                body: r#"{"message":"Too many requests"}"#.to_string(),
            },
            "rate-limit",
        ),
        (
            ok_body(
                r#"{"errors":[{"message":"API rate limit exceeded","extensions":{"type":"RATE_LIMITED"}}]}"#,
            ),
            "rate-limit",
        ),
        (
            ok_body(
                r#"{"errors":[{"message":"Bad credentials","extensions":{"code":"UNAUTHORIZED"}}]}"#,
            ),
            "auth",
        ),
        (
            HttpResponse {
                status: 503,
                headers: Vec::new(),
                body: r#"{"message":"Service unavailable"}"#.to_string(),
            },
            "network",
        ),
    ];
    for (response, expected) in cases {
        let transport = FakeTransport::new();
        transport.enqueue(response);
        let client = client_with(transport);
        let error = client
            .list_issues(&RepositoryIdentity::gwt_upstream(), &owner_deadline())
            .expect_err("typed remote failure");
        match expected {
            "rate-limit" => assert!(matches!(error, ApiError::RateLimited { .. })),
            "auth" => assert!(matches!(error, ApiError::Unauthorized)),
            "network" => assert!(matches!(error, ApiError::Network(_))),
            _ => unreachable!(),
        }
    }
}

#[test]
fn owner_pagination_propagates_one_absolute_deadline_to_every_request() {
    let transport = FakeTransport::new();
    transport.enqueue(owner_page(vec![owner_issue(1, &[])], true, Some("next")));
    transport.enqueue(owner_page(vec![owner_issue(2, &[])], false, None));
    let client = client_with(transport);
    let deadline = owner_deadline();

    client
        .list_issues(&RepositoryIdentity::gwt_upstream(), &deadline)
        .expect("complete issue corpus");

    assert_eq!(
        client.transport().recorded_deadlines(),
        vec![deadline.expires_at(), deadline.expires_at()]
    );
}

#[test]
fn owner_history_lookups_parse_pull_request_release_and_commit_comparison() {
    let base = "a".repeat(40);
    let head = "c".repeat(40);
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        r#"{"number":7,"merged":true,"merge_commit_sha":"abc123","merged_at":"2026-07-01T00:00:00Z"}"#,
    ));
    transport.enqueue(ok_body(
        r#"{"tag_name":"v9.66.0","target_commitish":"abc123","published_at":"2026-07-02T00:00:00Z"}"#,
    ));
    transport.enqueue(ok_body(
        &serde_json::json!({
            "status":"ahead",
            "ahead_by":2,
            "behind_by":0,
            "base_commit":{"sha":base},
            "merge_base_commit":{"sha":base},
            "commits":[{"sha":"b".repeat(40)},{"sha":head}]
        })
        .to_string(),
    ));
    let client = client_with(transport);
    let repository = RepositoryIdentity::gwt_upstream();
    let pull = client
        .fetch_merged_pull_request(&repository, IssueNumber(7), &owner_deadline())
        .expect("pull request")
        .expect("merged pull request");
    assert_eq!(pull.merge_commit_sha, "abc123");
    let release = client
        .fetch_release_by_tag(&repository, "v9.66.0", &owner_deadline())
        .expect("release")
        .expect("published release");
    assert_eq!(release.target_commitish, "abc123");
    let comparison = client
        .compare_commits(&repository, &base, &head, &owner_deadline())
        .expect("comparison");
    assert_eq!(comparison.status, CommitComparisonStatus::Ahead);
    assert_eq!(comparison.ahead_by, 2);
    assert_eq!(comparison.base_commit_sha, base);
    assert_eq!(comparison.merge_base_commit_sha, base);
    assert_eq!(comparison.head_commit_sha, head);
}

#[test]
fn owner_comparison_derives_identical_head_from_resolved_base() {
    let commit = "a".repeat(40);
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        &serde_json::json!({
            "status":"identical",
            "ahead_by":0,
            "behind_by":0,
            "base_commit":{"sha":commit},
            "merge_base_commit":{"sha":commit},
            "commits":[]
        })
        .to_string(),
    ));

    let comparison = client_with(transport)
        .compare_commits(
            &RepositoryIdentity::gwt_upstream(),
            &commit,
            &commit,
            &owner_deadline(),
        )
        .expect("identical comparison");

    assert_eq!(comparison.head_commit_sha, commit);
}

#[test]
fn owner_comparison_rejects_non_full_resolved_commit_identities() {
    let valid_base = "a".repeat(40);
    let valid_merge_base = "b".repeat(40);
    let valid_head = "c".repeat(40);
    let cases = [
        (
            "short-base".to_string(),
            valid_merge_base.clone(),
            valid_head.clone(),
        ),
        (
            valid_base.clone(),
            "short-merge-base".to_string(),
            valid_head.clone(),
        ),
        (
            valid_base.clone(),
            valid_merge_base.clone(),
            "short-head".to_string(),
        ),
    ];

    for (base_commit, merge_base_commit, head_commit) in cases {
        let transport = FakeTransport::new();
        transport.enqueue(ok_body(
            &serde_json::json!({
                "status":"diverged",
                "ahead_by":2,
                "behind_by":1,
                "base_commit":{"sha":base_commit},
                "merge_base_commit":{"sha":merge_base_commit},
                "commits":[{"sha":head_commit}]
            })
            .to_string(),
        ));

        let error = client_with(transport)
            .compare_commits(
                &RepositoryIdentity::gwt_upstream(),
                "refs/tags/v9.66.0",
                "refs/heads/develop",
                &owner_deadline(),
            )
            .expect_err("resolved commit identities must be full OIDs");

        assert!(matches!(error, ApiError::Parse { .. }));
    }
}

#[test]
fn owner_comparison_rejects_mismatched_resolved_head_identity() {
    let base = "a".repeat(40);
    let requested_head = "c".repeat(40);
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        &serde_json::json!({
            "status":"ahead",
            "ahead_by":2,
            "behind_by":0,
            "base_commit":{"sha":base},
            "merge_base_commit":{"sha":base},
            "commits":[{"sha":"d".repeat(40)}]
        })
        .to_string(),
    ));

    let error = client_with(transport)
        .compare_commits(
            &RepositoryIdentity::gwt_upstream(),
            &base,
            &requested_head,
            &owner_deadline(),
        )
        .expect_err("resolved head must match the requested full commit SHA");

    assert!(matches!(error, ApiError::Parse { .. }));
}

#[test]
fn owner_comparison_rejects_missing_final_ahead_commit_identity() {
    let base = "a".repeat(40);
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        &serde_json::json!({
            "status":"ahead",
            "ahead_by":2,
            "behind_by":0,
            "base_commit":{"sha":base},
            "merge_base_commit":{"sha":base},
            "commits":[]
        })
        .to_string(),
    ));

    let error = client_with(transport)
        .compare_commits(
            &RepositoryIdentity::gwt_upstream(),
            &base,
            "refs/tags/v9.66.0",
            &owner_deadline(),
        )
        .expect_err("ahead comparison must resolve its final head commit");

    assert!(matches!(error, ApiError::Parse { .. }));
}

#[test]
fn owner_comparison_rejects_ahead_head_equal_to_base() {
    let base = "a".repeat(40);
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        &serde_json::json!({
            "status":"ahead",
            "ahead_by":1,
            "behind_by":0,
            "base_commit":{"sha":base},
            "merge_base_commit":{"sha":base},
            "commits":[{"sha":base}]
        })
        .to_string(),
    ));

    let error = client_with(transport)
        .compare_commits(
            &RepositoryIdentity::gwt_upstream(),
            &base,
            "refs/heads/develop",
            &owner_deadline(),
        )
        .expect_err("ahead comparison must resolve head beyond base");

    assert!(matches!(error, ApiError::Parse { .. }));
}

#[test]
fn owner_history_rejects_missing_merged_flag() {
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        r#"{"number":7,"merge_commit_sha":"abc123","merged_at":"2026-07-01T00:00:00Z"}"#,
    ));
    let error = client_with(transport)
        .fetch_merged_pull_request(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(7),
            &owner_deadline(),
        )
        .expect_err("missing merged identity must fail closed");

    assert!(matches!(error, ApiError::Parse { .. }));
}

#[test]
fn owner_history_rejects_mismatched_pull_request_identity() {
    let pull_transport = FakeTransport::new();
    pull_transport.enqueue(ok_body(
        r#"{"number":8,"merged":true,"merge_commit_sha":"abc123","merged_at":"2026-07-01T00:00:00Z"}"#,
    ));
    let pull_error = client_with(pull_transport)
        .fetch_merged_pull_request(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(7),
            &owner_deadline(),
        )
        .expect_err("mismatched pull request must fail closed");
    assert!(matches!(pull_error, ApiError::Parse { .. }));
}

#[test]
fn owner_history_validates_pull_request_identity_before_unmerged_outcome() {
    for body in [r#"{"merged":false}"#, r#"{"number":8,"merged":false}"#] {
        let transport = FakeTransport::new();
        transport.enqueue(ok_body(body));

        let error = client_with(transport)
            .fetch_merged_pull_request(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(7),
                &owner_deadline(),
            )
            .expect_err("unmerged response must still identify the requested pull request");

        assert!(matches!(error, ApiError::Parse { .. }), "{body}");
    }
}

#[test]
fn owner_history_rejects_mismatched_release_identity() {
    let release_transport = FakeTransport::new();
    release_transport.enqueue(ok_body(
        r#"{"tag_name":"v9.65.0","target_commitish":"develop","published_at":"2026-07-02T00:00:00Z"}"#,
    ));
    let release_error = client_with(release_transport)
        .fetch_release_by_tag(
            &RepositoryIdentity::gwt_upstream(),
            "v9.66.0",
            &owner_deadline(),
        )
        .expect_err("mismatched release must fail closed");
    assert!(matches!(release_error, ApiError::Parse { .. }));
}

#[test]
fn owner_comparison_rejects_mismatched_resolved_base_identity() {
    let requested_base = "a".repeat(40);
    let resolved_base = "b".repeat(40);
    let resolved_head = "c".repeat(40);
    let transport = FakeTransport::new();
    transport.enqueue(ok_body(
        &serde_json::json!({
            "status":"ahead",
            "ahead_by":2,
            "behind_by":0,
            "base_commit":{"sha":resolved_base},
            "merge_base_commit":{"sha":"b".repeat(40)},
            "commits":[{"sha":resolved_head}]
        })
        .to_string(),
    ));

    let error = client_with(transport)
        .compare_commits(
            &RepositoryIdentity::gwt_upstream(),
            &requested_base,
            &"c".repeat(40),
            &owner_deadline(),
        )
        .expect_err("resolved base must match the requested full commit SHA");

    assert!(matches!(error, ApiError::Parse { .. }));
}

#[test]
fn owner_comparison_rejects_malformed_forward_ancestry_shape() {
    let base = "a".repeat(40);
    let resolved_head = "c".repeat(40);
    let malformed = [
        ("ahead", 0, 0, base.clone()),
        ("ahead", 2, 1, base.clone()),
        ("ahead", 2, 0, "b".repeat(40)),
        ("identical", 1, 0, base.clone()),
    ];

    for (status, ahead_by, behind_by, merge_base) in malformed {
        let transport = FakeTransport::new();
        transport.enqueue(ok_body(
            &serde_json::json!({
                "status":status,
                "ahead_by":ahead_by,
                "behind_by":behind_by,
                "base_commit":{"sha":base},
                "merge_base_commit":{"sha":merge_base},
                "commits":[{"sha":resolved_head}]
            })
            .to_string(),
        ));
        let error = client_with(transport)
            .compare_commits(
                &RepositoryIdentity::gwt_upstream(),
                &base,
                "refs/heads/develop",
                &owner_deadline(),
            )
            .expect_err("malformed ancestry response must fail closed");
        assert!(matches!(error, ApiError::Parse { .. }));
    }
}

#[test]
fn owner_test_endpoint_override_requires_explicit_loopback_mode() {
    let rejected = client_with(FakeTransport::new()).with_test_endpoints(
        "http://127.0.0.1:43123",
        "http://127.0.0.1:43123/graphql",
        false,
    );
    assert!(matches!(
        rejected,
        Err(ApiError::TestOverrideRejected { .. })
    ));

    let remote = client_with(FakeTransport::new()).with_test_endpoints(
        "https://example.com",
        "https://example.com/graphql",
        true,
    );
    assert!(matches!(remote, Err(ApiError::TestOverrideRejected { .. })));

    let loopback = client_with(FakeTransport::new()).with_test_endpoints(
        "http://127.0.0.1:43123",
        "http://localhost:43123/graphql",
        true,
    );
    if cfg!(debug_assertions) {
        assert!(loopback.is_ok());
    } else {
        assert!(matches!(
            loopback,
            Err(ApiError::TestOverrideRejected { .. })
        ));
    }
}

#[test]
fn owner_environment_override_requires_complete_debug_loopback_contract() {
    let _lock = OWNER_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let env = ScopedEnv::cleared(&OWNER_ENV_KEYS);
    env.set(OWNER_ENV_KEYS[0], "loopback-v1");

    let incomplete =
        HttpIssueClient::from_owner_environment_with_deadline("akiojin", "gwt", &owner_deadline());
    assert!(matches!(
        incomplete,
        Err(ApiError::TestOverrideRejected { .. })
    ));

    env.set(OWNER_ENV_KEYS[1], "http://127.0.0.1:43123");
    env.set(OWNER_ENV_KEYS[2], "http://localhost:43123/graphql");
    env.set(OWNER_ENV_KEYS[3], "test-owner-token");
    let complete =
        HttpIssueClient::from_owner_environment_with_deadline("akiojin", "gwt", &owner_deadline());
    if cfg!(debug_assertions) {
        assert!(complete.is_ok());
    } else {
        assert!(matches!(
            complete,
            Err(ApiError::TestOverrideRejected { .. })
        ));
    }

    env.set(OWNER_ENV_KEYS[3], " ");
    let empty_token =
        HttpIssueClient::from_owner_environment_with_deadline("akiojin", "gwt", &owner_deadline());
    assert!(matches!(
        empty_token,
        Err(ApiError::TestOverrideRejected { .. })
    ));
}
