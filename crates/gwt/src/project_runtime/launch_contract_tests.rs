//! Existing owner launch authority contracts moved from the GUI launch module.

use crate::agent_capability::AgentCapabilityIssuer;
use crate::docker_launch::{finalize_docker_agent_launch_config_with_binding, DockerLaunchBinding};
use crate::launch_wizard::LaunchWizardMemoryCache;
use crate::project_runtime::launch_authority::install_agent_capability_env;
use crate::project_runtime::launch_finalization::{
    persist_genesis_session_binding_with, terminalize_materialized_genesis_launch,
    FinalizedAgentCapabilityLaunch, MaterializedGenesisLaunch,
};
use crate::project_runtime::launch_payload::ProcessLaunch;
use crate::session_launch::recovery::{
    clear_durable_launch_recovery, persist_durable_launch_recovery, DurableLaunchRecoveryKind,
};
use crate::session_launch::{durable_launch_recovery_exists, persist_finalized_launch_session};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod docker_session_persistence_tests {
    use super::*;

    #[test]
    fn production_docker_session_reload_matches_finalized_process_worktree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let project = temp.path().join("project");
        let sessions_dir = temp.path().join("sessions");
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/production\n",
        )
        .expect("write compose");

        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .working_dir(&project)
            .branch("work/docker-production")
            .build();
        config.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        config.command = "codex".to_string();
        config.args = vec!["--no-alt-screen".to_string()];
        let runtime = crate::docker_launch::resolved_test_docker_runtime(temp.path());
        let plan = crate::docker_launch::resolve_docker_launch_plan(&project, Some("app"))
            .expect("resolve Docker launch plan");
        let binding = DockerLaunchBinding::capture_for_test(runtime.clone(), plan);
        let runtime_worktree =
            finalize_docker_agent_launch_config_with_binding(&mut config, Some(&binding))
                .expect("finalize Docker launch")
                .expect("Docker runtime worktree");

        let mut session = gwt_agent::Session::new(
            &project,
            "work/docker-production",
            gwt_agent::AgentId::Codex,
        );
        session.project_state_root = Some(project.clone());
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        let session_id = session.id.clone();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);

        persist_finalized_launch_session(
            &sessions_dir,
            Some(&runtime_path),
            &mut session,
            Some(&runtime_worktree),
        )
        .expect("persist finalized production launch");

        let process_launch = ProcessLaunch {
            initial_prompt_file: None,
            command: config.command,
            args: config.args,
            env: config.env_vars,
            remove_env: config.remove_env,
            cwd: config.working_dir,
            resource_policy: None,
        };
        assert_eq!(process_launch.command, runtime.binary());
        let workdir_index = process_launch
            .args
            .iter()
            .position(|arg| arg == "-w")
            .expect("compose exec -w");
        let process_worktree = process_launch
            .args
            .get(workdir_index + 1)
            .expect("compose exec worktree");
        let reloaded = gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
            .expect("reload production Session");
        let binding = reloaded
            .docker_runtime_binding
            .expect("persisted Docker binding");

        assert_eq!(
            binding.runtime_worktree_path,
            PathBuf::from(process_worktree)
        );
        assert_eq!(
            binding.project_state_scope_hash,
            gwt_core::paths::project_scope_hash(&project)
                .as_str()
                .to_string()
        );
        assert!(runtime_path.exists(), "runtime state must be persisted");
    }
}

#[cfg(test)]
mod agent_endpoint_env_tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    fn init_execution_repo(repo: &Path, branch: &str) {
        std::fs::create_dir_all(repo).expect("create execution repository");
        for args in [
            vec!["init", "-q", "-b", branch],
            vec![
                "remote",
                "add",
                "origin",
                "https://github.com/example/launch-binding.git",
            ],
        ] {
            let output = gwt_core::process::hidden_command("git")
                .args(&args)
                .current_dir(repo)
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    struct PersistedExecutionLaunch {
        project: PathBuf,
        sessions_dir: PathBuf,
        owner: crate::cli::execution_state::ExecutionOwnerKey,
        session: gwt_agent::Session,
    }

    fn persisted_execution_launch(home: &Path) -> PersistedExecutionLaunch {
        let project = home.join("project");
        init_execution_repo(&project, "work/issue-2359");
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Spec,
            number: 2359,
        };
        let mut session =
            gwt_agent::Session::new(&project, "work/issue-2359", gwt_agent::AgentId::Codex);
        session.project_state_root = Some(project.clone());
        session.linked_issue_number = Some(owner.number);
        session.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
        persist_finalized_launch_session(&sessions_dir, Some(&runtime_path), &mut session, None)
            .expect("persist finalized Session before authority");
        PersistedExecutionLaunch {
            project,
            sessions_dir,
            owner,
            session,
        }
    }

