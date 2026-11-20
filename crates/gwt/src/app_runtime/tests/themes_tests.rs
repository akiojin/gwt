//! Issue #7643 (SPEC #7007 T-025): the Themes screen of a project.

use super::*;
use gwt_github::client::{ThemeIssue, ThemeSubIssue};

fn theme(number: u64, sub_issues: &[(u64, IssueState)]) -> ThemeIssue {
    ThemeIssue {
        number: IssueNumber(number),
        title: format!("Theme {number}"),
        body: "What the theme is for.".to_string(),
        url: format!("https://github.com/example/repo/issues/{number}"),
        sub_issues_total: sub_issues.len() as u64,
        sub_issues_completed: sub_issues
            .iter()
            .filter(|(_, state)| *state == IssueState::Closed)
            .count() as u64,
        sub_issues: sub_issues
            .iter()
            .map(|(number, state)| ThemeSubIssue {
                number: IssueNumber(*number),
                title: format!("Issue {number}"),
                state: *state,
                url: format!("https://github.com/example/repo/issues/{number}"),
                in_repository: true,
            })
            .collect(),
    }
}

struct ThemesFixture {
    runtime: AppRuntime,
    recorded: Arc<Mutex<Vec<UserEvent>>>,
    tasks: BlockingTestTaskQueue,
    client: Arc<FakeIssueClient>,
    repo: PathBuf,
}

fn themes_fixture(temp: &Path, git: bool) -> ThemesFixture {
    let repo = temp.join("repo");
    fs::create_dir_all(&repo).expect("repo");
    if git {
        init_repo(&repo);
    }
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp, vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let client = Arc::new(FakeIssueClient::new());
    runtime.issue_client_factory = Arc::new({
        let client = Arc::clone(&client);
        move |_owner, _repo| {
            let client: Arc<dyn IssueClient> = client.clone();
            Ok(client)
        }
    });
    ThemesFixture {
        runtime,
        recorded,
        tasks,
        client,
        repo,
    }
}

/// Send one request from the project page and return its off-loop reply.
fn themes_reply(fixture: &mut ThemesFixture, event: serde_json::Value) -> serde_json::Value {
    let scope = ClientScope::Project(fixture.runtime.test_context().project_key.clone());
    let immediate = fixture.runtime.handle_frontend_event_in_scope(
        "themes".into(),
        serde_json::from_value(event).expect("frontend event"),
        &scope,
    );
    assert!(immediate.is_empty(), "GitHub is read off the event loop");
    let task = fixture.tasks.lock().unwrap().remove(0);
    task();
    let mut replies: Vec<serde_json::Value> = fixture
        .recorded
        .lock()
        .unwrap()
        .drain(..)
        // The reply is bound to the project it was asked for, so it is
        // dropped if that project is gone by the time it arrives.
        .filter_map(|event| match event {
            UserEvent::ProjectCompletion { event, .. } => match *event {
                UserEvent::Dispatch(events) => Some(events),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .filter(|event| event.target == DispatchTarget::Client("themes".into()))
        .map(|event| serde_json::to_value(&event.event).unwrap())
        .filter(|value| value["kind"] == "themes" || value["kind"] == "theme_error")
        .collect();
    assert_eq!(replies.len(), 1, "one reply to the requesting client");
    replies.remove(0)
}

#[test]
fn themes_screen_reads_themes_and_what_was_completed_today() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = themes_fixture(temp.path(), true);
    fixture.client.seed_themes(vec![theme(
        7,
        &[(8, IssueState::Open), (9, IssueState::Closed)],
    )]);
    // The cache takes in one close today.
    let cache = Cache::new(issue_cache_root(&fixture.repo));
    let mut issue = sample_issue_snapshot(9, "Issue 9", &[], "body", "2026-04-20T00:00:00Z");
    cache.write_snapshot(&issue).unwrap();
    issue.state = IssueState::Closed;
    cache.write_snapshot(&issue).unwrap();

    let reply = themes_reply(
        &mut fixture,
        serde_json::json!({"kind": "load_themes", "request_id": 3}),
    );

    assert_eq!(reply["kind"], "themes");
    assert_eq!(reply["request_id"], 3);
    assert_eq!(reply["view"]["completed_today"], 1);
    let themes = reply["view"]["themes"].as_array().unwrap();
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0]["number"], 7);
    assert_eq!(themes[0]["issues_total"], 2);
    assert_eq!(themes[0]["issues_completed"], 1);
    assert_eq!(themes[0]["issues"][1]["state"], "closed");
}

#[test]
fn deleting_a_theme_deletes_its_issue_and_answers_with_the_remaining_themes() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = themes_fixture(temp.path(), true);
    fixture.client.seed_themes(vec![
        theme(7, &[(8, IssueState::Open)]),
        theme(20, &[(21, IssueState::Open)]),
    ]);

    let reply = themes_reply(
        &mut fixture,
        serde_json::json!({"kind": "delete_theme", "number": 7, "request_id": 4}),
    );

    assert_eq!(reply["kind"], "themes");
    assert_eq!(reply["request_id"], 4);
    let numbers: Vec<u64> = reply["view"]["themes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|theme| theme["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, vec![20]);
    assert!(fixture
        .client
        .call_log()
        .contains(&"delete_issue:#7".to_string()));
}

#[test]
fn deleting_an_issue_that_is_not_a_theme_is_refused() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = themes_fixture(temp.path(), true);
    fixture
        .client
        .seed_themes(vec![theme(7, &[(8, IssueState::Open)])]);

    let reply = themes_reply(
        &mut fixture,
        serde_json::json!({"kind": "delete_theme", "number": 8, "request_id": 5}),
    );

    assert_eq!(reply["kind"], "theme_error");
    assert_eq!(reply["request_id"], 5);
    assert!(!fixture
        .client
        .call_log()
        .iter()
        .any(|call| call.starts_with("delete_issue")));
}

#[test]
fn themes_screen_says_why_a_project_without_github_has_no_themes() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = themes_fixture(temp.path(), false);

    let reply = themes_reply(
        &mut fixture,
        serde_json::json!({"kind": "load_themes", "request_id": 6}),
    );

    assert_eq!(reply["kind"], "theme_error");
    assert_eq!(reply["request_id"], 6);
    assert!(!reply["message"].as_str().unwrap().is_empty());
}
