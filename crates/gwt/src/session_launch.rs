//! Session launch preparation and persistence without a GUI dependency.

use std::path::Path;

pub fn initialize_launch_session(
    worktree: &Path,
    project_root: &Path,
    config: &gwt_agent::LaunchConfig,
) -> gwt_agent::Session {
    let branch_name = config.branch.clone().unwrap_or_else(|| "work".to_string());
    let mut session = gwt_agent::Session::new(worktree, branch_name, config.agent_id.clone());
    session.project_state_root = Some(gwt_core::paths::normalize_windows_child_process_path(
        project_root,
    ));
    session.display_name = config.display_name.clone();
    session.tool_version = config.tool_version.clone();
    session.model = config.model.clone();
    session.reasoning_level = config.reasoning_level.clone();
    session.session_mode = config.session_mode;
    session.skip_permissions = config.skip_permissions;
    session.fast_mode = config.fast_mode;
    session.codex_fast_mode = config.codex_fast_mode;
    session.runtime_target = config.runtime_target;
    session.docker_service = config.docker_service.clone();
    session.docker_lifecycle_intent = config.docker_lifecycle_intent;
    session.linked_issue_number = config.linked_issue_number;
    session.launch_route = config.launch_route;
    session.launch_command = config.command.clone();
    session.launch_args = config.args.clone();
    session.codex_auth_root = config.validated_codex_auth_root_for_cwd(worktree);
    session.windows_shell = config.windows_shell;
    apply_resume_identity_to_session(&mut session, config);
    session.update_status(gwt_agent::AgentStatus::Running);
    session
}

pub fn apply_resume_identity_to_session(
    session: &mut gwt_agent::Session,
    config: &gwt_agent::LaunchConfig,
) {
    if config.session_mode == gwt_agent::SessionMode::Resume {
        session.agent_session_id = config.resume_session_id.clone();
    }
}

#[cfg(test)]
type FinalizedSessionPostSaveHook = Box<dyn FnOnce() -> std::io::Result<()> + 'static>;

#[cfg(test)]
thread_local! {
    static FINALIZED_SESSION_POST_SAVE_HOOK:
        std::cell::RefCell<Option<FinalizedSessionPostSaveHook>> =
            std::cell::RefCell::new(None);
}

#[cfg(test)]
fn set_finalized_session_post_save_hook_for_test(hook: FinalizedSessionPostSaveHook) {
    FINALIZED_SESSION_POST_SAVE_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(test)]
fn invoke_finalized_session_post_save_hook() -> std::io::Result<()> {
    FINALIZED_SESSION_POST_SAVE_HOOK
        .with(|slot| slot.borrow_mut().take().map_or(Ok(()), |hook| hook()))
}

#[cfg(not(test))]
fn invoke_finalized_session_post_save_hook() -> std::io::Result<()> {
    Ok(())
}

fn interrupt_exact_running_session_after_persistence_error(
    sessions_dir: &Path,
    session: &mut gwt_agent::Session,
    running_snapshot: &gwt_agent::Session,
    context: &str,
    error: impl std::fmt::Display,
) -> String {
    session.update_status(gwt_agent::AgentStatus::Interrupted);
    match session.save_if_unchanged(sessions_dir, running_snapshot) {
        Ok(true) => format!("{context}; Session was marked Interrupted: {error}"),
        Ok(false) => {
            format!("{context} and no exact Running Session remained: {error}")
        }
        Err(interruption_error) => {
            format!("{context} and Session interruption failed: {error}; {interruption_error}")
        }
    }
}

