//! SessionStart readiness policy is shared without a GUI runtime.

use gwt::project_runtime::launch_readiness::{
    continue_work_readiness_decision, ContinueWorkReadinessWatch, LaunchPaneDisposition,
    ReadinessDeadlineDecision, ReadinessPaneEvidence,
};

/// Issue #3475: the readiness deadline is progress-aware. A pane that is alive
/// and still emitting PTY output buys another extension instead of aborting a
/// prepared successor mid-bootstrap, and the silent streak resets.
#[test]
fn continue_work_readiness_deadline_extends_while_the_pane_is_alive_and_producing_output() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 0,
        silent_extensions: 1,
        observed_output_bytes: 128,
        handed_off: false,
    };

    assert_eq!(
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, 4096),
        ReadinessDeadlineDecision::Extend(ContinueWorkReadinessWatch {
            operation_id: "op".to_string(),
            extensions: 1,
            silent_extensions: 0,
            observed_output_bytes: 4096,
            handed_off: false,
        }),
    );
}

/// Issue #3482 AC-4 (slow progress): a pane that is nearly out of extension
/// budget but still emitting output keeps extending. Slowness alone is never
/// evidence that the launch is dead.
#[test]
fn continue_work_readiness_deadline_extends_a_slow_but_progressing_pane() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 3,
        silent_extensions: 2,
        observed_output_bytes: 12_288,
        handed_off: false,
    };

    assert_eq!(
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, 12_289),
        ReadinessDeadlineDecision::Extend(ContinueWorkReadinessWatch {
            operation_id: "op".to_string(),
            extensions: 4,
            silent_extensions: 0,
            observed_output_bytes: 12_289,
            handed_off: false,
        }),
    );
}

/// Issue #3475: no liveness evidence means the deadline still aborts on the
/// spot — the extension budget must never keep a dead launch pending.
/// Issue #3482 AC-3: a dead pane is exactly the case that may still be torn
/// down, because there is no live process left to destroy.
#[test]
fn continue_work_readiness_deadline_aborts_at_once_when_the_pane_process_is_gone() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 1,
        silent_extensions: 0,
        observed_output_bytes: 10,
        handed_off: false,
    };

    let ReadinessDeadlineDecision::Abort { detail, pane } =
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Dead, 4096)
    else {
        panic!("a dead pane must abort the readiness deadline");
    };
    assert_eq!(pane, LaunchPaneDisposition::Teardown);
    assert!(detail.contains("after 150s"), "{detail}");
    assert!(detail.contains("no longer running"), "{detail}");
}

/// Issue #3482 AC-3 (identity mismatch): when another runtime owns the launch
/// window, the durable candidate is still rolled back — but the pane that is
/// there now belongs to somebody else and must survive untouched.
#[test]
fn continue_work_readiness_deadline_retains_a_foreign_pane_while_rolling_the_candidate_back() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 1,
        silent_extensions: 0,
        observed_output_bytes: 10,
        handed_off: false,
    };

    let ReadinessDeadlineDecision::Abort { detail, pane } =
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Foreign, 4096)
    else {
        panic!("a foreign pane must still roll the candidate back");
    };
    assert_eq!(
        pane,
        LaunchPaneDisposition::Retain,
        "cleanup must never terminate a pane this launch does not own",
    );
    assert!(detail.contains("after 150s"), "{detail}");
    assert!(detail.contains("no longer owns"), "{detail}");
}

/// Issue #3482 AC-2 (silent live): the silent streak still bounds *waiting*,
/// but reaching the bound hands the launch to the user instead of killing a
/// process that is demonstrably alive.
#[test]
fn continue_work_readiness_deadline_hands_off_a_silent_live_pane_without_killing_it() {
    let mut watch = ContinueWorkReadinessWatch::new("op".to_string());
    let mut granted = 0_u32;
    let (handoff, detail) = loop {
        match continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, 0) {
            ReadinessDeadlineDecision::Extend(next) => {
                granted += 1;
                assert!(granted <= 8, "a silent pane must not extend forever");
                watch = next;
            }
            ReadinessDeadlineDecision::HandOff {
                watch: next,
                detail,
            } => {
                break (
                    next,
                    detail.expect("the handoff transition must be diagnosable"),
                )
            }
            ReadinessDeadlineDecision::Abort { .. } => {
                panic!("a live pane must never be aborted by the readiness deadline")
            }
        }
    };

    assert_eq!(granted, 2);
    assert!(handoff.handed_off);
    assert!(detail.contains("after 210s"), "{detail}");
    assert!(detail.contains("never produced any output"), "{detail}");
}

/// Issue #3482 AC-2: progress buys time but not an unbounded wait. Reaching the
/// absolute cap stops the waiting, not the agent — the launch is handed off
/// with the pane and its in-flight resume intact.
#[test]
fn continue_work_readiness_deadline_hands_off_a_progressing_pane_at_the_absolute_cap() {
    let mut watch = ContinueWorkReadinessWatch::new("op".to_string());
    let mut output_bytes = 0_u64;
    let mut granted = 0_u32;
    let (handoff, detail) = loop {
        output_bytes += 4096;
        match continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, output_bytes) {
            ReadinessDeadlineDecision::Extend(next) => {
                granted += 1;
                assert!(granted <= 8, "a progressing pane must not extend forever");
                watch = next;
            }
            ReadinessDeadlineDecision::HandOff {
                watch: next,
                detail,
            } => {
                break (
                    next,
                    detail.expect("the handoff transition must be diagnosable"),
                )
            }
            ReadinessDeadlineDecision::Abort { .. } => {
                panic!("a live pane must never be aborted by the readiness deadline")
            }
        }
    };

    assert_eq!(granted, 4);
    assert!(handoff.handed_off);
    assert!(detail.contains("after 330s"), "{detail}");
    assert!(detail.contains("never reported"), "{detail}");
}

/// Issue #3482: after the handoff the deadline degrades into a supervision
/// loop. It keeps re-arming so a later death is still reaped, but it must not
/// repeat the diagnostic every minute for a pane the user already owns.
#[test]
fn continue_work_readiness_supervision_stays_quiet_while_the_handed_off_pane_lives() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 4,
        silent_extensions: 0,
        observed_output_bytes: 4096,
        handed_off: true,
    };

    assert_eq!(
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, 8192),
        ReadinessDeadlineDecision::HandOff {
            watch: watch.clone(),
            detail: None,
        },
    );
}

/// Issue #3482 AC-3: handing off does not abandon the durable candidate. Once
/// the supervised pane really dies, the same bounded cleanup runs.
#[test]
fn continue_work_readiness_supervision_cleans_up_once_the_handed_off_pane_dies() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 4,
        silent_extensions: 0,
        observed_output_bytes: 4096,
        handed_off: true,
    };

    let ReadinessDeadlineDecision::Abort { detail, pane } =
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Dead, 8192)
    else {
        panic!("a supervised pane that died must roll the candidate back");
    };
    assert_eq!(pane, LaunchPaneDisposition::Teardown);
    assert!(detail.contains("no longer running"), "{detail}");
}
