//! Issue #4433: a client that reconnects while a branch cleanup is still
//! running must be able to pull the in-flight operation's current state.
//! Progress and results used to be replied to the originating `client_id`
//! only, so the reconnected client (new `client_id`) saw nothing and the UI
//! reported a failure that never happened.

use gwt::{
    BranchCleanupOperationSnapshot, BranchCleanupOperationStore, BranchCleanupProgressEntry,
    BranchCleanupProgressPhase, BranchCleanupResultEntry, BranchCleanupResultStatus,
};

const WINDOW_ID: &str = "branches-1";
const OPERATION_ID: &str = "branches-1-1758000000000-1";

fn progress(index: usize) -> BranchCleanupProgressEntry {
    BranchCleanupProgressEntry {
        branch: "work/old".to_string(),
        execution_branch: Some("work/old".to_string()),
        index,
        total: 2,
        phase: BranchCleanupProgressPhase::Running,
        message: format!("Removing work/old ({index}/2)"),
    }
}

fn results() -> Vec<BranchCleanupResultEntry> {
    vec![BranchCleanupResultEntry {
        branch: "work/old".to_string(),
        execution_branch: Some("work/old".to_string()),
        status: BranchCleanupResultStatus::Success,
        message: "Deleted".to_string(),
    }]
}

#[test]
fn reconnecting_client_recovers_in_flight_progress_then_result() {
    let store = BranchCleanupOperationStore::new();

    store.record_progress(WINDOW_ID, Some(OPERATION_ID), &progress(1));

    // The reconnected client has a fresh client_id, so it can only ask by
    // (window id, operation id). It must still see the running operation.
    assert_eq!(
        store.snapshot(WINDOW_ID, OPERATION_ID),
        Some(BranchCleanupOperationSnapshot::Progress(progress(1)))
    );

    store.record_result(WINDOW_ID, Some(OPERATION_ID), &results());

    assert_eq!(
        store.snapshot(WINDOW_ID, OPERATION_ID),
        Some(BranchCleanupOperationSnapshot::Result(results())),
        "the final result must replace progress so a late reconnect is not stuck at running"
    );
}

#[test]
fn cleared_operation_stops_answering_and_never_leaks_into_the_next_cleanup() {
    let store = BranchCleanupOperationStore::new();
    store.record_result(WINDOW_ID, Some(OPERATION_ID), &results());

    store.clear(WINDOW_ID, OPERATION_ID);
    assert_eq!(
        store.snapshot(WINDOW_ID, OPERATION_ID),
        None,
        "a cleared operation must not synthesize a stale cleanup result"
    );

    // A second cleanup in the same window carries a new operation id; the
    // previous run's state must never be served under it.
    store.record_result(WINDOW_ID, Some(OPERATION_ID), &results());
    assert_eq!(
        store.snapshot(WINDOW_ID, "branches-1-1758000009999-2"),
        None
    );
}

#[test]
fn a_newer_operation_evicts_the_previous_one_for_the_same_window() {
    let store = BranchCleanupOperationStore::new();
    store.record_result(WINDOW_ID, Some(OPERATION_ID), &results());

    let next_operation_id = "branches-1-1758000009999-2";
    store.record_progress(WINDOW_ID, Some(next_operation_id), &progress(1));

    assert_eq!(store.snapshot(WINDOW_ID, OPERATION_ID), None);
    assert_eq!(
        store.snapshot(WINDOW_ID, next_operation_id),
        Some(BranchCleanupOperationSnapshot::Progress(progress(1)))
    );
}

#[test]
fn clear_from_a_different_operation_does_not_drop_the_running_one() {
    let store = BranchCleanupOperationStore::new();
    store.record_progress(WINDOW_ID, Some(OPERATION_ID), &progress(1));

    // A late close of the previous modal must not wipe the operation that is
    // currently running in the same window.
    store.clear(WINDOW_ID, "branches-1-1757999999999-0");

    assert_eq!(
        store.snapshot(WINDOW_ID, OPERATION_ID),
        Some(BranchCleanupOperationSnapshot::Progress(progress(1)))
    );
}

#[test]
fn operations_without_an_operation_id_are_not_tracked() {
    let store = BranchCleanupOperationStore::new();

    store.record_progress(WINDOW_ID, None, &progress(1));
    store.record_result(WINDOW_ID, None, &results());

    assert_eq!(store.snapshot(WINDOW_ID, OPERATION_ID), None);
}

#[test]
fn separate_windows_keep_independent_operations() {
    let store = BranchCleanupOperationStore::new();
    store.record_progress(WINDOW_ID, Some(OPERATION_ID), &progress(1));
    store.record_result("__workspace_cleanup__", Some("ws-op-1"), &results());

    assert_eq!(
        store.snapshot(WINDOW_ID, OPERATION_ID),
        Some(BranchCleanupOperationSnapshot::Progress(progress(1)))
    );
    assert_eq!(
        store.snapshot("__workspace_cleanup__", "ws-op-1"),
        Some(BranchCleanupOperationSnapshot::Result(results()))
    );
}

#[test]
fn a_reloaded_client_can_enumerate_live_operations_it_never_started() {
    // A WebView reload wipes the page's JS state, so the client cannot name
    // the operation to re-sync. The backend hands it every live cleanup on
    // the initial per-client sync instead.
    let store = BranchCleanupOperationStore::new();
    store.record_progress(WINDOW_ID, Some(OPERATION_ID), &progress(1));
    store.record_result("__workspace_cleanup__", Some("ws-op-1"), &results());

    let mut live = store.live_operations();
    live.sort_by(|left, right| left.0.cmp(&right.0));

    assert_eq!(
        live,
        vec![
            (
                "__workspace_cleanup__".to_string(),
                "ws-op-1".to_string(),
                BranchCleanupOperationSnapshot::Result(results()),
            ),
            (
                WINDOW_ID.to_string(),
                OPERATION_ID.to_string(),
                BranchCleanupOperationSnapshot::Progress(progress(1)),
            ),
        ]
    );

    store.clear(WINDOW_ID, OPERATION_ID);
    store.clear("__workspace_cleanup__", "ws-op-1");
    assert!(store.live_operations().is_empty());
}

#[test]
fn later_progress_replaces_earlier_progress() {
    let store = BranchCleanupOperationStore::new();
    store.record_progress(WINDOW_ID, Some(OPERATION_ID), &progress(1));
    store.record_progress(WINDOW_ID, Some(OPERATION_ID), &progress(2));

    assert_eq!(
        store.snapshot(WINDOW_ID, OPERATION_ID),
        Some(BranchCleanupOperationSnapshot::Progress(progress(2)))
    );
}