    fn rebound_relaunch_fixture(
        home: &Path,
    ) -> (
        PersistedExecutionLaunch,
        AgentCapabilityIssuer,
        gwt_agent::SessionExecutionBinding,
    ) {
        let mut launch = persisted_execution_launch(home);
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("materialize predecessor generation");
        let (receipt, binding) =
            crate::prepare_resume_producing_authority(&launch.project, &launch.session.id, false)
                .expect("recover producing authority for the relaunch");
        assert_eq!(
            receipt.outcome,
            crate::AgentExecutionContinuationOutcome::ReboundCurrent,
            "an Active ECR relaunch of its own primary session rebinds the current generation"
        );
        assert_eq!(binding.session_id, launch.session.id);
        let session_path = launch
            .sessions_dir
            .join(format!("{}.toml", launch.session.id));
        let mut predecessor =
            gwt_agent::Session::load(&session_path).expect("reload rebound predecessor");
        predecessor.update_status(gwt_agent::AgentStatus::Stopped);
        predecessor
            .save(&launch.sessions_dir)
            .expect("persist stopped rebound predecessor");
        let predecessor_identity = gwt_agent::SessionExecutionIdentity::from_session(&predecessor)
            .expect("read rebound predecessor identity")
            .expect("rebound predecessor must be bound");
        let host_started_at = crate::process::host_process_start_time(std::process::id())
            .expect("resolve test Host process identity");
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Stopped,
            &predecessor_identity,
            1,
            host_started_at,
            i32::MAX as u32,
            1,
        )
        .save(&gwt_agent::runtime_state_path(
            &launch.sessions_dir,
            &launch.session.id,
        ))
        .expect("persist exact stopped rebound runtime");
        (launch, issuer, binding)
    }

    #[test]
    fn failed_genesis_cache_tombstone_preserves_same_id_replacement() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let (launch, _, _) = rebound_relaunch_fixture(home.path());
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&launch.session)
            .expect("identity")
            .expect("bound Session");
        let mut cache =
            LaunchWizardMemoryCache::load_with_agent_options(&launch.sessions_dir, Vec::new());
        cache.sessions();
        let mut replacement = launch.session.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
        cache.record_prepared_session(&replacement.id, Some(replacement.clone()));

        cache.forget_prepared_genesis_session(&identity);

        assert_eq!(
            cache
                .session_by_id(&replacement.id)
                .map(|session| &session.agent_id),
            Some(&replacement.agent_id)
        );
    }

    #[test]
    fn autonomous_fresh_launch_imports_legacy_active_for_operator_recovery() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());
        launch.session.launch_route = gwt_agent::LaunchRoute::Autonomous;
        crate::cli::execution_state::materialize_at_launch(
            &launch.project,
            launch.owner.kind,
            launch.owner.number,
            "legacy-july-holder",
            "$gwt-execute #2359",
            false,
        )
        .expect("materialize legacy Active execution");
        let projection = crate::cli::trusted_store::read(&launch.project, "execution-control.json")
            .expect("read legacy projection")
            .expect("legacy projection");
        let mut legacy: serde_json::Value = serde_json::from_str(&projection).unwrap();
        legacy.as_object_mut().unwrap().remove("content_hash");
        legacy["launched_at"] = serde_json::json!("2026-07-28T00:00:00Z");
        crate::cli::trusted_store::write(
            &launch.project,
            "execution-control.json",
            &serde_json::to_vec(&legacy).unwrap(),
        )
        .expect("write generation-less July record");
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let install = |session: &mut gwt_agent::Session, env: &mut HashMap<String, String>| {
            FinalizedAgentCapabilityLaunch {
                issuer: Some(&issuer),
                sessions_dir: &launch.sessions_dir,
                session,
                project_root: &launch.project,
                worktree: &launch.project,
                producing_owner: Some(launch.owner),
                prepared_continuation: None,
                rebound_continuation: None,
                execution_entrypoint: "$gwt-execute #2359",
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                container_runtime: None,
                permission_decision: None,
            }
            .install(env)
        };
        let mut env = HashMap::new();
        let error = install(&mut launch.session, &mut env)
            .expect_err("unknown legacy holder must still prevent duplicate production");
        assert!(error.contains("already has an Active execution"), "{error}");
        assert!(env.is_empty());
        assert!(launch.session.execution_binding.is_none());
        let diagnosis = crate::cli::execution_state::diagnose_owner(&launch.project, launch.owner);
        assert_eq!(diagnosis.recommended_recovery, "issue.monitor.stop");
        assert_eq!(
            diagnosis.holder_session_id.as_deref(),
            Some("legacy-july-holder")
        );
        let imported = crate::cli::execution_state::load(&launch.project)
            .unwrap()
            .unwrap();
        assert_eq!(
            imported.status,
            crate::cli::execution_state::ExecutionControlStatus::Active
        );
        assert_eq!(
            imported.launched_at.to_rfc3339(),
            "2026-07-28T00:00:00+00:00"
        );
        assert!(matches!(
            crate::cli::execution_state::release_revoked_launch_generation(
                &launch.project,
                launch.owner,
                "operator releases stranded legacy execution",
            )
            .unwrap(),
            crate::cli::execution_state::LaunchGenerationRelease::Released { .. }
        ));
        let mut relaunch = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        relaunch.project_state_root = Some(launch.project.clone());
        relaunch.linked_issue_number = Some(launch.owner.number);
        relaunch.launch_route = gwt_agent::LaunchRoute::Autonomous;
        install(&mut relaunch, &mut env).expect("launch after operator recovery");
        assert!(relaunch.execution_binding.is_some());
        assert!(!env.is_empty());
    }

    #[test]
    fn autonomous_fresh_launch_refuses_active_owner_with_or_without_generation() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for state in ["legacy", "unknown", "live", "corrupt"] {
            let home = tempfile::tempdir().expect("home tempdir");
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let mut launch = persisted_execution_launch(home.path());
            let issuer = AgentCapabilityIssuer::for_test(
                "http://127.0.0.1:45123/internal/hook-live",
                "ws://127.0.0.1:46234/ws",
                "ws://127.0.0.1:45123/internal/pane-ws",
            );
            if state == "legacy" {
                crate::cli::execution_state::materialize_at_launch(
                    &launch.project,
                    launch.owner.kind,
                    launch.owner.number,
                    &launch.session.id,
                    "$gwt-execute #2359",
                    false,
                )
                .expect("materialize legacy Active execution");
            } else {
                FinalizedAgentCapabilityLaunch {
                    issuer: Some(&issuer),
                    sessions_dir: &launch.sessions_dir,
                    session: &mut launch.session,
                    project_root: &launch.project,
                    worktree: &launch.project,
                    producing_owner: Some(launch.owner),
                    prepared_continuation: None,
                    rebound_continuation: None,
                    execution_entrypoint: "$gwt-execute #2359",
                    runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                    container_runtime: None,
                    permission_decision: None,
                }
                .install(&mut HashMap::new())
                .expect("materialize the first producing generation");
            }
            if state == "live" {
                let identity = gwt_agent::SessionExecutionIdentity::from_session(&launch.session)
                    .expect("holder identity")
                    .expect("bound holder");
                let started_at = crate::process::host_process_start_time(std::process::id())
                    .expect("test process identity");
                gwt_agent::SessionRuntimeState::for_execution_process(
                    gwt_agent::AgentStatus::Running,
                    &identity,
                    1,
                    started_at,
                    std::process::id(),
                    started_at,
                )
                .save(&gwt_agent::runtime_state_path(
                    &launch.sessions_dir,
                    &launch.session.id,
                ))
                .expect("persist a live exact holder");
            } else if state == "corrupt" {
                crate::cli::trusted_store::write(
                    &launch.project,
                    "execution-generation-pointer.json",
                    b"{}",
                )
                .expect("invalidate the pointer");
                assert_eq!(
                    crate::cli::execution_state::load(&launch.project)
                        .expect("read fail-closed sentinel")
                        .expect("sentinel record")
                        .content_hash,
                    "invalid-generation-authority",
                );
            }
            let projection =
                crate::cli::trusted_store::read(&launch.project, "execution-control.json")
                    .expect("read Active projection");
            let pointer = crate::cli::trusted_store::read(
                &launch.project,
                "execution-generation-pointer.json",
            )
            .expect("read generation pointer");
            let ledger = crate::cli::execution_state::load_owner_generation_ledger(
                &launch.project,
                launch.owner,
            )
            .expect("read Active ledger");
            let mut relaunch = gwt_agent::Session::new(
                &launch.project,
                "work/issue-2359",
                gwt_agent::AgentId::Codex,
            );
            relaunch.project_state_root = Some(launch.project.clone());
            relaunch.linked_issue_number = Some(launch.owner.number);
            relaunch.launch_route = gwt_agent::LaunchRoute::Autonomous;
            let mut env = HashMap::new();

            let error = FinalizedAgentCapabilityLaunch {
                issuer: Some(&issuer),
                sessions_dir: &launch.sessions_dir,
                session: &mut relaunch,
                project_root: &launch.project,
                worktree: &launch.project,
                producing_owner: Some(launch.owner),
                prepared_continuation: None,
                rebound_continuation: None,
                execution_entrypoint: "$gwt-execute #2359",
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                container_runtime: None,
                permission_decision: None,
            }
            .install(&mut env)
            .expect_err("Monitor must not start a second producer for an Active owner");

            assert!(error.contains("already has an Active execution"), "{error}");
            assert!(relaunch.execution_binding.is_none());
            assert!(env.is_empty(), "a refused duplicate receives no capability");
            assert_eq!(
                crate::cli::trusted_store::read(&launch.project, "execution-control.json")
                    .expect("read preserved Active projection"),
                projection
            );
            if state == "legacy" {
                let imported = crate::cli::execution_state::load_owner_generation_ledger(
                    &launch.project,
                    launch.owner,
                )
                .expect("read imported ledger")
                .expect("legacy Active is visible to operator recovery");
                assert_eq!(imported.generations.len(), 1);
                assert!(imported.takeovers.is_empty());
                continue;
            }
            assert_eq!(
                crate::cli::trusted_store::read(
                    &launch.project,
                    "execution-generation-pointer.json"
                )
                .expect("read preserved generation pointer"),
                pointer
            );
            assert_eq!(
                crate::cli::execution_state::load_owner_generation_ledger(
                    &launch.project,
                    launch.owner
                )
                .expect("read preserved Active ledger"),
                ledger
            );
        }
    }

    #[test]
    fn autonomous_fresh_launch_recovers_stopped_holder_before_active_owner_gate() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for recovery in ["exited-child", "stopped-holder", "interrupted-reaper"] {
            let home = tempfile::tempdir().expect("home tempdir");
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let (launch, issuer, _) = rebound_relaunch_fixture(home.path());
            let holder_path = launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id));
            let mut holder = gwt_agent::Session::load(&holder_path).expect("reload holder");
            let holder_status = if recovery == "exited-child" {
                gwt_agent::AgentStatus::Running
            } else {
                gwt_agent::AgentStatus::Stopped
            };
            holder.update_status(holder_status);
            holder
                .save(&launch.sessions_dir)
                .expect("persist holder status");
            let holder_bytes = std::fs::read(&holder_path).expect("read holder");
            let identity = gwt_agent::SessionExecutionIdentity::from_session(&holder)
                .expect("holder identity")
                .expect("bound holder");
            let host_started_at = crate::process::host_process_start_time(std::process::id())
                .expect("test Host process identity");
            gwt_agent::SessionRuntimeState::for_execution_process(
                holder_status,
                &identity,
                1,
                host_started_at,
                i32::MAX as u32,
                1,
            )
            .save(&gwt_agent::runtime_state_path(
                &launch.sessions_dir,
                &holder.id,
            ))
            .expect("persist exited child under a live Host");
            let runtime = crate::cli::execution_state::classify_exact_session_runtime(
                &launch.sessions_dir,
                &identity,
            )
            .expect("classify holder");
            assert!(if recovery == "exited-child" {
                runtime == crate::cli::execution_state::ExactSessionRuntimeDisposition::ChildExited
            } else {
                matches!(
                    runtime,
                    crate::cli::execution_state::ExactSessionRuntimeDisposition::Terminal(_)
                )
            });

            if recovery == "interrupted-reaper" {
                let projection =
                    crate::cli::trusted_store::read(&launch.project, "execution-control.json")
                        .expect("read Active projection")
                        .expect("Active projection");
                let pointer = crate::cli::trusted_store::read(
                    &launch.project,
                    "execution-generation-pointer.json",
                )
                .expect("read Active pointer")
                .expect("Active pointer");
                let candidate =
                    crate::cli::execution_state::inspect_startup_active_generation_ledgers(
                        std::slice::from_ref(&launch.project),
                    )
                    .candidates
                    .into_iter()
                    .find(|candidate| candidate.owner == launch.owner)
                    .expect("exact Active holder candidate");
                crate::cli::execution_state::reap_startup_defunct_active_generation(
                    &candidate,
                    &launch.sessions_dir,
                    &identity,
                    &[],
                )
                .expect("commit the audited Blocked reaper event");
                // Keep the committed ledger but restore the publication that an
                // interrupted ledger-first write leaves behind.
                crate::cli::trusted_store::write(
                    &launch.project,
                    "execution-control.json",
                    projection.as_bytes(),
                )
                .expect("restore pre-reaper projection");
                crate::cli::trusted_store::write(
                    &launch.project,
                    "execution-generation-pointer.json",
                    pointer.as_bytes(),
                )
                .expect("restore pre-reaper pointer");
                assert_eq!(
                    crate::cli::execution_state::load(&launch.project)
                        .expect("read sentinel")
                        .expect("sentinel")
                        .content_hash,
                    "invalid-generation-authority"
                );
                assert_eq!(
                    crate::cli::execution_state::load_owner_generation_ledger(
                        &launch.project,
                        launch.owner
                    )
                    .expect("read committed ledger")
                    .expect("committed ledger")
                    .current_effective_status(),
                    Some(crate::cli::execution_state::ExecutionControlStatus::Blocked)
                );
            }

            let mut successor = gwt_agent::Session::new(
                &launch.project,
                "work/issue-2359",
                gwt_agent::AgentId::Codex,
            );
            successor.project_state_root = Some(launch.project.clone());
            successor.linked_issue_number = Some(launch.owner.number);
            successor.launch_route = gwt_agent::LaunchRoute::Autonomous;
            let mut env = HashMap::new();
            FinalizedAgentCapabilityLaunch {
                issuer: Some(&issuer),
                sessions_dir: &launch.sessions_dir,
                session: &mut successor,
                project_root: &launch.project,
                worktree: &launch.project,
                producing_owner: Some(launch.owner),
                prepared_continuation: None,
                rebound_continuation: None,
                execution_entrypoint: "$gwt-execute #2359",
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                container_runtime: None,
                permission_decision: None,
            }
            .install(&mut env)
            .unwrap_or_else(|error| {
                panic!("Monitor must recover {recovery} and prepare its Blocked successor: {error}")
            });

            let ledger =
                crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                    .expect("read reaped ledger")
                    .expect("owner ledger");
            assert!(ledger.current_generation_settled_by_host_reaper());
            assert_eq!(
                ledger.lifecycle_events.len(),
                1,
                "replay must preserve the exact audited settlement"
            );
            assert_eq!(
                ledger.current_effective_status(),
                Some(crate::cli::execution_state::ExecutionControlStatus::Blocked),
            );
            assert_eq!(
                ledger
                    .continuation_attempts
                    .last()
                    .expect("Prepared successor")
                    .status,
                crate::cli::execution_state::ContinuationAttemptStatus::Prepared,
            );
            assert!(env.contains_key(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV));
            assert_eq!(
                std::fs::read(&holder_path).expect("read holder after reaping"),
                holder_bytes
            );
        }
    }

    /// SPEC #3590 FR-009: each concurrent session settles its own generation.
    /// The predecessor is no longer current, so its settlement must land on
    /// the generation it holds instead of being refused for the successor's.
    #[test]
    fn concurrent_sessions_settle_their_own_generations_independently() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut genesis_env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut genesis_env)
        .expect("materialize the first producing generation");
        let holder_id = launch.session.id.clone();
        let holder_generation = gwt_agent::SessionExecutionIdentity::from_session(
            &gwt_agent::Session::load(&launch.sessions_dir.join(format!("{holder_id}.toml")))
                .expect("reload holder Session"),
        )
        .expect("read holder identity")
        .expect("holder must be bound")
        .execution_binding
        .identity
        .generation_id;

        let mut relaunch = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        relaunch.project_state_root = Some(launch.project.clone());
        relaunch.linked_issue_number = Some(launch.owner.number);
        relaunch.update_status(gwt_agent::AgentStatus::Running);
        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut relaunch,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("a concurrent launch must start beside a live holder");
        let successor_generation = relaunch
            .execution_binding
            .clone()
            .expect("the concurrent launch must be bound")
            .identity
            .generation_id;

        // The launch leaves its successor Prepared; the runtime activates it
        // when the pane starts producing. Drive that step so both generations
        // are live at once, which is the state FR-009 is about.
        let prepared =
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("load owner ledger")
                .expect("owner ledger exists")
                .continuation_attempts
                .iter()
                .rev()
                .find(|attempt| {
                    attempt.request.source
                        == crate::cli::execution_state::CONCURRENT_LINKED_OWNER_LAUNCH_SOURCE
                })
                .expect("the concurrent launch must record a successor attempt")
                .request
                .clone();
        crate::cli::execution_state::activate_successor(&launch.project, launch.owner, &prepared)
            .expect("activate the concurrent successor");

        let settled = crate::cli::execution_state::settle(
            &launch.project,
            &holder_id,
            crate::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("the predecessor session must be able to settle");
        assert!(
            matches!(
                settled,
                crate::cli::execution_state::SettleResult::Settled(ref record)
                    if record.primary_session_id == holder_id
                        && record.status
                            == crate::cli::execution_state::ExecutionControlStatus::Completed
            ),
            "the predecessor must settle its own generation: {settled:?}"
        );

        let ledger =
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("load owner ledger")
                .expect("owner ledger exists");
        assert!(
            ledger.lifecycle_events.iter().any(|event| {
                event.generation_id == holder_generation
                    && event.to_status
                        == crate::cli::execution_state::ExecutionControlStatus::Completed
            }),
            "the predecessor settlement must be audited on its own generation"
        );
        assert_eq!(
            ledger.current_generation_id, successor_generation,
            "settling the predecessor must not move the current generation"
        );
        assert_eq!(
            crate::cli::execution_state::load(&launch.project)
                .expect("load flat execution projection")
                .expect("flat projection exists")
                .primary_session_id,
            relaunch.id,
            "the flat projection must keep naming the current generation's session"
        );
    }

    /// Issue #3457: an unreachable holder is not merely concurrent. Once its
    /// Host is gone the durable `.toml` is an orphan with no runtime sidecar in
    /// any namespace, so nothing can ever settle that generation. It is
    /// terminalized under exact proof and released to a successor rather than
    /// left Active beside the new launch forever.
    #[test]
    fn fresh_launch_supersedes_a_generation_whose_holder_is_unreachable() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut genesis_env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut genesis_env)
        .expect("materialize the first producing generation");

        // The Host process that owned the holder is gone: its runtime
        // namespace was cleared and the durable record still reads Running.
        let holder_id = launch.session.id.clone();
        std::fs::remove_file(gwt_agent::runtime_state_path(
            &launch.sessions_dir,
            &holder_id,
        ))
        .expect("clear the holder runtime sidecar");

        let mut relaunch = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        relaunch.project_state_root = Some(launch.project.clone());
        relaunch.linked_issue_number = Some(launch.owner.number);
        relaunch.update_status(gwt_agent::AgentStatus::Running);

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut relaunch,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("a fresh launch must supersede an unreachable holder");

        assert!(
            env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "the superseding launch must receive its producing capability"
        );
        let binding = relaunch
            .execution_binding
            .clone()
            .expect("the superseding launch must be bound");
        assert_eq!(binding.session_id, relaunch.id);
        assert_ne!(
            binding.identity.generation_id,
            gwt_agent::Session::load(&launch.sessions_dir.join(format!("{holder_id}.toml")))
                .expect("reload holder Session")
                .execution_binding
                .expect("holder binding")
                .identity
                .generation_id,
            "the successor must own a new generation, not the holder's"
        );
    }

    #[test]
    fn fresh_launch_retries_three_abandoned_prepared_candidates_without_receipts() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let install = |session: &mut gwt_agent::Session| {
            FinalizedAgentCapabilityLaunch {
                issuer: Some(&issuer),
                sessions_dir: &launch.sessions_dir,
                session,
                project_root: &launch.project,
                worktree: &launch.project,
                producing_owner: Some(launch.owner),
                prepared_continuation: None,
                rebound_continuation: None,
                execution_entrypoint: "$gwt-execute #2359",
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                container_runtime: None,
                permission_decision: None,
            }
            .install(&mut HashMap::new())
        };
        install(&mut launch.session).expect("genesis");
        std::fs::remove_file(gwt_agent::runtime_state_path(
            &launch.sessions_dir,
            &launch.session.id,
        ))
        .expect("old holder Host disappeared");
        let new_candidate = || {
            let mut session = gwt_agent::Session::new(
                &launch.project,
                "work/issue-2359",
                gwt_agent::AgentId::Codex,
            );
            session.project_state_root = Some(launch.project.clone());
            session.linked_issue_number = Some(launch.owner.number);
            session.update_status(gwt_agent::AgentStatus::Running);
            session
        };
        let mut candidate = new_candidate();
        install(&mut candidate).expect("first candidate");
        for failure in 0..3 {
            let attempt =
                crate::cli::execution_state::prepared_fresh_linked_owner_launch_for_session(
                    &launch.project,
                    launch.owner,
                    &candidate.id,
                )
                .expect("read attempt")
                .expect("Prepared attempt");
            let mut handshake = crate::cli::execution_state::claim_prepared_session_launch(
                &launch.project,
                launch.owner,
                &launch.sessions_dir,
                &candidate,
            )
            .expect("claim candidate")
            .expect("handshake");
            clear_durable_launch_recovery(&launch.sessions_dir, &candidate.id)
                .expect("simulate lost receipt");
            let mut next = new_candidate();
            if failure == 0 {
                let candidate_path = launch.sessions_dir.join(format!("{}.toml", candidate.id));
                let before = std::fs::read(&candidate_path).expect("live candidate bytes");
                install(&mut next).expect("independent launch beside live Prepared candidate");
                assert_eq!(
                    std::fs::read(&candidate_path).expect("preserved live candidate"),
                    before
                );
                assert_eq!(
                    crate::cli::execution_state::continuation_attempt_for_operation(
                        &launch.project,
                        launch.owner,
                        &attempt.request.operation_id,
                    )
                    .expect("read live attempt")
                    .expect("live attempt")
                    .status,
                    crate::cli::execution_state::ContinuationAttemptStatus::Prepared,
                );
                next = new_candidate();
            }
            handshake.host_pid = i32::MAX as u32;
            handshake.host_started_at = 1;
            std::fs::write(
                gwt_agent::active_launch_handshake_path(&launch.sessions_dir, &candidate.id),
                serde_json::to_vec_pretty(&handshake).expect("serialize dead Host proof"),
            )
            .expect("persist exact dead Host handshake");
            let prepared_work =
                gwt_core::workspace_projection::transact_workspace_state_with_commit(
                    &launch.project,
                    &attempt.request.operation_id,
                    |_projection, _work_items, _| Ok(((), Vec::new())),
                    || {
                        Err(gwt_core::error::GwtError::Other(
                            "simulate Host exit before Work commit".to_string(),
                        ))
                    },
                );
            assert!(prepared_work.is_err());
            install(&mut next).expect("next launch must release the exact abandoned candidate");
            assert_eq!(
                crate::cli::execution_state::continuation_attempt_for_operation(
                    &launch.project,
                    launch.owner,
                    &attempt.request.operation_id,
                )
                .expect("read aborted attempt")
                .expect("attempt")
                .status,
                crate::cli::execution_state::ContinuationAttemptStatus::Aborted,
            );
            assert_eq!(
                gwt_core::workspace_projection::workspace_state_external_commit_resolution(
                    &launch.project,
                    &attempt.request.operation_id,
                )
                .expect("read rejected Work transaction"),
                gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected,
            );
            assert!(!launch
                .sessions_dir
                .join(format!("{}.toml", candidate.id))
                .exists());
            candidate = next;
        }
    }

    /// Issue #3759: the owner ledger is committed before the worktree-scoped
    /// pointer/projection pair (`write_activated_generation`), and trusted-
    /// store GC of a removed worktree deletes that pair while the owner ledger
    /// stays. Either way the next Issue Monitor / Start Work launch on the
    /// same worktree used to die before the PTY with "execution generation
    /// pointer is missing after ledger ownership was established", burning
    /// every retry. A lost publication is a pure function of the ledger and
    /// must be rebuilt before the fresh-launch preflight reads authority.
    #[test]
    fn fresh_launch_republishes_a_lost_generation_pointer_before_reading_authority() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut genesis_env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut genesis_env)
        .expect("materialize the first producing generation");

        // The holder's Host is gone (forced restart) ...
        let holder_id = launch.session.id.clone();
        std::fs::remove_file(gwt_agent::runtime_state_path(
            &launch.sessions_dir,
            &holder_id,
        ))
        .expect("clear the holder runtime sidecar");
        // ... and the worktree-scoped publication was lost while the owner
        // ledger survived (GC of the trusted worktree directory).
        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(&launch.project)
            .expect("trusted worktree directory");
        std::fs::remove_file(trusted_dir.join("execution-generation-pointer.json"))
            .expect("drop trusted pointer");
        std::fs::remove_file(trusted_dir.join("execution-control.json"))
            .expect("drop trusted projection");
        let _ = std::fs::remove_file(
            launch
                .project
                .join(".gwt/skill-state/execution-generation-pointer.json"),
        );
        let _ = std::fs::remove_file(
            launch
                .project
                .join(".gwt/skill-state/execution-control.json"),
        );
        let error =
            crate::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect_err("fixture must reproduce the strict-view failure");
        assert!(
            error.to_string().contains(
                "execution generation pointer is missing after ledger ownership was established"
            ),
            "{error}"
        );

        let mut relaunch = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        relaunch.project_state_root = Some(launch.project.clone());
        relaunch.linked_issue_number = Some(launch.owner.number);
        relaunch.update_status(gwt_agent::AgentStatus::Running);

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut relaunch,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("a fresh launch must self-heal a lost pointer instead of failing before the PTY");

        assert!(
            env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "the healed launch must receive its producing capability"
        );
        let binding = relaunch
            .execution_binding
            .clone()
            .expect("the healed launch must be bound");
        assert_eq!(binding.session_id, relaunch.id);
        // The successor is Prepared at install time and activates once the
        // PTY handshake completes, so the published current generation is
        // still the healed predecessor; what matters is that the strict view
        // is readable again instead of failing before the PTY.
        assert!(
            crate::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("strict view is readable again")
                .is_some(),
            "the healed pointer/projection must publish the predecessor generation"
        );
        assert_ne!(
            binding.identity.generation_id,
            gwt_agent::Session::load(&launch.sessions_dir.join(format!("{holder_id}.toml")))
                .expect("reload holder Session")
                .execution_binding
                .expect("holder binding")
                .identity
                .generation_id,
            "the successor must own a new generation, not the holder's"
        );
    }

    /// Issue #3472: a `Completed` generation is already settled — its holder
    /// returned its authority on the way out, so nothing can ever reopen it in
    /// place. Refusing a later fresh launch there permanently strands the
    /// owner: only the manual Launch Agent wizard reaches the Completed
    /// successor route, so Issue Monitor retries and Start Work collide
    /// forever with a generation that finished successfully. A Completed
    /// predecessor must mint the next lifetime instead of blocking it.
    #[test]
    fn fresh_launch_succeeds_over_a_completed_generation() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut genesis_env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut genesis_env)
        .expect("materialize the first producing generation");

        // The predecessor settles cleanly and its Host exits. The durable
        // Session and its runtime sidecar both survive, so the holder is
        // *reachable* evidence — the unreachable-holder route of Issue #3457
        // deliberately does not fire here.
        let holder_id = launch.session.id.clone();
        assert!(
            matches!(
                crate::cli::execution_state::settle(
                    &launch.project,
                    &holder_id,
                    crate::cli::execution_state::ExecutionSettlement::Completed,
                )
                .expect("settle the predecessor generation"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ),
            "the fixture must reach a Completed generation"
        );
        let mut holder =
            gwt_agent::Session::load(&launch.sessions_dir.join(format!("{holder_id}.toml")))
                .expect("reload holder Session");
        holder.update_status(gwt_agent::AgentStatus::Stopped);
        holder
            .save(&launch.sessions_dir)
            .expect("persist stopped holder");
        assert_eq!(
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("read ledger")
                .expect("ledger")
                .current_effective_status(),
            Some(crate::cli::execution_state::ExecutionControlStatus::Completed)
        );
        let predecessor_binding =
            crate::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read completed predecessor binding")
                .expect("completed predecessor generation");

        let mut relaunch = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        relaunch.project_state_root = Some(launch.project.clone());
        relaunch.linked_issue_number = Some(launch.owner.number);
        relaunch.update_status(gwt_agent::AgentStatus::Running);

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut relaunch,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("a fresh launch must start a successor over a Completed generation");

        assert_eq!(
            crate::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read current generation"),
            Some(predecessor_binding),
            "pre-readiness preparation must preserve the Completed predecessor as current",
        );
        assert!(
            env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "the successor launch must receive its producing capability"
        );
        let binding = relaunch
            .execution_binding
            .clone()
            .expect("the successor launch must be bound");
        assert_eq!(binding.session_id, relaunch.id);
        let ledger =
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("read ledger after the successor launch")
                .expect("ledger after the successor launch");
        assert_ne!(
            binding.identity.generation_id,
            gwt_agent::Session::load(&launch.sessions_dir.join(format!("{holder_id}.toml")))
                .expect("reload holder Session")
                .execution_binding
                .expect("holder binding")
                .identity
                .generation_id,
            "the successor must own a new generation, not the settled one"
        );
        // The audit trail must state which terminal state the successor left,
        // so a later reader can tell a completed handoff from a recovery.
        let attempt = ledger
            .continuation_attempts
            .last()
            .expect("the successor launch must append a continuation attempt");
        assert_eq!(
            attempt.predecessor_status,
            crate::cli::execution_state::SuccessorPredecessorStatus::Completed,
            "the attempt must record that its predecessor was Completed"
        );
        assert_eq!(
            attempt.request.source,
            crate::cli::execution_state::MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE,
            "a Completed predecessor uses the Completed owner-launch source"
        );
        assert!(
            crate::cli::execution_state::is_owner_launch_successor_attempt(attempt),
            "the attempt must classify as an owner launch successor"
        );
        assert!(
            ledger.lifecycle_events.iter().any(|event| {
                event.to_status == crate::cli::execution_state::ExecutionControlStatus::Completed
                    && event.session_id == holder_id
            }),
            "the settled predecessor transition must remain in the lifecycle audit trail"
        );
    }

    /// Issue #3472: an Active generation held by a Session that durably
    /// stopped and left an exact terminal runtime exit record can never
    /// settle itself. The genesis path must supersede it through the exact
    /// terminal proof transaction instead of refusing forever, exactly like
    /// the manual Launch Agent preflight already does.
    #[test]
    fn fresh_launch_supersedes_active_generation_with_exact_terminal_holder_proof() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut genesis_env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut genesis_env)
        .expect("materialize the first producing generation");

        // The holder durably stopped and its runtime sidecar records the
        // exact dead child process, so no Host can ever settle it.
        let holder_id = launch.session.id.clone();
        let mut holder =
            gwt_agent::Session::load(&launch.sessions_dir.join(format!("{holder_id}.toml")))
                .expect("reload holder Session");
        holder.update_status(gwt_agent::AgentStatus::Stopped);
        holder
            .save(&launch.sessions_dir)
            .expect("persist stopped holder");
        let holder_identity = gwt_agent::SessionExecutionIdentity::from_session(&holder)
            .expect("read holder identity")
            .expect("holder must be bound");
        let host_started_at = crate::process::host_process_start_time(std::process::id())
            .expect("resolve test Host process identity");
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Stopped,
            &holder_identity,
            1,
            host_started_at,
            i32::MAX as u32,
            1,
        )
        .save(&gwt_agent::runtime_state_path(
            &launch.sessions_dir,
            &holder_id,
        ))
        .expect("persist exact terminal holder runtime");

        let mut relaunch = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        relaunch.project_state_root = Some(launch.project.clone());
        relaunch.linked_issue_number = Some(launch.owner.number);
        relaunch.update_status(gwt_agent::AgentStatus::Running);

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut relaunch,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("a fresh launch must supersede a holder with exact terminal proof");

        assert!(
            env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "the superseding launch must receive its producing capability"
        );
        let binding = relaunch
            .execution_binding
            .clone()
            .expect("the superseding launch must be bound");
        assert_ne!(
            binding.identity.generation_id,
            holder_identity.execution_binding.identity.generation_id,
            "the successor must own a new generation, not the holder's"
        );
        let ledger =
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("load owner ledger")
                .expect("owner ledger exists");
        assert!(
            ledger.lifecycle_events.iter().any(|event| {
                event.generation_id == holder_identity.execution_binding.identity.generation_id
                    && event.from_status
                        == crate::cli::execution_state::ExecutionControlStatus::Active
                    && event.to_status
                        == crate::cli::execution_state::ExecutionControlStatus::Blocked
                    && event
                        .operation_id
                        .as_deref()
                        .is_some_and(|id| id.ends_with(":terminalize-predecessor"))
            }),
            "the supersede must audit the exact predecessor terminalization"
        );
    }

    #[test]
    fn rebound_current_resume_installs_active_authority_for_predecessor_session() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let (launch, issuer, binding) = rebound_relaunch_fixture(home.path());

        let durable_predecessor = gwt_agent::Session::load(
            &launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id)),
        )
        .expect("reload rebound predecessor");
        let predecessor_identity =
            gwt_agent::SessionExecutionIdentity::from_session(&durable_predecessor)
                .expect("read rebound predecessor identity")
                .expect("rebound predecessor must be bound");
        let predecessor_runtime = gwt_agent::SessionRuntimeState::load(
            &gwt_agent::runtime_state_path(&launch.sessions_dir, &launch.session.id),
        )
        .expect("load rebound predecessor runtime");
        assert_eq!(durable_predecessor.status, gwt_agent::AgentStatus::Stopped);
        assert_eq!(
            predecessor_runtime.execution_identity.as_ref(),
            Some(&predecessor_identity),
            "ReboundCurrent requires exact terminal predecessor proof",
        );

        // Issue #3423 root cause: a ReboundCurrent binding has no Prepared
        // attempt, so the Prepared install validator must never see it.
        assert!(
            !crate::cli::execution_state::prepared_execution_binding_matches(
                &launch.project,
                launch.owner,
                &launch.session.id,
                &binding.identity,
            )
            .expect("read Prepared authority"),
            "a rebound current-generation binding never matches a Prepared attempt"
        );

        let mut resumed = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        resumed.project_state_root = Some(launch.project.clone());
        resumed.linked_issue_number = Some(launch.owner.number);
        resumed.id = binding.session_id.clone();
        resumed
            .set_execution_binding(Some(binding.clone()))
            .expect("carry rebound binding on the resumed Session");
        resumed.update_status(gwt_agent::AgentStatus::Running);

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut resumed,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: None,
            prepared_continuation: None,
            rebound_continuation: Some(&binding),
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("install rebound continuation authority");

        let token = env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("rebound capability token");
        let grant = issuer
            .grant_for_test(token)
            .expect("authenticate issued capability");
        assert!(
            grant.principal().authorizes_producing_mutation(),
            "a rebound relaunch must recover producing authority"
        );
        assert_eq!(grant.principal().execution_binding(), Some(&binding));
    }

    /// Issue #4207: drive the Issue Monitor resume coordinator over a
    /// predecessor generation that is already settled — exactly what a Session
    /// that finished its work and pushed leaves behind.
    ///
    /// The coordinator answers `SuccessorCreated` there, and that answer comes
    /// out of `activate_successor_with_session_rebind`: the successor is
    /// planned *and activated* under one lease, so the binding it hands back is
    /// Active authority for a generation that is already current.
    fn successor_created_relaunch_fixture(
        home: &Path,
    ) -> (
        PersistedExecutionLaunch,
        AgentCapabilityIssuer,
        gwt_agent::SessionExecutionBinding,
    ) {
        let mut launch = persisted_execution_launch(home);
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("materialize predecessor generation");

        let holder_id = launch.session.id.clone();
        assert!(
            matches!(
                crate::cli::execution_state::settle(
                    &launch.project,
                    &holder_id,
                    crate::cli::execution_state::ExecutionSettlement::Completed,
                )
                .expect("settle the predecessor generation"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ),
            "the fixture must reach a Completed generation"
        );
        let mut holder =
            gwt_agent::Session::load(&launch.sessions_dir.join(format!("{holder_id}.toml")))
                .expect("reload holder Session");
        holder.update_status(gwt_agent::AgentStatus::Stopped);
        holder
            .save(&launch.sessions_dir)
            .expect("persist stopped holder");
        // The predecessor incarnation ran and exited: its runtime sidecar
        // survives, still describing the generation it held and a child that
        // is provably gone. That is what a relaunch actually finds on disk.
        let holder_identity = gwt_agent::SessionExecutionIdentity::from_session(&holder)
            .expect("read holder identity")
            .expect("holder must be bound");
        let host_started_at = crate::process::host_process_start_time(std::process::id())
            .expect("resolve test Host process identity");
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Stopped,
            &holder_identity,
            1,
            host_started_at,
            i32::MAX as u32,
            1,
        )
        .save(&gwt_agent::runtime_state_path(
            &launch.sessions_dir,
            &holder_id,
        ))
        .expect("persist the finished predecessor incarnation");

        let (receipt, binding) =
            crate::prepare_resume_producing_authority(&launch.project, &holder_id, false)
                .expect("recover producing authority for the relaunch");
        assert_eq!(
            receipt.outcome,
            crate::AgentExecutionContinuationOutcome::SuccessorCreated,
            "a settled predecessor cannot rebind in place; it mints a successor"
        );
        assert_eq!(binding.session_id, holder_id);
        (launch, issuer, binding)
    }

    /// Issue #4207 AC-1 / AC-2: a `SuccessorCreated` resume must launch.
    ///
    /// The launch worker used to route this outcome into the Prepared install
    /// arm on the premise that "only `SuccessorCreated` carries a Prepared
    /// attempt". The coordinator activates the successor it creates, so that
    /// premise is false: `prepared_execution_binding_matches` looks for an
    /// attempt that is already `Activated`, answers `false`, and the launch
    /// dies before the PTY with `Prepared continuation no longer matches its
    /// owner generation attempt`. Every resume of a Session that had settled
    /// its generation — i.e. every Issue that had progressed far enough to
    /// push — was refused that way, permanently.
    ///
    /// The binding is Active authority for the current generation, so the
    /// install must recover it as such instead of parking the owner.
    #[test]
    fn successor_created_resume_installs_active_authority_instead_of_refusing_as_prepared() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let (launch, issuer, binding) = successor_created_relaunch_fixture(home.path());

        assert_eq!(
            crate::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read current generation"),
            Some(binding.identity.clone()),
            "the coordinator already activated the successor it handed back"
        );
        assert!(
            !crate::cli::execution_state::prepared_execution_binding_matches(
                &launch.project,
                launch.owner,
                &binding.session_id,
                &binding.identity,
            )
            .expect("read Prepared authority"),
            "an activated successor can never satisfy the Prepared validator — \
             this is the exact reading that refused the launch"
        );

        let mut resumed = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        resumed.project_state_root = Some(launch.project.clone());
        resumed.linked_issue_number = Some(launch.owner.number);
        resumed.id = binding.session_id.clone();
        resumed
            .set_execution_binding(Some(binding.clone()))
            .expect("carry the activated binding on the resumed Session");
        resumed.update_status(gwt_agent::AgentStatus::Running);

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut resumed,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: None,
            prepared_continuation: Some(&binding),
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("a SuccessorCreated resume must install its already-active authority");

        let token = env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("resumed capability token");
        let grant = issuer
            .grant_for_test(token)
            .expect("authenticate issued capability");
        assert!(
            grant.principal().authorizes_producing_mutation(),
            "the resume must recover producing authority"
        );
        assert_eq!(grant.principal().execution_binding(), Some(&binding));
    }

    #[test]
    fn rebound_continuation_rejects_stale_generation_binding() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let (launch, issuer, binding) = rebound_relaunch_fixture(home.path());

        let mut stale = binding.clone();
        stale.identity.generation_id = "gen-stale".to_string();
        let mut resumed = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        resumed.project_state_root = Some(launch.project.clone());
        resumed.linked_issue_number = Some(launch.owner.number);
        resumed.id = stale.session_id.clone();
        resumed
            .set_execution_binding(Some(stale.clone()))
            .expect("carry stale binding");

        let mut env = HashMap::new();
        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut resumed,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: None,
            prepared_continuation: None,
            rebound_continuation: Some(&stale),
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect_err("a stale rebound binding must fail closed");
        assert!(
            error.contains(
                "Rebound continuation no longer matches the current execution generation"
            ),
            "{error}"
        );
        assert!(
            !env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "a rejected rebound install must not issue a capability"
        );
    }

    #[test]
    fn rebound_continuation_launch_conflict_exposes_retryable_error() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let (launch, issuer, binding) = rebound_relaunch_fixture(home.path());
        let expected = gwt_agent::SessionExecutionIdentity::for_binding(&launch.session, &binding)
            .expect("exact rebound Session identity");
        let held = crate::cli::execution_state::begin_active_session_launch_handshake(
            &launch.sessions_dir,
            &expected,
        )
        .expect("begin competing launch")
        .expect("hold exact Active launch handshake");
        let mut resumed = launch.session.clone();
        let mut env = HashMap::new();

        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut resumed,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: None,
            prepared_continuation: None,
            rebound_continuation: Some(&binding),
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install_with_prepared_claim(&mut env, None)
        .expect_err("competing exact Active launch must be refused");

        let value = serde_json::to_value(error).expect("serialize launch refusal");
        assert_eq!(value["error_code"], "active_launch_conflict");
        assert_eq!(value["retryable"], true);
        assert!(!env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV));
        assert!(
            crate::cli::execution_state::finish_active_session_launch_handshake(
                &launch.sessions_dir,
                &held,
            )
            .expect("finish competing exact handshake")
        );

        let error =
            gwt_agent::with_session_path_lease(&launch.sessions_dir, &launch.session.id, |_| {
                Ok(FinalizedAgentCapabilityLaunch {
                    issuer: Some(&issuer),
                    sessions_dir: &launch.sessions_dir,
                    session: &mut resumed,
                    project_root: &launch.project,
                    worktree: &launch.project,
                    producing_owner: None,
                    prepared_continuation: None,
                    rebound_continuation: Some(&binding),
                    execution_entrypoint: "$gwt-execute #2359",
                    runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                    container_runtime: None,
                    permission_decision: None,
                }
                .install_with_prepared_claim(&mut env, None)
                .expect_err("held Session lease must refuse the handshake"))
            })
            .expect("hold Session lease without relying on elapsed time");
        let value = serde_json::to_value(error).expect("serialize lease refusal");
        assert_eq!(value["error_code"], "active_launch_conflict");
        assert_eq!(value["retryable"], true);
        assert!(!env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV));
    }

    #[test]
    fn rebound_continuation_authority_refusal_exposes_nonretryable_error() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let (launch, issuer, binding) = rebound_relaunch_fixture(home.path());
        let mut replacement = launch.session.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
        replacement
            .save(&launch.sessions_dir)
            .expect("persist different durable Session authority");
        let replacement_path = launch.sessions_dir.join(format!("{}.toml", replacement.id));
        let before = std::fs::read(&replacement_path).expect("read replacement Session");
        let mut resumed = launch.session.clone();
        let mut env = HashMap::new();

        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut resumed,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: None,
            prepared_continuation: None,
            rebound_continuation: Some(&binding),
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install_with_prepared_claim(&mut env, None)
        .expect_err("different durable Session authority must fail closed");

        let value = serde_json::to_value(error).expect("serialize authority refusal");
        assert_eq!(value["error_code"], "active_launch_authority_rejected");
        assert_eq!(value["retryable"], false);
        assert!(!env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV));
        assert_eq!(std::fs::read(&replacement_path).unwrap(), before);
    }

    #[test]
    fn rebound_continuation_cannot_enter_genesis_launch_path() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let (launch, issuer, binding) = rebound_relaunch_fixture(home.path());

        let mut resumed = gwt_agent::Session::new(
            &launch.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        resumed.project_state_root = Some(launch.project.clone());
        resumed.id = binding.session_id.clone();

        let mut env = HashMap::new();
        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut resumed,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: Some(&binding),
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect_err("rebound continuation must not mint genesis authority");
        assert!(
            error.contains("Rebound continuation cannot enter the genesis execution launch path"),
            "{error}"
        );
    }

    #[test]
    fn prepared_capability_issuance_retains_same_id_replacement() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut predecessor = persisted_execution_launch(home.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &predecessor.sessions_dir,
            session: &mut predecessor.session,
            project_root: &predecessor.project,
            worktree: &predecessor.project,
            producing_owner: Some(predecessor.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut HashMap::new())
        .expect("materialize predecessor generation");
        crate::cli::execution_state::settle(
            &predecessor.project,
            &predecessor.session.id,
            crate::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor");

        let continuation_session_id = uuid::Uuid::new_v4().to_string();
        let request = crate::cli::execution_state::SuccessorRequest {
            operation_id: "prepared-replacement-operation".to_string(),
            principal_id: "host-test-principal".to_string(),
            work_id: Some("work-prepared-replacement".to_string()),
            source: "continue-work".to_string(),
            session_binding_id: uuid::Uuid::new_v4().to_string(),
            initial_session_id: continuation_session_id.clone(),
            entrypoint: "resume".to_string(),
            requested_at: chrono::Utc::now(),
        };
        crate::cli::execution_state::prepare_successor(
            &predecessor.project,
            predecessor.owner,
            &request,
        )
        .expect("prepare successor");
        let planned_identity = crate::cli::execution_state::prepared_successor_execution_binding(
            &predecessor.project,
            predecessor.owner,
            &request,
        )
        .expect("derive future binding");
        let mut candidate = gwt_agent::Session::new(
            &predecessor.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        candidate.id = continuation_session_id;
        candidate.project_state_root = Some(predecessor.project.clone());
        candidate.linked_issue_number = Some(predecessor.owner.number);
        candidate.update_status(gwt_agent::AgentStatus::Running);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: candidate.id.clone(),
            repo_hash: candidate
                .repo_hash
                .clone()
                .expect("candidate repository hash"),
            owner_kind: predecessor.owner.kind.as_str().to_string(),
            owner_number: predecessor.owner.number,
            identity: planned_identity,
            capability_generation: 1,
        };
        candidate
            .set_execution_binding(Some(binding.clone()))
            .expect("bind candidate");

        let session_path = predecessor
            .sessions_dir
            .join(format!("{}.toml", candidate.id));
        let mut replacement = candidate.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
        replacement
            .save(&predecessor.sessions_dir)
            .expect("save same-id replacement");
        let replacement_before =
            std::fs::read(&session_path).expect("read replacement Session bytes");
        let mut env = HashMap::new();

        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &predecessor.sessions_dir,
            session: &mut candidate,
            project_root: &predecessor.project,
            worktree: &predecessor.project,
            producing_owner: None,
            prepared_continuation: Some(&binding),
            rebound_continuation: None,
            execution_entrypoint: "resume",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect_err("capability issuance must reject a same-id replacement");

        assert!(
            error.contains("already exists"),
            "replacement conflict should be actionable: {error}"
        );
        assert_eq!(
            std::fs::read(&session_path).expect("read retained replacement"),
            replacement_before,
            "capability issuance must retain the replacement byte-identically"
        );
        assert!(
            !env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "no capability may be issued after Session create-if-absent rejection"
        );
    }

    #[test]
    fn explicit_linked_owner_launch_imports_generationless_blocked_predecessor_before_preparing_fresh_lifetime(
    ) {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let project = home.path().join("project");
        init_execution_repo(&project, "work/issue-1974");
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 1974,
        };
        let legacy_session_id = "fe3a92d8-60c6-46ae-8a83-1b3bac0891fa";
        crate::cli::execution_state::materialize_at_launch(
            &project,
            owner.kind,
            owner.number,
            legacy_session_id,
            "$gwt-execute #1974",
            false,
        )
        .expect("materialize legacy flat execution");
        assert!(matches!(
            crate::cli::execution_state::settle(
                &project,
                legacy_session_id,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "legacy delivery stopped before PR handoff".to_string(),
                    missing_verification: Some("legacy browser verification".to_string()),
                },
            )
            .expect("settle legacy flat execution"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        assert!(
            crate::cli::execution_state::current_execution_binding(&project, owner)
                .expect("probe generation-less legacy state")
                .is_none(),
            "the regression fixture must not pre-create a generation ledger"
        );
        let legacy_projection = crate::cli::trusted_store::read(&project, "execution-control.json")
            .expect("read trusted legacy projection")
            .expect("trusted legacy projection");

        let mut successor =
            gwt_agent::Session::new(&project, "work/issue-1974", gwt_agent::AgentId::Codex);
        successor.project_state_root = Some(project.clone());
        successor.linked_issue_number = Some(owner.number);
        successor.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &successor.id);
        persist_finalized_launch_session(&sessions_dir, Some(&runtime_path), &mut successor, None)
            .expect("persist fresh launch Session before authority");
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();

        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &sessions_dir,
            session: &mut successor,
            project_root: &project,
            worktree: &project,
            producing_owner: Some(owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #1974",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("prepare a fresh lifetime from the generation-less Blocked predecessor");

        assert_eq!(
            crate::cli::trusted_store::read(&project, "execution-control.json")
                .expect("read imported predecessor projection")
                .expect("imported predecessor projection"),
            legacy_projection,
            "pre-readiness launch must preserve the exact terminal predecessor bytes"
        );
        let ledger = crate::cli::execution_state::load_generation_ledger(&project, owner)
            .expect("read imported legacy ledger")
            .expect("imported legacy ledger");
        assert_eq!(ledger.generations.len(), 1);
        assert_eq!(
            ledger.generations[0].execution_control_json, legacy_projection,
            "legacy terminal bytes must remain the immutable predecessor snapshot"
        );
        assert_eq!(
            ledger.current_effective_status(),
            Some(crate::cli::execution_state::ExecutionControlStatus::Blocked)
        );
        assert_eq!(
            ledger.generations[0].identity.initial_session_id,
            legacy_session_id
        );
        let persisted =
            gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", successor.id)))
                .expect("reload fresh Prepared Session");
        let binding = persisted
            .execution_binding
            .expect("fresh launch must persist a Prepared successor binding");
        assert_ne!(
            crate::cli::execution_state::current_execution_binding(&project, owner)
                .expect("read current predecessor binding")
                .expect("current predecessor binding"),
            binding.identity,
            "Prepared successor must not become current before authenticated Ready"
        );
        let attempt = crate::cli::execution_state::prepared_fresh_linked_owner_launch_for_session(
            &project,
            owner,
            &successor.id,
        )
        .expect("read fresh Prepared attempt")
        .expect("fresh Prepared attempt");
        assert_eq!(
            attempt.status,
            crate::cli::execution_state::ContinuationAttemptStatus::Prepared
        );
        assert!(env.contains_key(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV));
        let token = env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("fresh Prepared capability token");
        let grant = issuer
            .grant_for_test(token)
            .expect("authenticate fresh Prepared capability");
        assert_eq!(
            grant.principal().execution_authority_kind(),
            crate::agent_capability::AgentExecutionAuthorityKind::Prepared
        );
    }

    #[test]
    fn explicit_linked_owner_launch_prepares_fresh_lifetime_from_blocked_predecessor() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut predecessor = persisted_execution_launch(home.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &predecessor.sessions_dir,
            session: &mut predecessor.session,
            project_root: &predecessor.project,
            worktree: &predecessor.project,
            producing_owner: Some(predecessor.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut HashMap::new())
        .expect("materialize predecessor generation");
        assert!(matches!(
            crate::cli::execution_state::settle(
                &predecessor.project,
                &predecessor.session.id,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "legacy blocker without same-lifetime recovery contract".to_string(),
                    missing_verification: Some("legacy blocker".to_string()),
                },
            )
            .expect("settle predecessor as Blocked"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let predecessor_binding = crate::cli::execution_state::current_execution_binding(
            &predecessor.project,
            predecessor.owner,
        )
        .expect("read blocked predecessor binding")
        .expect("blocked predecessor generation");

        let holder_path = predecessor
            .sessions_dir
            .join(format!("{}.toml", predecessor.session.id));
        let mut holder = gwt_agent::Session::load(&holder_path).expect("reload Blocked holder");
        holder.update_status(gwt_agent::AgentStatus::Stopped);
        holder
            .save(&predecessor.sessions_dir)
            .expect("persist stopped Blocked holder");

        let mut successor = gwt_agent::Session::new(
            &predecessor.project,
            "work/issue-2359",
            gwt_agent::AgentId::Codex,
        );
        successor.project_state_root = Some(predecessor.project.clone());
        successor.linked_issue_number = Some(predecessor.owner.number);
        successor.launch_route = gwt_agent::LaunchRoute::Autonomous;
        successor.update_status(gwt_agent::AgentStatus::Running);
        let runtime_path = gwt_agent::runtime_state_path(&predecessor.sessions_dir, &successor.id);
        persist_finalized_launch_session(
            &predecessor.sessions_dir,
            Some(&runtime_path),
            &mut successor,
            None,
        )
        .expect("persist fresh launch Session before authority");

        let mut env = HashMap::new();
        FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &predecessor.sessions_dir,
            session: &mut successor,
            project_root: &predecessor.project,
            worktree: &predecessor.project,
            producing_owner: Some(predecessor.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect("explicit fresh launch must prepare a new lifetime from legacy Blocked");

        assert_eq!(
            crate::cli::execution_state::current_execution_binding(
                &predecessor.project,
                predecessor.owner,
            )
            .expect("read current generation"),
            Some(predecessor_binding),
            "pre-readiness preparation must preserve the terminal predecessor as current",
        );
        let persisted = gwt_agent::Session::load(
            &predecessor
                .sessions_dir
                .join(format!("{}.toml", successor.id)),
        )
        .expect("reload fresh prepared Session");
        let binding = persisted
            .execution_binding
            .expect("fresh launch must persist its prepared generation binding");
        let token = env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("fresh launch Prepared capability token");
        let grant = issuer
            .grant_for_test(token)
            .expect("authenticate fresh Prepared capability");
        assert_eq!(
            grant.principal().execution_authority_kind(),
            crate::agent_capability::AgentExecutionAuthorityKind::Prepared,
        );
        assert!(!grant.principal().authorizes_producing_mutation());
        assert_eq!(grant.principal().execution_binding(), Some(&binding));
    }

    #[test]
    fn failed_bound_capability_install_rolls_back_the_persisted_session_binding() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "not-a-hook-url",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let runtime = crate::docker_launch::resolved_test_docker_runtime(home.path());
        let mut env = HashMap::new();
        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
            container_runtime: Some(&runtime),
            permission_decision: None,
        }
        .install(&mut env)
        .expect_err("invalid Docker hook endpoint must fail");

        assert!(error.contains("invalid host hook forward URL"));
        let persisted = gwt_agent::Session::load(
            &launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id)),
        )
        .expect("reload failed launch Session");
        assert!(
            persisted.execution_binding.is_none(),
            "failed capability installation must not leave producing authority"
        );
        assert!(
            !env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "failed capability installation must not expose a bearer"
        );
        assert!(
            crate::cli::execution_state::load(&launch.project)
                .expect("read failed launch ECR")
                .is_none(),
            "known endpoint failure must be rejected before flat ECR materialization"
        );
        assert!(
            crate::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read failed launch generation")
                .is_none(),
            "known endpoint failure must not leave a ghost generation"
        );
    }

    #[test]
    fn producing_launch_without_host_issuer_refuses_before_execution_materialization() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let mut env = HashMap::new();
        let error = FinalizedAgentCapabilityLaunch {
            issuer: None,
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect_err("producing launch requires its Host capability issuer");

        assert!(error.contains("Host capability issuer"));
        assert!(crate::cli::execution_state::load(&launch.project)
            .expect("read refused launch ECR")
            .is_none());
        assert!(crate::cli::execution_state::current_execution_binding(
            &launch.project,
            launch.owner
        )
        .expect("read refused launch generation")
        .is_none());
        let persisted = gwt_agent::Session::load(
            &launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id)),
        )
        .expect("reload refused launch Session");
        assert!(persisted.execution_binding.is_none());
    }

    #[test]
    fn producing_launch_preflights_a_closing_issuer_before_execution_materialization() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let inspection = issuer
            .issue(&launch.project, &launch.session.id)
            .expect("issue pre-existing inspection capability");
        let grant = issuer
            .grant_for_test(&inspection.token)
            .expect("authenticate pre-existing capability");
        let close_ticket = issuer
            .begin_self_close_if_current(&grant)
            .expect("hold issuer in closing state");
        let mut env = HashMap::new();
        let error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env)
        .expect_err("closing Host issuer must refuse producing launch");

        assert!(issuer.rollback_self_close(&close_ticket));
        assert!(issuer.revoke_token(&inspection.token));
        assert!(error.contains("closing"));
        assert!(
            crate::cli::execution_state::load(&launch.project)
                .expect("read closing launch ECR")
                .is_none(),
            "issuer availability must be rejected before flat ECR materialization"
        );
        assert!(
            crate::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read closing launch generation")
                .is_none(),
            "issuer availability must be rejected before generation materialization"
        );
    }

    #[test]
    fn genesis_session_binding_io_failure_fails_closed_without_unverified_terminalization() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let session_path = launch
            .sessions_dir
            .join(format!("{}.toml", launch.session.id));
        let saved_session_path = home.path().join("session-before-io-failure.toml");
        let session_before = std::fs::read(&session_path).expect("read Session before failure");
        std::fs::rename(&session_path, &saved_session_path)
            .expect("move Session file before injected failure");
        std::fs::create_dir(&session_path).expect("block atomic Session replacement");
        let mut env = HashMap::new();
        let result = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut env);
        std::fs::remove_dir(&session_path).expect("remove injected Session path blocker");
        std::fs::rename(&saved_session_path, &session_path)
            .expect("restore Session file after injected failure");
        let error = result.expect_err("Session binding persistence must fail");

        assert!(error.contains("Session binding"), "{error}");
        assert!(error.contains("terminal"), "{error}");
        let failed_ledger =
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("read failed genesis ledger")
                .expect("failed genesis ledger remains as audit evidence");
        assert_eq!(
            failed_ledger.current_effective_status(),
            Some(crate::cli::execution_state::ExecutionControlStatus::Active),
            "unreadable Session identity must retain authority for exact reconciliation"
        );
        let persisted = gwt_agent::Session::load(
            &launch
                .sessions_dir
                .join(format!("{}.toml", launch.session.id)),
        )
        .expect("reload pre-binding Session");
        assert!(persisted.execution_binding.is_none());
        assert_eq!(
            std::fs::read(&session_path).expect("read restored Session"),
            session_before,
            "the failed write must retain the pre-binding Session byte-identically",
        );
        assert!(durable_launch_recovery_exists(
            &launch.sessions_dir,
            &launch.session.id,
        ));
        assert!(
            gwt_core::workspace_projection::load_workspace_work_items(&launch.project)
                .expect("read absent Work projection")
                .is_none(),
            "pre-readiness failure must not publish Work",
        );
        assert!(
            !env.contains_key(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            "binding persistence failure must precede bearer issuance"
        );

        let ledger_before_retry = failed_ledger;
        let receipt_path = launch
            .sessions_dir
            .join("execution-launch-recovery")
            .join(format!("{}.json", launch.session.id));
        let receipt_before_retry =
            std::fs::read(&receipt_path).expect("read exact recovery receipt");
        let retry_error = FinalizedAgentCapabilityLaunch {
            issuer: Some(&issuer),
            sessions_dir: &launch.sessions_dir,
            session: &mut launch.session,
            project_root: &launch.project,
            worktree: &launch.project,
            producing_owner: Some(launch.owner),
            prepared_continuation: None,
            rebound_continuation: None,
            execution_entrypoint: "$gwt-execute #2359",
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            container_runtime: None,
            permission_decision: None,
        }
        .install(&mut HashMap::new())
        .expect_err("unreconciled Active authority must reject a blind retry");
        assert!(retry_error.contains("Continue work"), "{retry_error}");
        assert_eq!(
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("read authority after retry")
                .expect("authority after retry"),
            ledger_before_retry,
        );
        assert_eq!(
            std::fs::read(&session_path).expect("read Session after retry"),
            session_before,
        );
        assert_eq!(
            std::fs::read(&receipt_path).expect("read receipt after retry"),
            receipt_before_retry,
        );
    }

    #[test]
    fn genesis_session_persistence_unknown_outcome_cleans_exact_durable_binding() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());

        persist_durable_launch_recovery(
            &launch.sessions_dir,
            DurableLaunchRecoveryKind::Genesis,
            &launch.session.id,
            &launch.project,
            &launch.project,
            launch.owner,
            None,
            None,
        )
        .expect("persist genesis recovery receipt");
        crate::cli::execution_state::materialize_at_launch(
            &launch.project,
            launch.owner.kind,
            launch.owner.number,
            &launch.session.id,
            "$gwt-execute #2359",
            false,
        )
        .expect("materialize genesis authority");
        crate::cli::execution_state::ensure_generation_ledger(
            &launch.project,
            launch.owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize genesis generation ledger");
        let identity =
            crate::cli::execution_state::current_execution_binding(&launch.project, launch.owner)
                .expect("read genesis authority")
                .expect("genesis binding");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: launch.session.id.clone(),
            repo_hash: launch
                .session
                .repo_hash
                .clone()
                .expect("fixture repository hash"),
            owner_kind: launch.owner.kind.as_str().to_string(),
            owner_number: launch.owner.number,
            identity,
            capability_generation: 1,
        };
        launch
            .session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind genesis Session");
        let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&launch.session)
            .unwrap()
            .unwrap();
        let genesis = MaterializedGenesisLaunch {
            worktree_path: launch.project.clone(),
            owner: launch.owner,
            session_id: launch.session.id.clone(),
            binding,
            session_identity,
        };
        let sessions_dir = launch.sessions_dir.clone();
        persist_durable_launch_recovery(
            &sessions_dir,
            DurableLaunchRecoveryKind::Genesis,
            &genesis.session_id,
            &genesis.worktree_path,
            &genesis.worktree_path,
            genesis.owner,
            Some(&genesis.binding),
            Some(&genesis.session_identity.agent_id),
        )
        .expect("bind genesis recovery receipt to the exact Session");

        let error = persist_genesis_session_binding_with(
            &sessions_dir,
            &mut launch.session,
            &genesis,
            |session| {
                session.save(&sessions_dir)?;
                Err::<bool, _>(std::io::Error::other(
                    "simulated response loss after durable Session replacement",
                ))
            },
        )
        .expect_err("unknown persistence outcome must fail the launch");

        assert!(error.contains("simulated response loss"), "{error}");
        assert_eq!(
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner,)
                .expect("read terminalized genesis")
                .expect("genesis ledger")
                .current_effective_status(),
            Some(crate::cli::execution_state::ExecutionControlStatus::Blocked),
        );
        assert!(
            !sessions_dir
                .join(format!("{}.toml", genesis.session_id))
                .exists(),
            "an exact Session that may have committed before Err must be removed"
        );
        assert!(
            !crate::session_launch::durable_launch_recovery_exists(
                &sessions_dir,
                &genesis.session_id,
            ),
            "the receipt may clear only after exact Session cleanup"
        );
    }

    #[test]
    fn genesis_terminalization_rejects_same_binding_agent_replacement_without_authority_mutation() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut launch = persisted_execution_launch(home.path());
        crate::cli::execution_state::materialize_at_launch(
            &launch.project,
            launch.owner.kind,
            launch.owner.number,
            &launch.session.id,
            "$gwt-execute #2359",
            false,
        )
        .expect("materialize genesis authority");
        crate::cli::execution_state::ensure_generation_ledger(
            &launch.project,
            launch.owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize genesis ledger");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: launch.session.id.clone(),
            repo_hash: launch
                .session
                .repo_hash
                .clone()
                .expect("fixture repository hash"),
            owner_kind: launch.owner.kind.as_str().to_string(),
            owner_number: launch.owner.number,
            identity: crate::cli::execution_state::current_execution_binding(
                &launch.project,
                launch.owner,
            )
            .expect("read genesis authority")
            .expect("genesis binding"),
            capability_generation: 1,
        };
        launch
            .session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind genesis Session");
        launch
            .session
            .save(&launch.sessions_dir)
            .expect("save exact genesis Session");
        let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&launch.session)
            .unwrap()
            .unwrap();
        let genesis = MaterializedGenesisLaunch {
            worktree_path: launch.project.clone(),
            owner: launch.owner,
            session_id: launch.session.id.clone(),
            binding,
            session_identity,
        };
        let session_path = launch
            .sessions_dir
            .join(format!("{}.toml", genesis.session_id));
        let mut replacement = launch.session.clone();
        replacement.agent_id = gwt_agent::AgentId::Custom("review-bot".to_string());
        replacement
            .save(&launch.sessions_dir)
            .expect("replace genesis Session Agent");
        let replacement_before = std::fs::read(&session_path).expect("read replacement Session");
        let ledger_before =
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("read authority before")
                .expect("authority before");

        let error = terminalize_materialized_genesis_launch(
            &launch.sessions_dir,
            Some(&genesis),
            "simulated launch rollback",
        )
        .expect_err("replacement Agent must fail closed");

        assert!(error.contains("identity changed"), "{error}");
        assert_eq!(
            std::fs::read(&session_path).expect("read retained replacement Session"),
            replacement_before,
        );
        assert_eq!(
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("read authority after")
                .expect("authority after"),
            ledger_before,
        );

        launch
            .session
            .save(&launch.sessions_dir)
            .expect("restore exact genesis Session");
        terminalize_materialized_genesis_launch(
            &launch.sessions_dir,
            Some(&genesis),
            "simulated exact launch rollback",
        )
        .expect("exact genesis cleanup");
        assert!(!session_path.exists());
        assert_eq!(
            crate::cli::execution_state::load_generation_ledger(&launch.project, launch.owner)
                .expect("read terminal authority")
                .expect("terminal authority")
                .current_effective_status(),
            Some(crate::cli::execution_state::ExecutionControlStatus::Blocked),
        );
    }

    #[test]
    fn launch_injects_the_runtime_specific_pane_websocket_endpoint() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();

        install_agent_capability_env(
            &mut env,
            Some(&issuer),
            project.path(),
            "session-pane-env",
            gwt_agent::LaunchRuntimeTarget::Host,
            None,
        )
        .expect("install agent launch endpoints");

        assert_eq!(
            env.get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://127.0.0.1:45123/internal/hook-live")
        );
        assert_eq!(
            env.get(gwt_agent::GWT_PANE_WS_URL_ENV).map(String::as_str),
            Some("ws://127.0.0.1:45123/internal/pane-ws")
        );

        let mut docker_env = HashMap::new();
        let docker_runtime = crate::docker_launch::resolved_test_docker_runtime(project.path());
        install_agent_capability_env(
            &mut docker_env,
            Some(&issuer),
            project.path(),
            "session-pane-env-docker",
            gwt_agent::LaunchRuntimeTarget::Docker,
            Some(&docker_runtime),
        )
        .expect("install Docker agent launch endpoints");

        assert_eq!(
            docker_env
                .get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://host.docker.internal:45123/internal/hook-live")
        );
        assert_eq!(
            docker_env
                .get(gwt_agent::GWT_PANE_WS_URL_ENV)
                .map(String::as_str),
            Some("ws://host.docker.internal:45123/internal/pane-ws")
        );
    }

    #[cfg(unix)]
    #[test]
    fn launch_contract_pins_a_stateful_wrapper_across_override_and_endpoint_consumers() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let wrapper = temp.path().join("stateful-container-wrapper");
        std::fs::write(
            &wrapper,
            r#"#!/bin/sh
counter="$0.count"
count=0
if [ -f "$counter" ]; then
  read count < "$counter"
fi
count=$((count + 1))
printf '%s\n' "$count" > "$counter"
if [ "$count" -eq 1 ]; then
  printf 'Docker version 28.3.0, build test\n'
else
  printf 'podman version 5.4.2\n'
fi
"#,
        )
        .expect("write stateful wrapper");
        let mut permissions = std::fs::metadata(&wrapper)
            .expect("wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).expect("chmod stateful wrapper");

        // test-hygiene: allow-production-probe-deadline Integration test observes real CLI invocation counts and pinned binary reuse.
        let runtime = gwt_docker::detect::ResolvedContainerRuntime::resolve(
            wrapper.to_str().expect("UTF-8 wrapper path"),
        )
        .expect("resolve launch runtime once");
        let bundle = crate::docker_launch::docker_bundle_mounts_for_home(temp.path());
        let override_content = crate::docker_launch::docker_bundle_override_content_for_runtime(
            "app",
            &bundle,
            runtime.kind(),
        );
        let issuer = AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45123/internal/hook-live",
            "ws://127.0.0.1:46234/ws",
            "ws://127.0.0.1:45123/internal/pane-ws",
        );
        let mut env = HashMap::new();
        install_agent_capability_env(
            &mut env,
            Some(&issuer),
            temp.path(),
            "session-stateful-runtime",
            gwt_agent::LaunchRuntimeTarget::Docker,
            Some(&runtime),
        )
        .expect("install pinned Docker endpoints");

        assert!(override_content.contains(gwt_docker::DOCKER_HOST_GATEWAY_EXTRA_HOST));
        assert_eq!(
            env.get(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://host.docker.internal:45123/internal/hook-live")
        );
        assert_eq!(
            env.get(gwt_agent::GWT_PANE_WS_URL_ENV).map(String::as_str),
            Some("ws://host.docker.internal:45123/internal/pane-ws")
        );
        assert_eq!(
            std::fs::read_to_string(wrapper.with_extension("count"))
                .expect("read wrapper probe count")
                .trim(),
            "1",
            "one launch must probe a configured runtime wrapper exactly once"
        );
    }
}

