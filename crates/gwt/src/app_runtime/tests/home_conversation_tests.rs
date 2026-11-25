//! Issue #7639 (SPEC #7007 T-021): the Home conversation on the Hub connection.

use super::*;

fn home_runtime(
    temp: &Path,
) -> (
    AppRuntime,
    Arc<Mutex<Vec<UserEvent>>>,
    BlockingTestTaskQueue,
    String,
) {
    let tab = sample_project_tab("tab-1", "Repo", temp.into(), ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp, vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let project_key = runtime.test_context().project_key.to_string();
    (runtime, recorded, tasks, project_key)
}

fn hub_event(runtime: &mut AppRuntime, event: serde_json::Value) -> Vec<serde_json::Value> {
    runtime
        .handle_frontend_event_in_scope(
            "home".into(),
            serde_json::from_value(event).expect("frontend event"),
            &ClientScope::Hub,
        )
        .iter()
        .map(|event| serde_json::to_value(&event.event).expect("serialize"))
        .collect()
}

#[test]
fn home_reads_the_decisions_of_an_open_project_without_its_other_reports() {
    use gwt_core::pm_report::{post_report, PmReportKind};
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    post_report(temp.path(), PmReportKind::Progress, "Work is moving.", None).unwrap();
    post_report(
        temp.path(),
        PmReportKind::Decision,
        "Ship the release?",
        None,
    )
    .unwrap();
    let (mut runtime, recorded, tasks, project_key) = home_runtime(temp.path());

    let immediate = hub_event(
        &mut runtime,
        serde_json::json!({"kind": "load_home_conversation", "request_id": 7}),
    );
    assert!(immediate.is_empty(), "the store is read off the event loop");
    let task = tasks.lock().unwrap().remove(0);
    task();

    let replies: Vec<serde_json::Value> = recorded
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            UserEvent::Dispatch(events) => Some(events.clone()),
            _ => None,
        })
        .flatten()
        .filter(|event| event.target == DispatchTarget::Client("home".into()))
        .map(|event| serde_json::to_value(&event.event).unwrap())
        .filter(|value| value["kind"] == "home_conversation")
        .collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["request_id"], 7);
    let project = &replies[0]["projects"][0];
    assert_eq!(project["project_key"], project_key.as_str());
    assert_eq!(project["title"], "Repo");
    assert_eq!(project["pm_available"], false);
    // Issue #7742: the decisions are the project's decision queue; a decision
    // the PM raised through a report is bridged into it.
    let decisions = project["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["question"], "Ship the release?");
    assert_eq!(project["waiting_count"], 1);
}

fn decision_results(recorded: &Arc<Mutex<Vec<UserEvent>>>) -> Vec<serde_json::Value> {
    recorded
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            UserEvent::Dispatch(events) => Some(events.clone()),
            _ => None,
        })
        .flatten()
        .filter(|event| event.target == DispatchTarget::Client("home".into()))
        .map(|event| serde_json::to_value(&event.event).unwrap())
        .filter(|value| value["kind"] == "decision_result")
        .collect()
}

/// Issue #7742 AC-2 / AC-4: Home answers and defers through the project's
/// decision queue. No PM is registered here, and the answer is still recorded.
#[test]
fn home_answers_and_defers_through_the_queue_without_a_pm() {
    use gwt_core::decision_queue as queue;
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let register = |target: &str| {
        queue::register(
            temp.path(),
            &queue::Registration {
                issue: Some(7742),
                kind: Some("product".into()),
                target: Some(target.into()),
                question: Some("Which storage should the queue use?".into()),
                materials: Some("Both options pass the existing tests.".into()),
                options: vec!["Keep JSON".into(), "Move to SQLite".into()],
                recommendation: Some("Keep JSON".into()),
                reason: None,
                session_id: Some("waiting-agent".into()),
            },
        )
        .unwrap()
        .item
        .id
    };
    let (older, newer) = (register("storage"), register("ordering"));
    let (mut runtime, recorded, tasks, project_key) = home_runtime(temp.path());

    let immediate = hub_event(
        &mut runtime,
        serde_json::json!({"kind": "home_defer_decision", "project_key": project_key,
            "decision_id": newer, "request_id": 4}),
    );
    assert!(
        immediate.is_empty(),
        "the queue is written off the event loop"
    );
    tasks.lock().unwrap().remove(0)();
    let results = decision_results(&recorded);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["id"], project_key.as_str());
    assert_eq!(results[0]["request_id"], 4);
    assert_eq!(results[0]["action"], "defer");
    assert_eq!(results[0]["ok"], true);
    assert_eq!(
        results[0]["decisions"]["waiting_count"], 2,
        "deferring is not a decision"
    );
    assert_eq!(
        results[0]["decisions"]["items"][1]["id"],
        newer.as_str(),
        "deferred to the tail"
    );

    hub_event(
        &mut runtime,
        serde_json::json!({"kind": "home_answer_decision", "project_key": project_key,
            "decision_id": older, "answer": "1. Keep JSON", "request_id": 5}),
    );
    tasks.lock().unwrap().remove(0)();
    let results = decision_results(&recorded);
    assert_eq!(results[1]["action"], "answer");
    assert_eq!(results[1]["ok"], true);
    assert_eq!(results[1]["decisions"]["waiting_count"], 1);
    let pending = queue::pending_deliveries(temp.path()).unwrap();
    assert_eq!(
        pending.len(),
        1,
        "gwtd owes the answer to the waiting session"
    );
    assert!(pending[0].message.contains("1. Keep JSON"));

    // A project that is not open is refused out loud, and so is a project
    // connection borrowing the Home request.
    let refused = hub_event(
        &mut runtime,
        serde_json::json!({"kind": "home_answer_decision", "project_key": "ffffffffffffffff",
            "decision_id": newer, "answer": "x", "request_id": 6}),
    );
    assert_eq!(refused[0]["kind"], "decision_result");
    assert_eq!(refused[0]["ok"], false);
    assert!(refused[0]["error"].as_str().unwrap().contains("not open"));
    let scope = ClientScope::Project(runtime.test_context().project_key.clone());
    let replies = runtime.handle_frontend_event_in_scope(
        "project".into(),
        serde_json::from_value(serde_json::json!({"kind": "home_defer_decision",
            "project_key": project_key, "decision_id": newer, "request_id": 7}))
        .unwrap(),
        &scope,
    );
    let value = serde_json::to_value(&replies[0].event).unwrap();
    assert_eq!(value["kind"], "decision_result");
    assert_eq!(value["ok"], false);
    assert_eq!(queue::snapshot(temp.path()).unwrap().waiting_count, 1);
}

