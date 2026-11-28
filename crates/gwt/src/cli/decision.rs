//! `decision.*` — the gwtd decision queue (Issue #7742).
//!
//! Agents register decisions here and nowhere else. The queue belongs to the
//! project, so it survives a stopped or replaced PM; answers are delivered by
//! gwtd to the waiting sessions, never relayed through the PM.

use gwt_core::decision_queue::{self as queue, AnsweredBy, Registration};
use gwt_github::{client::ApiError, SpecOpsError};

use super::CliEnv;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionCommand {
    /// The caller's session identity is taken from the launch environment,
    /// never from the envelope.
    Register(Box<Registration>),
    List {
        issue: Option<u64>,
        include_closed: bool,
    },
    Attach {
        id: String,
    },
    Merge {
        into: String,
        ids: Vec<String>,
    },
    Answer {
        id: String,
        answer: String,
    },
    Defer {
        id: String,
    },
}

fn failed(error: gwt_core::GwtError) -> SpecOpsError {
    match error {
        gwt_core::GwtError::Config(message) => SpecOpsError::Operation(message),
        other => SpecOpsError::Operation(other.to_string()),
    }
}

/// Closing, bundling and reordering are the PM's first pass (or the user's
/// own direct call). An agent waiting on a decision cannot settle it itself.
fn settle_authority<E: CliEnv>(env: &E, operation: &str) -> Result<AnsweredBy, SpecOpsError> {
    // Without a session the caller is unattributed: an agent shell can unset
    // GWT_SESSION_ID, so this is recorded as a CLI answer, never as the user.
    if super::pm::ambient_session_id().is_none() {
        return Ok(AnsweredBy::Cli);
    }
    if super::pm::caller_is_registered_pm(env.repo_path()) {
        return Ok(AnsweredBy::Pm);
    }
    Err(SpecOpsError::from(ApiError::PermissionDenied {
        message: format!("{operation} requires the registered PM or a direct user CLI call"),
    }))
}

fn render(out: &mut String, value: serde_json::Value) {
    out.push_str(&serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()));
    out.push('\n');
}