mod owner_checkpoint_contract_tests {
    use crate::project_runtime::{events::OwnerCommand, test_support::ProjectOwnerHarness};
    use std::sync::atomic::Ordering;

    #[test]
    fn launch_complete_dropped_prepared_handoff_defers_exact_cleanup() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("executor");
        executor.block_on(async {
            for args in [
                vec!["init", "-q", "-b", "main"],
                vec![
                    "-c",
                    "user.name=Owner fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    "prepared owner fixture",
                ],
            ] {
                let output = gwt_core::process::hidden_command("git")
                    .args(args)
                    .current_dir(temp.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let owner = ProjectOwnerHarness::start(temp.path())
                .await
                .expect("production owner");
            owner
                .set_pty_start_gate(
                    std::env::current_exe().expect("fixture executable"),
                    vec![
                        "project_runtime::test_support::pty_start_gate_helper".into(),
                        "--exact".into(),
                        "--nocapture".into(),
                    ],
                )
                .await
                .expect("actual fixture gate");
            let checkpoint = owner
                .pause_next_agent_launch()
                .await
                .expect("capture next prepared handoff");
            let id = owner.pane_id("lost-handoff");
            let operation_id = "lost-handoff-operation".to_string();
            let mut config =
                gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom("fixture".into()))
                    .working_dir(temp.path())
                    .build();
            if cfg!(windows) {
                config.command = "cmd".into();
                config.args = vec!["/d".into(), "/s".into(), "/c".into(), "exit /b 0".into()];
            } else {
                config.command = "/bin/sh".into();
                config.args = vec!["-c".into(), "exit 0".into()];
            }
            let client = owner.client();
            let launch_id = id.clone();
            let launch_operation = operation_id.clone();
            let project_key = gwt_core::paths::project_scope_hash(temp.path())
                .as_str()
                .to_string();
            let launch = tokio::spawn(async move {
                client
                    .command(OwnerCommand::LaunchAgent {
                        id: launch_id,
                        operation_id: launch_operation,
                        tab_id: project_key,
                        config: Box::new(config),
                        metadata: Box::default(),
                        cols: 80,
                        rows: 24,
                    })
                    .await
            });
            let evidence = checkpoint.wait().await;
            assert_eq!(evidence.id, id);
            assert_eq!(evidence.operation_id, operation_id);
            let session_id = evidence
                .session_id
                .as_ref()
                .expect("actual prepared Session");
            let inventory = owner
                .command(OwnerCommand::ListPanes)
                .await
                .expect("owner inventory");
            assert!(inventory["panes"]
                .as_array()
                .expect("installed panes")
                .is_empty());
            assert_eq!(
                inventory["pending"]
                    .as_array()
                    .expect("pending handoff")
                    .len(),
                1,
                "uninstalled handoff keeps exactly one owner cleanup obligation"
            );
            let client = owner.client();
            let cancel = tokio::spawn(async move {
                client
                    .command(OwnerCommand::PendingCancel { id, operation_id })
                    .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                while evidence.current.load(Ordering::Acquire) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("canonical owner accepted exact cancellation");
            checkpoint.release();
            assert!(launch.await.expect("launch worker").is_err());
            cancel
                .await
                .expect("cancel worker")
                .expect("exact cleanup ACK");
            let session = gwt_agent::Session::load(
                &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
            )
            .expect("settled uninstalled Session");
            assert_eq!(session.status, gwt_agent::AgentStatus::Stopped);
            assert!(!session.restore_window_on_startup);
            let inventory = owner
                .command(OwnerCommand::ListPanes)
                .await
                .expect("settled owner inventory");
            assert!(inventory["panes"].as_array().unwrap().is_empty());
            assert!(inventory["pending"].as_array().unwrap().is_empty());
            owner.shutdown().await;
        });
    }
}
