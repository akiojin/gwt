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
    let decisions = project["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["body"], "Ship the release?");
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
