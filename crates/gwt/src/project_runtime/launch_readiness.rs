//! Progress-aware SessionStart readiness policy shared by project runtime owners.
//!
//! Callers supply exact pane evidence and output progress; timers, observation,
//! and rollback remain with the runtime that owns the launch.

/// Issue #3475: base deadline for the authenticated SessionStart receipt of a
/// candidate launch, measured from the PTY handoff. The previous fixed 60s
/// aborted a healthy launch that resumed a 12.5 MB Codex conversation in
/// ~60.3s, so the base now carries real headroom before the progress-aware
/// extensions below take over.
const CONTINUE_WORK_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
/// Issue #3475: one extension granted when the deadline arrives and the pane
/// process is still alive.
const CONTINUE_WORK_READY_EXTENSION: std::time::Duration = std::time::Duration::from_secs(60);
/// Issue #3475: absolute cap on extensions (90s + 4 × 60s = 330s). Bounds the
/// wait for an agent that never reports SessionStart no matter how busy it looks.
const CONTINUE_WORK_READY_MAX_EXTENSIONS: u32 = 4;
/// Issue #3475: cap on *consecutive* extensions granted without new PTY output
/// (90s + 2 × 60s = 210s). New output resets the streak, so a pane that is
/// visibly working keeps the full budget above while a wedged one gives up sooner.
const CONTINUE_WORK_READY_MAX_SILENT_EXTENSIONS: u32 = 2;

/// Issue #3475: progress-aware state for one authenticated SessionStart
/// readiness deadline.
///
/// It rides on the correlated timer event rather than living in an
/// `AppRuntime` map, so an extension needs no extra teardown path and a late
/// timer for a superseded launch stays as harmless as it was before. `waited`
/// is derived from the granted extension count instead of a measured clock so
/// tests never depend on wall-clock absolutes (Issue #3339).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinueWorkReadinessWatch {
    /// The launch operation this deadline is correlated with.
    pub operation_id: String,
    /// Extensions already granted; `0` is the base deadline.
    pub extensions: u32,
    /// Consecutive extensions granted without new PTY output.
    pub silent_extensions: u32,
    /// Cumulative PTY output bytes observed when this deadline was armed.
    pub observed_output_bytes: u64,
    /// Issue #3482: the wait budget is spent and the launch was handed to the
    /// user with its live pane intact. The deadline keeps firing from here on,
    /// but only to reap the candidate once the pane stops being ours.
    pub handed_off: bool,
}

impl ContinueWorkReadinessWatch {
    pub fn new(operation_id: String) -> Self {
        Self {
            operation_id,
            extensions: 0,
            silent_extensions: 0,
            observed_output_bytes: 0,
            handed_off: false,
        }
    }

    /// How long the launch has already waited when this deadline fires.
    fn waited(&self) -> std::time::Duration {
        CONTINUE_WORK_READY_TIMEOUT + CONTINUE_WORK_READY_EXTENSION * self.extensions
    }

    /// How long this deadline sleeps before firing.
    pub fn delay(&self) -> std::time::Duration {
        if self.extensions == 0 {
            CONTINUE_WORK_READY_TIMEOUT
        } else {
            CONTINUE_WORK_READY_EXTENSION
        }
    }

    /// Issue #3482: freeze this watch into supervision mode. The budget fields
    /// stop moving because nothing is being waited for any more.
    fn handed_off(&self) -> Self {
        Self {
            handed_off: true,
            ..self.clone()
        }
    }
}

/// Issue #3482: what a fired readiness deadline can prove about the pane in the
/// launch's window. A window id can outlive the runtime that was installed in
/// it, so liveness alone is not enough to decide who may be torn down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessPaneEvidence {
    /// gwt still owns the window, the exact candidate Session is bound to it,
    /// and the PTY process is running.
    Live,
    /// The launch's own pane is gone: no PTY runtime, or the process watcher
    /// reported an exit or error.
    Dead,
    /// The window is gone, or another Session owns it now. Nothing in that
    /// window belongs to this launch.
    Foreign,
}

/// Issue #3482: whether rolling a failed launch back may also tear down the
/// pane and window it was launched into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPaneDisposition {
    /// The pane belongs to this launch, so it goes down with it.
    Teardown,
    /// Something else owns the window now — clean up durable state only.
    Retain,
}

