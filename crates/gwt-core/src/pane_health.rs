//! Read-only pane health decisions (SPEC #3834). Activity timestamps and
//! terminal contents never become part of the public reason.

use serde::Serialize;

use crate::process_tree::observation::LiveProcess;

/// Existing unresolved directory-trust receipt, shared with the runtime.
pub const CODEX_DIRECTORY_TRUST_PROMPT_REASON: &str =
    "Codex requires directory trust confirmation for the managed worktree";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    Running,
    Idle,
    Waiting,
    Stopped,
    Interrupted,
    Error,
    Starting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    DirectoryTrust,
    Permission,
    HumanInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailSignal {
    Approval(PromptKind),
    Interrupted,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Healthy,
    SettledIdle,
    WaitingForHuman,
    Stalled,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedAction {
    None,
    CloseOrRelaunch,
    ReportToHuman,
    StopAndRequeue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Health {
    pub classification: Classification,
    pub reason_code: &'static str,
    pub recommended_action: RecommendedAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_kind: Option<PromptKind>,
    pub processes: Vec<LiveProcess>,
}

/// Reduce optional terminal evidence to a fixed enum before classification.
/// Callers without a terminal snapshot can use the runtime's typed status.
pub fn tail_signal(tail: &str) -> TailSignal {
    let tail = tail.to_ascii_lowercase();
    if tail.contains("trust the files in this folder")
        || tail.contains("trust this directory")
        || tail.contains("do you trust the contents of this directory?")
    {
        return TailSignal::Approval(PromptKind::DirectoryTrust);
    }
    if tail.contains("would you like to run the following command")
        || tail.contains("do you want to proceed?")
        || tail.contains("allow this tool")
    {
        return TailSignal::Approval(PromptKind::Permission);
    }
    if tail.contains("interrupted") || tail.contains("signal 15") {
        return TailSignal::Interrupted;
    }
    TailSignal::None
}

/// A missing census is uncertainty, never evidence that a pane has no children.
pub fn classify(state: PaneState, tail: TailSignal, processes: Option<&[LiveProcess]>) -> Health {
    use Classification::*;
    use RecommendedAction::{CloseOrRelaunch, None as NoAction, ReportToHuman, StopAndRequeue};
    let result = |classification, reason_code, recommended_action, prompt_kind, processes| Health {
        classification,
        reason_code,
        recommended_action,
        prompt_kind,
        processes,
    };
    let Some(processes) = processes else {
        return result(
            Blocked,
            "observation_unavailable",
            NoAction,
            None,
            Vec::new(),
        );
    };
    if !processes.is_empty() {
        return result(
            Healthy,
            "live_child_process",
            NoAction,
            None,
            processes.to_vec(),
        );
    }
    if state == PaneState::Error {
        return result(Blocked, "launch_failed", StopAndRequeue, None, Vec::new());
    }
    if let TailSignal::Approval(kind) = tail {
        return result(
            WaitingForHuman,
            "approval_prompt",
            ReportToHuman,
            Some(kind),
            Vec::new(),
        );
    }
    if state == PaneState::Waiting {
        return result(
            WaitingForHuman,
            "human_input_prompt",
            ReportToHuman,
            Some(PromptKind::HumanInput),
            Vec::new(),
        );
    }
    if matches!(state, PaneState::Interrupted) || tail == TailSignal::Interrupted {
        return result(Stalled, "interrupted", StopAndRequeue, None, Vec::new());
    }
    if matches!(state, PaneState::Idle | PaneState::Stopped) {
        return result(
            SettledIdle,
            "turn_finished",
            CloseOrRelaunch,
            None,
            Vec::new(),
        );
    }
    result(
        Healthy,
        "running_without_children",
        NoAction,
        None,
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_health_decision_table_and_process_precedence() {
        use Classification::*;
        use RecommendedAction::{CloseOrRelaunch, None as NoAction, ReportToHuman, StopAndRequeue};
        let child = LiveProcess {
            pid: 42,
            name: "cargo".into(),
            elapsed_secs: 1200,
        };
        for state in [
            PaneState::Running,
            PaneState::Idle,
            PaneState::Waiting,
            PaneState::Error,
            PaneState::Interrupted,
        ] {
            let health = classify(
                state,
                TailSignal::Approval(PromptKind::Permission),
                Some(std::slice::from_ref(&child)),
            );
            assert_eq!(
                (health.classification, health.recommended_action),
                (Healthy, NoAction)
            );
            assert_eq!(health.reason_code, "live_child_process");
            assert_eq!(health.processes, vec![child.clone()]);
        }
        for (state, tail, classification, code, action) in [
            (
                PaneState::Running,
                TailSignal::Approval(PromptKind::Permission),
                WaitingForHuman,
                "approval_prompt",
                ReportToHuman,
            ),
            (
                PaneState::Waiting,
                TailSignal::None,
                WaitingForHuman,
                "human_input_prompt",
                ReportToHuman,
            ),
            (
                PaneState::Idle,
                TailSignal::None,
                SettledIdle,
                "turn_finished",
                CloseOrRelaunch,
            ),
            (
                PaneState::Idle,
                TailSignal::Interrupted,
                Stalled,
                "interrupted",
                StopAndRequeue,
            ),
            (
                PaneState::Interrupted,
                TailSignal::None,
                Stalled,
                "interrupted",
                StopAndRequeue,
            ),
            (
                PaneState::Error,
                TailSignal::None,
                Blocked,
                "launch_failed",
                StopAndRequeue,
            ),
            (
                PaneState::Running,
                TailSignal::None,
                Healthy,
                "running_without_children",
                NoAction,
            ),
        ] {
            let health = classify(state, tail, Some(&[]));
            assert_eq!(
                (
                    health.classification,
                    health.reason_code,
                    health.recommended_action
                ),
                (classification, code, action)
            );
        }
        let unavailable = classify(PaneState::Idle, TailSignal::None, None);
        assert_eq!(unavailable.recommended_action, NoAction);
        assert_eq!(unavailable.reason_code, "observation_unavailable");
    }

    #[test]
    fn pane_health_approval_reason_never_contains_terminal_secrets() {
        assert_eq!(
            tail_signal("Do you trust the contents of this directory?"),
            TailSignal::Approval(PromptKind::DirectoryTrust)
        );
        let tail = "Do you trust the files in this folder? /secret/token-123\ncommand: rm /private/file --api-key=token-123";
        let signal = tail_signal(tail);
        assert_eq!(signal, TailSignal::Approval(PromptKind::DirectoryTrust));
        let health = classify(PaneState::Running, signal, Some(&[]));
        assert_eq!(health.classification, Classification::WaitingForHuman);
        let json = serde_json::to_string(&health).unwrap();
        for secret in ["/secret", "token-123", "rm ", "/private", "--api-key"] {
            assert!(!json.contains(secret), "leaked {secret}: {json}");
        }
    }
}
