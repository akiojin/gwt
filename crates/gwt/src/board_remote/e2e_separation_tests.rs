//! End-to-end Board separation test (SPEC-2963 FR-027).
//!
//! Confirms that two projects mapped to two different Slack channels do **not**
//! mix — through the *real* runtime path: the production blocking `reqwest`
//! [`ReqwestHttpClient`] and the real [`SlackProvider`] driving an actual HTTP
//! round-trip (post → read) against a local Slack-compatible server. This is the
//! closest credential-free equivalent of a live Slack E2E: it exercises the same
//! wire path a real Slack workspace would, only the server is local.
//!
//! Project A posts to channel `C-A`, project B to `C-B`. The assertions prove
//! that A reads back only A's content and B only B's, and that the server
//! received each project's posts on its own channel exclusively.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Form, Query, State},
    routing::{get, post},
    Json, Router,
};
use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind, BoardProvider};
use serde_json::{json, Value};

use crate::board_remote::http::ReqwestHttpClient;
use crate::board_remote::slack::SlackProvider;

/// A single stored Slack message.
#[derive(Clone)]
struct Msg {
    ts: String,
    text: String,
    thread_ts: Option<String>,
}

/// Minimal stateful Slack-compatible server: records every posted message per
/// channel and serves thread replies back, so a full post → read round-trip
/// works against it.
#[derive(Default)]
struct FakeSlack {
    counter: AtomicU64,
    /// channel id → posted messages (in order).
    messages: Mutex<HashMap<String, Vec<Msg>>>,
}

impl FakeSlack {
    fn next_ts(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        // Slack-style `secs.micros` so the provider can parse a timestamp.
        format!("17000000{n:02}.000100")
    }

    fn push(&self, channel: &str, msg: Msg) {
        self.messages
            .lock()
            .unwrap()
            .entry(channel.to_string())
            .or_default()
            .push(msg);
    }

    /// The root message plus its replies for `(channel, root_ts)`.
    fn thread(&self, channel: &str, root_ts: &str) -> Vec<Value> {
        let guard = self.messages.lock().unwrap();
        guard
            .get(channel)
            .map(|msgs| {
                msgs.iter()
                    .filter(|m| m.ts == root_ts || m.thread_ts.as_deref() == Some(root_ts))
                    .map(|m| {
                        json!({
                            "ts": m.ts,
                            "text": m.text,
                            "thread_ts": m.thread_ts.clone().unwrap_or_else(|| root_ts.to_string()),
                            "username": "e2e",
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Channels that received at least one post.
    fn channels(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.messages.lock().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    /// All message texts posted to `channel`.
    fn texts(&self, channel: &str) -> Vec<String> {
        self.messages
            .lock()
            .unwrap()
            .get(channel)
            .map(|msgs| msgs.iter().map(|m| m.text.clone()).collect())
            .unwrap_or_default()
    }
}

async fn handle_post_message(
    State(store): State<Arc<FakeSlack>>,
    Form(form): Form<HashMap<String, String>>,
) -> Json<Value> {
    let channel = form.get("channel").cloned().unwrap_or_default();
    let text = form.get("text").cloned().unwrap_or_default();
    let thread_ts = form.get("thread_ts").cloned();
    let ts = store.next_ts();
    store.push(
        &channel,
        Msg {
            ts: ts.clone(),
            text,
            thread_ts,
        },
    );
    Json(json!({ "ok": true, "ts": ts, "channel": channel }))
}

async fn handle_update(Form(form): Form<HashMap<String, String>>) -> Json<Value> {
    Json(json!({ "ok": true, "ts": form.get("ts").cloned().unwrap_or_default() }))
}

async fn handle_history(Query(_q): Query<HashMap<String, String>>) -> Json<Value> {
    // The provider derives thread roots from its local JSONL store, not from
    // flat history, so an empty history is sufficient.
    Json(json!({ "ok": true, "messages": [] }))
}

async fn handle_replies(
    State(store): State<Arc<FakeSlack>>,
    Query(q): Query<HashMap<String, String>>,
) -> Json<Value> {
    let channel = q.get("channel").cloned().unwrap_or_default();
    let ts = q.get("ts").cloned().unwrap_or_default();
    Json(json!({ "ok": true, "messages": store.thread(&channel, &ts) }))
}

fn entry(body: &str) -> BoardEntry {
    BoardEntry::new(
        AuthorKind::User,
        "You",
        BoardEntryKind::Status,
        body,
        None,
        None,
        vec![],
        vec![],
    )
}

#[test]
fn two_projects_on_two_channels_do_not_mix_over_real_http() {
    let store = Arc::new(FakeSlack::default());

    // Start the local Slack-compatible server on its own runtime/thread.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener =
        rt.block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap() });
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/chat.postMessage", post(handle_post_message))
        .route("/chat.update", post(handle_update))
        .route("/conversations.history", get(handle_history))
        .route("/conversations.replies", get(handle_replies))
        .with_state(store.clone());
    rt.spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");

    // Two projects, two channels, the REAL blocking reqwest client.
    let provider_a = SlackProvider::new_with_base(
        &base,
        "xoxb-a",
        "C-A",
        std::collections::BTreeMap::new(),
        Box::new(ReqwestHttpClient::new()),
        60,
    );
    let provider_b = SlackProvider::new_with_base(
        &base,
        "xoxb-b",
        "C-B",
        std::collections::BTreeMap::new(),
        Box::new(ReqwestHttpClient::new()),
        60,
    );

    let root_a = tempfile::tempdir().unwrap();
    let root_b = tempfile::tempdir().unwrap();

    // Each project posts a distinct, identifiable message.
    provider_a
        .post_entry(root_a.path(), entry("alpha-secret-body"))
        .expect("project A post");
    provider_b
        .post_entry(root_b.path(), entry("beta-secret-body"))
        .expect("project B post");

    // Read each project's Board back over the wire.
    let snap_a = provider_a.load_snapshot(root_a.path()).expect("A read");
    let snap_b = provider_b.load_snapshot(root_b.path()).expect("B read");

    let bodies_a: String = snap_a
        .board
        .entries
        .iter()
        .map(|e| e.body.clone())
        .collect();
    let bodies_b: String = snap_b
        .board
        .entries
        .iter()
        .map(|e| e.body.clone())
        .collect();

    // Project A sees only its own content; project B's never leaks in.
    assert!(
        bodies_a.contains("alpha-secret-body"),
        "A must read its own post; got: {bodies_a}"
    );
    assert!(
        !bodies_a.contains("beta-secret-body"),
        "A must NOT read B's post; got: {bodies_a}"
    );
    assert!(
        bodies_b.contains("beta-secret-body"),
        "B must read its own post; got: {bodies_b}"
    );
    assert!(
        !bodies_b.contains("alpha-secret-body"),
        "B must NOT read A's post; got: {bodies_b}"
    );

    // The server received each project's posts on its own channel exclusively.
    assert_eq!(store.channels(), vec!["C-A".to_string(), "C-B".to_string()]);
    assert!(store.texts("C-A").iter().all(|t| !t.contains("beta")));
    assert!(store.texts("C-B").iter().all(|t| !t.contains("alpha")));
}