#[test]
fn home_send_is_refused_out_loud_when_no_pm_can_receive_it() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, _recorded, _tasks, project_key) = home_runtime(temp.path());

    for (key, expected) in [
        (project_key.as_str(), "No Project Manager is running"),
        ("ffffffffffffffff", "This project is not open"),
    ] {
        let replies = hub_event(
            &mut runtime,
            serde_json::json!({"kind": "home_pm_send", "project_key": key,
                "text": "Approve it\r", "request_id": 9}),
        );
        assert_eq!(replies.len(), 1, "{key}");
        assert_eq!(replies[0]["kind"], "pane_send_result");
        assert_eq!(replies[0]["ok"], false);
        assert_eq!(replies[0]["request_id"], 9);
        assert!(
            replies[0]["error"].as_str().unwrap().contains(expected),
            "{key}: {}",
            replies[0]["error"]
        );
    }
}

#[test]
fn a_project_connection_cannot_use_the_home_send() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, _recorded, _tasks, project_key) = home_runtime(temp.path());
    let scope = ClientScope::Project(runtime.test_context().project_key.clone());
    let replies = runtime.handle_frontend_event_in_scope(
        "project".into(),
        serde_json::from_value(serde_json::json!({"kind": "home_pm_send",
            "project_key": project_key, "text": "Approve it\r", "request_id": 2}))
        .unwrap(),
        &scope,
    );
    let value = serde_json::to_value(&replies[0].event).unwrap();
    assert_eq!(value["kind"], "pane_send_result");
    assert_eq!(value["ok"], false);
}

/// Issue #7654: a project page reads the waiting count of every open
/// project's decision queue, with no PM window, so its header and status bar
/// count what Home shows.
#[test]
fn a_project_connection_reads_the_decision_counts_without_a_pm_window() {
    use gwt_core::decision_queue as queue;
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    for target in ["storage", "ordering", "naming"] {
        queue::register(
            temp.path(),
            &queue::Registration {
                issue: Some(7654),
                kind: Some("product".into()),
                target: Some(target.into()),
                question: Some(format!("Which {target}?")),
                materials: Some("Both options work.".into()),
                options: vec!["Keep".into(), "Change".into()],
                recommendation: Some("Keep".into()),
                reason: None,
                session_id: Some("waiting-agent".into()),
            },
        )
        .unwrap();
    }
    let (mut runtime, recorded, tasks, project_key) = home_runtime(temp.path());
    let scope = ClientScope::Project(runtime.test_context().project_key.clone());
    let immediate = runtime.handle_frontend_event_in_scope(
        "project".into(),
        serde_json::from_value(
            serde_json::json!({"kind": "load_decision_counts", "request_id": 4}),
        )
        .unwrap(),
        &scope,
    );
    assert!(
        immediate.is_empty(),
        "the queues are read off the event loop"
    );
    let task = tasks.lock().unwrap().remove(0);
    task();

    let replies: Vec<serde_json::Value> = recorded
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            UserEvent::Dispatch(events) => Some(events.clone()),
            _ => None,
        })
        .flatten()
        .filter(|event| event.target == DispatchTarget::Client("project".into()))
        .map(|event| serde_json::to_value(&event.event).unwrap())
        .filter(|value| value["kind"] == "decision_counts")
        .collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["request_id"], 4);
    assert_eq!(
        replies[0]["projects"],
        serde_json::json!([{"project_key": project_key, "waiting_count": 3}])
    );
}