/// Issue #3475: what a fired readiness deadline should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessDeadlineDecision {
    /// Keep waiting; re-arm with this updated watch.
    Extend(ContinueWorkReadinessWatch),
    /// Issue #3482: stop waiting without stopping the agent. The pane is still
    /// the exact live launch pane, so it and its prepared candidate are left
    /// alone and the deadline re-arms as a supervisor. `detail` is `Some` only
    /// on the transition, so supervision does not repeat the diagnostic.
    HandOff {
        watch: ContinueWorkReadinessWatch,
        detail: Option<String>,
    },
    /// Roll the launch back. `detail` names the total wait and what never
    /// happened; `pane` says whether cleanup owns the window it fired for.
    Abort {
        detail: String,
        pane: LaunchPaneDisposition,
    },
}

/// Issue #3475 / #3482: decide a fired readiness deadline from evidence instead
/// of a fixed expiry. A live pane buys an extension, and new PTY output resets
/// the silent streak so a busy bootstrap keeps the larger budget.
///
/// Both budgets stay capped, but Issue #3482 draws the line the cap was
/// crossing: the deadline's authority is over *waiting*, not over a live
/// Session. Spending the budget on a pane that is provably still this launch's
/// own live pane hands the launch to the user instead of destroying an
/// in-flight resume. Only a dead pane — or a window some other Session owns
/// now — is rolled back, and the foreign case is rolled back without touching
/// whatever is running there.
///
/// Pure so the policy is unit-testable without a PTY or a clock.
pub fn continue_work_readiness_decision(
    watch: &ContinueWorkReadinessWatch,
    evidence: ReadinessPaneEvidence,
    output_bytes: u64,
) -> ReadinessDeadlineDecision {
    let waited = watch.waited().as_secs();
    match evidence {
        ReadinessPaneEvidence::Foreign => {
            return ReadinessDeadlineDecision::Abort {
                detail: readiness_timeout_detail(
                    waited,
                    "this launch no longer owns the pane in its window",
                ),
                pane: LaunchPaneDisposition::Retain,
            };
        }
        ReadinessPaneEvidence::Dead => {
            return ReadinessDeadlineDecision::Abort {
                detail: readiness_timeout_detail(waited, "the agent process is no longer running"),
                pane: LaunchPaneDisposition::Teardown,
            };
        }
        ReadinessPaneEvidence::Live => {}
    }
    if watch.handed_off {
        return ReadinessDeadlineDecision::HandOff {
            watch: watch.clone(),
            detail: None,
        };
    }
    if watch.extensions >= CONTINUE_WORK_READY_MAX_EXTENSIONS {
        return ReadinessDeadlineDecision::HandOff {
            watch: watch.handed_off(),
            detail: Some(readiness_handoff_detail(
                waited,
                "the agent process kept running but never reported an authenticated SessionStart",
            )),
        };
    }
    let silent_extensions = if output_bytes > watch.observed_output_bytes {
        0
    } else {
        watch.silent_extensions + 1
    };
    if silent_extensions > CONTINUE_WORK_READY_MAX_SILENT_EXTENSIONS {
        return ReadinessDeadlineDecision::HandOff {
            watch: watch.handed_off(),
            detail: Some(readiness_handoff_detail(
                waited,
                if output_bytes > 0 {
                    "the agent process is still running but stopped producing output before reporting an authenticated SessionStart"
                } else {
                    "the agent process is still running but never produced any output"
                },
            )),
        };
    }
    ReadinessDeadlineDecision::Extend(ContinueWorkReadinessWatch {
        operation_id: watch.operation_id.clone(),
        extensions: watch.extensions + 1,
        silent_extensions,
        observed_output_bytes: output_bytes,
        handed_off: false,
    })
}

fn readiness_timeout_detail(waited_secs: u64, observation: &str) -> String {
    format!("authenticated SessionStart readiness timed out after {waited_secs}s: {observation}")
}

/// Issue #3482: the user-facing half of a handoff. It has to say how long gwt
/// waited, what never happened, and that the pane was deliberately left alive —
/// otherwise a pane that simply stops progressing looks like a hang with no
/// explanation.
fn readiness_handoff_detail(waited_secs: u64, observation: &str) -> String {
    format!(
        "still waiting for an authenticated SessionStart after {waited_secs}s: {observation}. \
         gwt stopped waiting but left this agent running — keep waiting for it, or stop the \
         window to cancel the launch."
    )
}
