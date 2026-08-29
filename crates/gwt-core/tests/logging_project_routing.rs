//! Issue #3617 / SPEC-1924 US-17 — project log routing contract.
//!
//! This test owns its integration-test process because `logging::init`
//! installs the global tracing subscriber exactly once.

use std::{
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

use gwt_core::logging::{init, LogLevel, LoggingConfig, ProjectLogRouter, LOG_FILE_BASENAME};

fn read_messages(log_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return Vec::new();
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(LOG_FILE_BASENAME))
        })
        .collect::<Vec<_>>();
    paths.sort();

    paths
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|contents| {
            contents
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter_map(|record| {
                    record
                        .get("fields")?
                        .get("message")?
                        .as_str()
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn assert_exact_destination(
    marker: &str,
    expected: &str,
    machine: &[String],
    project_a: &[String],
    project_b: &[String],
) {
    let destinations = [
        ("machine", machine),
        ("project-a", project_a),
        ("project-b", project_b),
    ];
    let present = destinations
        .iter()
        .filter(|(_, messages)| messages.iter().any(|message| message == marker))
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    assert_eq!(
        present,
        vec![expected],
        "{marker} must have exactly one destination"
    );
}

fn register_concurrently(
    router: &ProjectLogRouter,
    project: &Path,
) -> Vec<gwt_core::logging::ProjectLogScope> {
    let workers = 8;
    let barrier = Arc::new(Barrier::new(workers));
    let mut joins = Vec::with_capacity(workers);
    for _ in 0..workers {
        let router = router.clone();
        let project = project.to_path_buf();
        let barrier = barrier.clone();
        joins.push(thread::spawn(move || {
            barrier.wait();
            router
                .register_project(&project)
                .expect("concurrent project registration")
        }));
    }
    joins
        .into_iter()
        .map(|join| join.join().expect("registration worker"))
        .collect()
}

#[test]
fn events_route_by_emission_scope_without_following_the_active_project() {
    std::env::remove_var("RUST_LOG");
    let fixture = tempfile::tempdir().expect("routing fixture");
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(fixture.path());
    let machine_dir = fixture.path().join("machine-logs");
    let project_a = fixture.path().join("project-a");
    let project_b = fixture.path().join("project-b");
    std::fs::create_dir_all(&project_a).expect("project A");
    std::fs::create_dir_all(&project_b).expect("project B");

    let config = LoggingConfig {
        log_dir: machine_dir.clone(),
        default_level: LogLevel::Debug,
        config_file_level: None,
        retention_days: 0,
    };
    let mut handles = init(config).expect("machine logging init");
    let mut ui_rx = handles.take_ui_rx().expect("live log receiver");
    let router = handles.router();

    let registrations = register_concurrently(&router, &project_a);
    let scope_a = registrations.first().expect("scope A").clone();
    assert!(
        registrations.iter().all(|scope| scope == &scope_a),
        "concurrent registration must return one stable scope"
    );
    assert_eq!(
        scope_a.log_dir(),
        gwt_core::paths::gwt_project_logs_dir_for_project_path(&project_a),
        "registration must use the canonical project store"
    );

    // Race the first write as well as registration. A registry that opens one
    // appender per caller can split, duplicate, or lose these records even if
    // it returned equal scope tokens above.
    let concurrent_write_count = registrations.len();
    let first_write_barrier = Arc::new(Barrier::new(concurrent_write_count));
    let first_writes = registrations
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, scope)| {
            let barrier = first_write_barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                let _scope = scope.enter();
                tracing::info!(
                    target: "gwt_core::logging::routing_test",
                    marker_index = index,
                    "marker-a-concurrent-first-write"
                );
            })
        })
        .collect::<Vec<_>>();
    for write in first_writes {
        write.join().expect("concurrent first project write");
    }

    let scope_b = router
        .register_project(&project_b)
        .expect("register project B");
    assert_eq!(
        scope_b.log_dir(),
        gwt_core::paths::gwt_project_logs_dir_for_project_path(&project_b),
        "registration must use the canonical project store"
    );

    tracing::info!(
        target: "gwt_core::logging::routing_test",
        gwt_project_scope = scope_a.as_str(),
        "marker-a-direct"
    );

    {
        // Use a plain tracing span rather than ProjectLogScope::enter so this
        // separately fixes the nearest-span fallback contract (no TLS helper).
        let span = tracing::info_span!(
            target: "gwt_core::logging::routing_test",
            "project-b-operation",
            gwt_project_scope = scope_b.as_str()
        );
        let _entered = span.enter();
        tracing::info!(
            target: "gwt_core::logging::routing_test",
            "marker-b-span"
        );
        tracing::info!(
            target: "gwt_core::logging::routing_test",
            gwt_project_scope = scope_a.as_str(),
            "marker-a-direct-overrides-b-span"
        );
    }

    let background_scope = scope_a.clone();
    let background = thread::spawn(move || {
        let _scope = background_scope.enter();
        tracing::info!(
            target: "gwt_core::logging::routing_test",
            "marker-a-background-after-b-selected"
        );
    });
    {
        let _scope = scope_b.enter();
        tracing::info!(
            target: "gwt_core::logging::routing_test",
            "marker-b-foreground"
        );
    }
    background.join().expect("project A background event");

    tracing::info!(
        target: "gwt_core::logging::routing_test",
        "marker-machine-unscoped"
    );
    tracing::info!(
        target: "gwt_core::logging::routing_test",
        gwt_project_scope = "ffffffffffffffff",
        "marker-machine-unknown-scope"
    );

    handles
        .set_level(LogLevel::Warn)
        .expect("reload logging filter");
    {
        let _scope = scope_a.enter();
        tracing::warn!(
            target: "gwt_core::logging::routing_test",
            "marker-a-after-filter-reload"
        );
    }

    let mut live = Vec::new();
    while let Ok(event) = ui_rx.try_recv() {
        live.push(event);
    }
    assert!(live.iter().any(|event| {
        event.message == "marker-a-direct"
            && event.project_scope.as_deref() == Some(scope_a.as_str())
    }));
    assert!(live.iter().any(|event| {
        event.message == "marker-b-span" && event.project_scope.as_deref() == Some(scope_b.as_str())
    }));
    assert!(live.iter().any(|event| {
        event.message == "marker-machine-unscoped" && event.project_scope.is_none()
    }));
    assert!(live.iter().any(|event| {
        event.message == "marker-machine-unknown-scope" && event.project_scope.is_none()
    }));

    let project_a_dir = scope_a.log_dir().to_path_buf();
    let project_b_dir = scope_b.log_dir().to_path_buf();
    drop(registrations);
    drop(scope_a);
    drop(scope_b);
    drop(router);
    drop(handles);

    let unknown_project_dir = fixture
        .path()
        .join(".gwt")
        .join("projects")
        .join("ffffffffffffffff");
    assert!(
        !unknown_project_dir.exists(),
        "an event-controlled token must never create an arbitrary destination"
    );

    let machine = read_messages(&machine_dir);
    let project_a = read_messages(&project_a_dir);
    let project_b = read_messages(&project_b_dir);
    assert_eq!(
        project_a
            .iter()
            .filter(|message| message.as_str() == "marker-a-concurrent-first-write")
            .count(),
        concurrent_write_count,
        "the single lazy project writer must retain every concurrent first record"
    );
    for marker in [
        "marker-a-concurrent-first-write",
        "marker-a-direct",
        "marker-a-direct-overrides-b-span",
        "marker-a-background-after-b-selected",
        "marker-a-after-filter-reload",
    ] {
        assert_exact_destination(marker, "project-a", &machine, &project_a, &project_b);
    }
    for marker in ["marker-b-span", "marker-b-foreground"] {
        assert_exact_destination(marker, "project-b", &machine, &project_a, &project_b);
    }
    for marker in ["marker-machine-unscoped", "marker-machine-unknown-scope"] {
        assert_exact_destination(marker, "machine", &machine, &project_a, &project_b);
    }
}