pub fn run<E: CliEnv>(
    env: &mut E,
    command: DecisionCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let repo = env.repo_path().to_path_buf();
    match command {
        DecisionCommand::Register(mut registration) => {
            registration.session_id = super::pm::ambient_session_id();
            let outcome = queue::register(&repo, &registration).map_err(failed)?;
            render(
                out,
                serde_json::json!({
                    "created": outcome.created,
                    "duplicate_of_existing": !outcome.created,
                    "decision": outcome.item,
                }),
            );
        }
        DecisionCommand::List {
            issue,
            include_closed,
        } => {
            let decisions = queue::list(&repo, issue, include_closed).map_err(failed)?;
            let waiting = queue::snapshot(&repo).map_err(failed)?.waiting_count;
            render(
                out,
                serde_json::json!({"waiting_count": waiting, "decisions": decisions}),
            );
        }
        DecisionCommand::Attach { id } => {
            let session = super::pm::ambient_session_id().unwrap_or_default();
            let decision = queue::attach(&repo, &id, &session).map_err(failed)?;
            render(out, serde_json::json!({"decision": decision}));
        }
        DecisionCommand::Merge { into, ids } => {
            settle_authority(env, "decision.merge")?;
            let decision = queue::merge(&repo, &into, &ids).map_err(failed)?;
            render(out, serde_json::json!({"decision": decision}));
        }
        DecisionCommand::Answer { id, answer } => {
            let by = settle_authority(env, "decision.answer")?;
            let decision = queue::answer(&repo, &id, &answer, by).map_err(failed)?;
            render(
                out,
                serde_json::json!({
                    "decision": decision,
                    "delivery": "gwtd delivers the answer to each waiting session",
                }),
            );
        }
        DecisionCommand::Defer { id } => {
            settle_authority(env, "decision.defer")?;
            let decision = queue::defer(&repo, &id).map_err(failed)?;
            let waiting = queue::snapshot(&repo).map_err(failed)?.waiting_count;
            render(
                out,
                serde_json::json!({"waiting_count": waiting, "decision": decision}),
            );
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use crate::cli::env::TestEnv;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
    use serde_json::{json, Value};
    use std::path::Path;

    fn env_lock() -> gwt_core::test_support::EnvLockGuard {
        crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn invoke(repo: &Path, operation: &str, params: Value) -> Result<Value, String> {
        let command = crate::cli::json_envelope::parse_operation(operation, &params)
            .map_err(|error| format!("{error:?}"))?;
        let (code, output) =
            crate::cli::run_collect(&mut TestEnv::new(repo.to_path_buf()), command)
                .map_err(|error| error.to_string())?;
        assert_eq!(code, 0, "{output}");
        Ok(serde_json::from_str(&output).unwrap())
    }

    fn product(target: &str) -> Value {
        json!({
            "issue": 7742, "kind": "product", "target": target,
            "question": "Which storage should the queue use?",
            "materials": "Both options pass the existing tests.",
            "options": ["Keep JSON", "Move to SQLite"],
            "recommendation": "Keep JSON",
        })
    }

    fn register_pm(repo: &Path, session_id: &str) {
        use crate::pm_registry::{self, PmPrefs, PmRegistration};
        pm_registry::save_pm_prefs(
            &pm_registry::pm_prefs_path_for_repo_path(repo),
            &PmPrefs {
                registration: Some(PmRegistration {
                    session_id: session_id.into(),
                    agent_id: "codex".into(),
                    worktree_path: repo.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..PmPrefs::default()
            },
        )
        .unwrap();
    }

    /// AC-1: the typed operation registers a decision, refuses one that lacks
    /// a required field, and returns the existing decision for a duplicate.
    #[test]
    fn register_validates_required_fields_and_refuses_a_duplicate() {
        let _lock = env_lock();
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _agent = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "agent-a");

        let error = invoke(&repo, "decision.register", json!({"issue": 7742})).unwrap_err();
        for field in ["kind", "target", "question", "materials"] {
            assert!(error.contains(field), "{field} missing from: {error}");
        }
        let mut no_choices = product("storage");
        no_choices.as_object_mut().unwrap().remove("options");
        let error = invoke(&repo, "decision.register", no_choices).unwrap_err();
        assert!(error.contains("options"), "{error}");
        assert_eq!(
            invoke(&repo, "decision.list", json!({})).unwrap()["decisions"],
            json!([])
        );

        let first = invoke(&repo, "decision.register", product("Queue storage")).unwrap();
        assert_eq!(first["created"], true);
        assert_eq!(
            first["decision"]["waiters"],
            json!([{"role": "session", "session_id": "agent-a"}])
        );
        // The session identity is never taken from the envelope.
        let mut forged = product("Queue storage");
        forged["session_id"] = json!("someone-else");
        assert!(invoke(&repo, "decision.register", forged).is_err());

        let _other = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "agent-b");
        let second = invoke(&repo, "decision.register", product(" queue  STORAGE ")).unwrap();
        assert_eq!(second["created"], false);
        assert_eq!(second["duplicate_of_existing"], true);
        assert_eq!(second["decision"]["id"], first["decision"]["id"]);
        assert_eq!(second["decision"]["waiters"].as_array().unwrap().len(), 2);

        let listed = invoke(&repo, "decision.list", json!({"issue": 7742})).unwrap();
        assert_eq!(listed["waiting_count"], 1);
        assert_eq!(listed["decisions"].as_array().unwrap().len(), 1);
        assert_eq!(
            invoke(&repo, "decision.list", json!({"issue": 1})).unwrap()["decisions"],
            json!([])
        );
    }

    /// Item 158: the PM closes what it can answer and bundles duplicates; a
    /// waiting agent can attach but cannot settle a decision itself.
    #[test]
    fn only_the_pm_or_the_user_settles_and_bundles_decisions() {
        let _lock = env_lock();
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        register_pm(&repo, "pm-a");
        let agent = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "agent-a");
        let kept = invoke(&repo, "decision.register", product("storage")).unwrap();
        let bundled = invoke(&repo, "decision.register", product("persistence")).unwrap();
        let (kept, bundled) = (
            kept["decision"]["id"].as_str().unwrap().to_owned(),
            bundled["decision"]["id"].as_str().unwrap().to_owned(),
        );
        for (operation, params) in [
            (
                "decision.answer",
                json!({"id": kept, "answer": "Keep JSON"}),
            ),
            ("decision.merge", json!({"into": kept, "ids": [bundled]})),
            ("decision.defer", json!({"id": kept})),
        ] {
            let error = invoke(&repo, operation, params).unwrap_err();
            assert!(error.contains("requires the registered PM"), "{error}");
        }
        drop(agent);

        let pm = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-a");
        let deferred = invoke(&repo, "decision.defer", json!({"id": bundled})).unwrap();
        assert_eq!(deferred["waiting_count"], 2, "deferring is not a decision");
        let merged = invoke(
            &repo,
            "decision.merge",
            json!({"into": kept, "ids": [bundled]}),
        )
        .unwrap();
        assert_eq!(merged["decision"]["id"], kept.as_str());
        let answered = invoke(
            &repo,
            "decision.answer",
            json!({"id": kept, "answer": "Keep JSON"}),
        )
        .unwrap();
        assert_eq!(answered["decision"]["status"], "answered");
        assert_eq!(answered["decision"]["answer"]["by"], "pm");
        drop(pm);

        // A session that asks later reads the recorded answer.
        let _late = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "agent-c");
        let attached = invoke(&repo, "decision.attach", json!({"id": bundled})).unwrap();
        assert_eq!(attached["decision"]["answer"]["text"], "Keep JSON");
        let all = invoke(
            &repo,
            "decision.list",
            json!({"issue": 7742, "include_closed": true}),
        )
        .unwrap();
        assert_eq!(all["waiting_count"], 0);
        assert_eq!(all["decisions"].as_array().unwrap().len(), 2);
        assert!(invoke(&repo, "decision.attach", json!({})).is_err());
        assert!(invoke(
            &repo,
            "decision.answer",
            json!({"id": "missing", "answer": "x"})
        )
        .is_err());
    }

    /// A call without an agent session may settle a decision, but it is
    /// recorded as an unattributed CLI answer, never as the user (Issue #8083).
    #[test]
    fn a_direct_user_call_answers_as_the_user() {
        let _lock = env_lock();
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let agent = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "agent-a");
        let id = invoke(&repo, "decision.register", product("storage")).unwrap()["decision"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        drop(agent);
        let _user = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        // Without a session there is nobody to resume, so nothing is registered.
        assert!(invoke(&repo, "decision.register", product("other")).is_err());
        let answered = invoke(
            &repo,
            "decision.answer",
            json!({"id": id, "answer": "Keep JSON"}),
        )
        .unwrap();
        assert_eq!(answered["decision"]["answer"]["by"], "cli");
    }
}
