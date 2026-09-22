#[test]
fn project_wizards_keep_independent_open_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let root_a = temp.path().join("a");
    let root_b = temp.path().join("b");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    let mut runtime = sample_runtime(
        temp.path(),
        vec![
            sample_project_tab("tab-a", "A", root_a.clone(), ProjectKind::Git, &[]),
            sample_project_tab("tab-b", "B", root_b.clone(), ProjectKind::Git, &[]),
        ],
        Some("tab-a"),
    );
    runtime
        .open_launch_wizard_for_branch("tab-a", &root_a, "feature/a", None, None)
        .unwrap();
    let context_a = runtime.project_context("tab-a").unwrap();
    let first = runtime.launch_wizard_state_outbound(&context_a);
    let wizard_a_id = runtime
        .launch_wizard_for(&context_a)
        .unwrap()
        .wizard_id
        .clone();
    runtime
        .open_launch_wizard_for_branch("tab-b", &root_b, "feature/b", None, None)
        .unwrap();
    let context_b = runtime.project_context("tab-b").unwrap();
    let wizard_b_id = runtime
        .launch_wizard_for(&context_b)
        .unwrap()
        .wizard_id
        .clone();
    let restored = runtime.launch_wizard_state_outbound(&context_a);
    match (first.event, restored.event) {
        (
            BackendEvent::LaunchWizardState {
                wizard: Some(first),
            },
            BackendEvent::LaunchWizardState {
                wizard: Some(restored),
            },
        ) => {
            assert_eq!(first.branch_name, restored.branch_name);
        }
        _ => panic!("project A must retain its own wizard while B is opened"),
    }
    runtime.handle_launch_wizard_action_for_client(
        &context_a,
        None,
        gwt::LaunchWizardAction::Cancel,
        None,
    );
    assert!(runtime.launch_wizard_for(&context_a).is_none());
    assert_eq!(
        runtime.launch_wizard_for(&context_b).unwrap().wizard_id,
        wizard_b_id
    );
    assert!(runtime
        .handle_launch_wizard_runtime_resolved(wizard_a_id, Err("late A response".into()))
        .is_empty());
    assert!(runtime
        .launch_wizard_for(&context_b)
        .unwrap()
        .wizard
        .hydration_error
        .is_none());
}