pub fn persist_finalized_launch_session(
    sessions_dir: &Path,
    runtime_path: &Path,
    session: &mut gwt_agent::Session,
    docker_runtime_worktree: Option<&str>,
) -> Result<(), String> {
    if let Some(runtime_worktree) = docker_runtime_worktree {
        let project_state_root = session
            .project_state_root
            .as_deref()
            .filter(|root| !root.as_os_str().is_empty())
            .ok_or_else(|| {
                "Docker launch is missing the host Project State root before Session persistence"
                    .to_string()
            })?
            .to_path_buf();
        session.bind_docker_runtime(runtime_worktree, &project_state_root)?;
    }

    let saved_identity = gwt_agent::SessionExecutionIdentity::from_session(session)?;
    let running_snapshot = session.clone();
    let save_result = if let Some(expected) = saved_identity.as_ref() {
        session.save_if_execution_identity_matches(sessions_dir, expected)
    } else {
        session.save_if_absent_or_unchanged(sessions_dir, session)
    };
    match save_result {
        Ok(true) => {
            if let Err(error) = invoke_finalized_session_post_save_hook() {
                return Err(interrupt_exact_running_session_after_persistence_error(
                    sessions_dir,
                    session,
                    &running_snapshot,
                    "failed to durably confirm launch Session",
                    error,
                ));
            }
        }
        Ok(false) if saved_identity.is_some() => {
            return Err(
                "bound Session identity changed before final launch persistence".to_string(),
            )
        }
        Ok(false) => {
            return Err("unbound Session changed before final launch persistence".to_string())
        }
        Err(error) => {
            // Atomic replace followed by parent-directory fsync has an
            // unknown outcome: the intended Running TOML may already be
            // visible even though persistence returned Err. Transition only
            // that exact intended snapshot to Interrupted; a concurrent
            // same-id replacement remains byte-identical.
            return Err(interrupt_exact_running_session_after_persistence_error(
                sessions_dir,
                session,
                &running_snapshot,
                "failed to durably persist launch Session",
                error,
            ));
        }
    }
    if let Err(error) =
        gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running).save(runtime_path)
    {
        let running_snapshot = session.clone();
        session.update_status(gwt_agent::AgentStatus::Interrupted);
        let rollback = if let Some(expected) = saved_identity.as_ref() {
            session.save_if_execution_identity_matches(sessions_dir, expected)
        } else {
            session.save_if_unchanged(sessions_dir, &running_snapshot)
        };
        return Err(match rollback {
            Ok(true) => format!(
                "failed to persist launch runtime state; Session was marked Interrupted: {error}"
            ),
            Ok(false) => format!(
                "failed to persist launch runtime state and Session changed before interruption: {error}"
            ),
            Err(rollback_error) => format!(
                "failed to persist launch runtime state and Session interruption failed: {error}; {rollback_error}"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_session_initializes_and_persists_without_gui() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode).build();
        config.tool_version = Some("2.1.156".into());
        config.session_mode = gwt_agent::SessionMode::Resume;
        config.resume_session_id = Some("conversation".to_string());
        config.branch = Some("work/example".to_string());
        let project = tmp.path().join("project");
        let worktree = tmp.path().join("worktree");
        let mut session = initialize_launch_session(&worktree, &project, &config);
        assert_eq!(session.project_state_root.as_ref(), Some(&project));
        assert_eq!(session.exact_resume_session_id(), Some("conversation"));
        assert_eq!(session.launch_command, "claude");
        let sessions = tmp.path().join("sessions");
        let runtime = gwt_agent::runtime_state_path(&sessions, &session.id);
        persist_finalized_launch_session(&sessions, &runtime, &mut session, None)
            .expect("persist launch");
        let saved = gwt_agent::Session::load(&sessions.join(format!("{}.toml", session.id)))
            .expect("load session");
        assert_eq!(saved.status, gwt_agent::AgentStatus::Running);
        assert_eq!(saved.exact_resume_session_id(), Some("conversation"));
        assert_eq!(saved.tool_version.as_deref(), Some("2.1.156"));
        assert!(saved.tool_version_selector.is_none());
        assert!(saved.tool_runtime_provenance.is_none());
        assert!(runtime.exists());
    }
    #[test]
    fn finalized_bound_session_persistence_retains_same_id_replacement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "prepared-candidate");
        let mut candidate = gwt_agent::Session::new(
            temp.path().join("worktree"),
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = "prepared-candidate".to_string();
        candidate.project_state_root = Some(temp.path().join("project"));
        candidate.repo_hash = Some("repo-hash".to_string());
        candidate.linked_issue_number = Some(2359);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: candidate.id.clone(),
            repo_hash: "repo-hash".to_string(),
            owner_kind: "spec".to_string(),
            owner_number: 2359,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation".to_string(),
                binding_id: "binding".to_string(),
                ledger_head_hash: "ledger-head".to_string(),
            },
            capability_generation: 1,
        };
        candidate
            .set_execution_binding(Some(binding))
            .expect("bind candidate");
        candidate.save(&sessions_dir).expect("save exact candidate");

        let session_path = sessions_dir.join("prepared-candidate.toml");
        let mut replacement = candidate.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
        replacement
            .save(&sessions_dir)
            .expect("save same-id replacement");
        let replacement_before =
            std::fs::read(&session_path).expect("read replacement Session bytes");

        let error =
            persist_finalized_launch_session(&sessions_dir, &runtime_path, &mut candidate, None)
                .expect_err("final persistence must reject a same-id replacement");

        assert!(
            error.contains("changed"),
            "replacement conflict should be actionable: {error}"
        );
        assert_eq!(
            std::fs::read(&session_path).expect("read retained replacement"),
            replacement_before,
            "final persistence must retain the replacement byte-identically"
        );
        assert!(
            !runtime_path.exists(),
            "runtime state must not publish after Session CAS rejection"
        );
    }

    #[test]
    fn finalized_unbound_session_persistence_retains_same_id_replacement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        let mut candidate = gwt_agent::Session::new(
            temp.path().join("worktree"),
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = "unbound-candidate".to_string();
        candidate.project_state_root = Some(temp.path().join("project"));
        let mut replacement = candidate.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
        replacement
            .save(&sessions_dir)
            .expect("save same-id replacement");
        let session_path = sessions_dir.join("unbound-candidate.toml");
        let replacement_before =
            std::fs::read(&session_path).expect("read replacement Session bytes");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &candidate.id);

        let error =
            persist_finalized_launch_session(&sessions_dir, &runtime_path, &mut candidate, None)
                .expect_err("final persistence must reject an unbound same-id replacement");

        assert!(error.contains("changed"), "{error}");
        assert_eq!(
            std::fs::read(&session_path).expect("read retained replacement"),
            replacement_before
        );
        assert!(!runtime_path.exists());
    }

    #[test]
    fn finalized_session_sidecar_failure_marks_durable_session_interrupted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("create Sessions directory");
        let runtime_root = sessions_dir.join("runtime");
        std::fs::write(&runtime_root, b"not-a-directory").expect("block runtime directory");
        let mut candidate = gwt_agent::Session::new(
            temp.path().join("worktree"),
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = "sidecar-failure-candidate".to_string();
        candidate.project_state_root = Some(temp.path().join("project"));
        candidate.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = runtime_root.join(format!("{}.json", candidate.id));

        let error =
            persist_finalized_launch_session(&sessions_dir, &runtime_path, &mut candidate, None)
                .expect_err("blocked runtime root must fail final persistence");

        assert!(error.contains("marked Interrupted"), "{error}");
        assert_eq!(
            gwt_agent::Session::load(&sessions_dir.join("sidecar-failure-candidate.toml"))
                .expect("load interrupted Session")
                .status,
            gwt_agent::AgentStatus::Interrupted
        );
    }

    #[test]
    fn finalized_session_unknown_save_outcome_marks_visible_running_session_interrupted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        let mut candidate = gwt_agent::Session::new(
            temp.path().join("worktree"),
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = "unknown-save-outcome-candidate".to_string();
        candidate.project_state_root = Some(temp.path().join("project"));
        candidate.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &candidate.id);
        set_finalized_session_post_save_hook_for_test(Box::new(|| {
            Err(std::io::Error::other(
                "simulated parent-directory fsync failure after rename",
            ))
        }));

        let error =
            persist_finalized_launch_session(&sessions_dir, &runtime_path, &mut candidate, None)
                .expect_err("unknown Session save outcome must fail launch persistence");

        assert!(error.contains("marked Interrupted"), "{error}");
        assert_eq!(
            gwt_agent::Session::load(&sessions_dir.join("unknown-save-outcome-candidate.toml"))
                .expect("load recovered Session")
                .status,
            gwt_agent::AgentStatus::Interrupted,
            "a launch that was never returned to the spawner must not remain Running"
        );
        assert!(
            !runtime_path.exists(),
            "runtime state must not publish after an unknown Session save outcome"
        );
    }
}
