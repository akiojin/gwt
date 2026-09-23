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
