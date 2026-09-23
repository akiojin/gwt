#[test]
#[should_panic(expected = "project-owned events require an explicit dispatch scope")]
fn global_broadcast_refuses_project_owned_payload() {
    OutboundEvent::broadcast(BackendEvent::LaunchWizardState { wizard: None });
}

#[test]
fn global_broadcast_allows_shared_update_progress() {
    let event = OutboundEvent::broadcast(BackendEvent::UpdateProgress {
        downloaded: 1,
        total: Some(2),
        asset: None,
        version: None,
    });
    assert!(matches!(event.target, DispatchTarget::All));
}

#[test]
fn global_broadcast_allows_host_settings_payloads() {
    for event in [
        BackendEvent::SystemSettings {
            language: "ja".into(),
            codex_trust_managed_hooks: None,
            board_provider: None,
            agent_resource: None,
        },
        BackendEvent::SystemSettingsUpdated {
            language: "ja".into(),
            codex_trust_managed_hooks: None,
            board_provider: None,
            agent_resource: None,
        },
        BackendEvent::SystemSettingsError {
            message: "settings".into(),
        },
        BackendEvent::AutostartStatus {
            enabled: false,
            mechanism: "test".into(),
            install_path: None,
        },
        BackendEvent::AutostartError {
            message: "autostart".into(),
        },
    ] {
        assert!(matches!(
            OutboundEvent::broadcast(event).target,
            DispatchTarget::All
        ));
    }
}

#[test]
fn global_broadcast_host_update_notice_is_an_ownerless_exception() {
    let notice = OutboundEvent::global_update_notice("info", "Host update ready");
    assert!(matches!(notice.target, DispatchTarget::All));
    assert!(matches!(notice.event, BackendEvent::IssueMonitorToast {
        notification_transition: None,
        issue_number: None, ref level, ref message,
    } if level == "info" && message == "Host update ready"));
    assert!(
        std::panic::catch_unwind(
            || OutboundEvent::broadcast(BackendEvent::IssueMonitorToast {
                notification_transition: None,
                issue_number: None,
                level: "info".into(),
                message: "Unscoped producer".into(),
            })
        )
        .is_err(),
        "ownerless toasts still require the dedicated host updater constructor"
    );
}

#[test]
fn global_broadcast_settings_requests_remain_client_replies() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let runtime = sample_runtime(temp.path(), vec![], None);
    for events in [
        runtime.system_settings_get_events("settings-client".into()),
        runtime.system_settings_update_events(
            "settings-client".into(),
            "ja".into(),
            None,
            None,
            None,
        ),
    ] {
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0].target, DispatchTarget::Client(id) if id == "settings-client"));
    }
}

#[test]
fn global_broadcast_direct_all_construction_inventory() {
    fn collect(
        root: &Path,
        path: &Path,
        pattern: &regex::Regex,
        inventory: &mut BTreeMap<String, usize>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, pattern, inventory);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                let compact: String = source.chars().filter(|ch| !ch.is_whitespace()).collect();
                let count = pattern.find_iter(&compact).count();
                if count > 0 {
                    inventory.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                        count,
                    );
                }
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let pattern =
        regex::Regex::new(r"target:(?:[A-Za-z_][A-Za-z_0-9]*::)*DispatchTarget::All\b").unwrap();
    let mut inventory = BTreeMap::new();
    collect(&root, &root, &pattern, &mut inventory);
    // Scan every Rust source, including newly added files. The only production
    // constructions are the two audited constructors in app_runtime/mod.rs.
    // Existing test-only constructions/patterns are inventoried explicitly:
    // embedded_server: broadcast_runtime_hook_event and transport_all;
    // main: transport_all and the update_available_event assertion pattern.
    // New test fixtures also require deliberate review of this inventory.
    assert_eq!(
        inventory,
        BTreeMap::from([
            ("app_runtime/mod.rs".into(), 2),
            ("embedded_server.rs".into(), 2),
            ("main.rs".into(), 2),
        ]),
        "new direct All targets must use the audited global constructors"
    );
}
