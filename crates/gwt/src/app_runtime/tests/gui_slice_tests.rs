use super::*;

#[test]
fn startup_pm_ensure_slice_defers_the_second_tab_when_the_budget_is_exhausted() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    runtime.pending_startup_pm_tabs = vec!["missing-a".to_string(), "missing-b".to_string()];

    let mut budget = super::super::GuiSliceBudget::exhausted();
    let events = runtime.startup_pm_ensure_slice(&mut budget);

    assert!(events.is_empty());
    assert_eq!(
        runtime.pending_startup_pm_tabs,
        vec!["missing-b".to_string()]
    );
    let AppEventProxy::Stub(recorded) = &runtime.proxy else {
        panic!("sample runtime records events on a stub proxy");
    };
    let recorded = recorded.lock().expect("recorded events");
    assert!(
        recorded
            .iter()
            .any(|event| matches!(event, UserEvent::GuiSliceContinue)),
        "the remainder must continue on a later event-loop turn"
    );
}
