//! Exact-runtime duplicate recovery for Issue Monitor (Issue #4466).

use super::*;

/// Exact durable Session and runtime observations, including unbound Sessions.
/// All process identity components must be present and match under the leases.
#[derive(Debug, Clone)]
pub struct MonitorDuplicateRuntimeProof {
    pub session: gwt_agent::Session,
    pub host_pid: u32,
    pub runtime: gwt_agent::SessionRuntimeState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorDuplicateStopOutcome {
    Stopped,
    Refused { reason: String },
}

/// Unknown evidence blocks its own Issue, or every Issue when its owner
/// cannot be established. Known unrelated legacy Sessions are not a fence.
pub fn monitor_runtime_uncertainty_affects_issue(
    uncertainty: &crate::session_inventory::SessionObservationUncertainty,
    sessions_dir: &Path,
    issue_number: u64,
) -> bool {
    let Some(session_id) = uncertainty.session_id.as_deref() else {
        return true;
    };
    if gwt_agent::validate_session_id_path_component(session_id).is_err() {
        return true;
    }
    gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
        .ok()
        .and_then(|session| session.linked_issue_number)
        .is_none_or(|owner| owner == issue_number)
}

/// Stop only the nonholder of an exact live pair. The callback stops a captured
/// PTY handle and returns true only after reaping that child. This never writes
/// the owner ledger or ECR. An unsuccessful exit proof publishes no terminal state.
pub fn stop_monitor_duplicate(
    project_root: &Path,
    issue_number: u64,
    sessions_dir: &Path,
    expected_pair: &[MonitorDuplicateRuntimeProof],
    target_session_id: &str,
    stop: impl FnOnce() -> io::Result<bool>,
) -> io::Result<MonitorDuplicateStopOutcome> {
    let [first, second] = expected_pair else {
        return refused("exactly two observed runtimes are required");
    };
    if first.session.id == second.session.id {
        return refused("two runtimes share one Session; ownership is ambiguous");
    }
    let Some(target_index) = expected_pair
        .iter()
        .position(|proof| proof.session.id == target_session_id)
    else {
        return refused("the stop target is not in the observed pair");
    };
    if expected_pair[target_index].host_pid != std::process::id() {
        return refused("the stop target is not a local captured runtime");
    }
    let Some(binding) = expected_pair
        .iter()
        .find_map(|proof| proof.session.execution_binding.as_ref())
    else {
        return refused("neither Session has execution ownership evidence");
    };
    let owner = ExecutionOwnerKey {
        kind: match binding.owner_kind.as_str() {
            "issue" => ExecutionOwnerKind::Issue,
            "spec" => ExecutionOwnerKind::Spec,
            _ => return refused("execution owner kind is unknown"),
        },
        number: issue_number,
    };
    with_generation_owner_lease(project_root, owner, |context| {
        let Some(ledger) = load_owner_generation_ledger_from_context(context)? else {
            return refused("the owner generation ledger is missing");
        };
        let Some(current) = ledger.current_generation() else {
            return refused("the current owner generation is missing");
        };
        if ledger.effective_status_for(current) != ExecutionControlStatus::Active {
            return refused("the current owner generation is not Active");
        }
        if !latest_prepared_successors_for_generation(&ledger, &current.identity.generation_id)
            .is_empty()
            || !latest_prepared_takeovers_for_generation(&ledger, &current.identity.generation_id)
                .is_empty()
        {
            return refused("a prepared ownership transition makes the runtime pair ambiguous");
        }
        gwt_agent::session::with_session_pair_lease(
            sessions_dir,
            [&first.session.id, &second.session.id],
            |sessions| {
                let mut runtimes = Vec::with_capacity(2);
                for (proof, session) in expected_pair.iter().zip(&sessions) {
                    if !same_session_authority(&proof.session, session)
                        || session.linked_issue_number != Some(issue_number)
                    {
                        return refused("the observed Session identity changed");
                    }
                    let session_context =
                        GenerationTransactionContext::resolve(&session.worktree_path, owner)?;
                    if session_context.owner_dir != context.owner_dir {
                        return refused("an observed Session belongs to another repository");
                    }
                    let runtime = gwt_agent::SessionRuntimeState::load(
                        &gwt_agent::runtime_state_path_for_pid(
                            sessions_dir,
                            proof.host_pid,
                            &session.id,
                        ),
                    )?;
                    if !same_runtime_identity(&runtime, &proof.runtime)
                        || runtime.runtime_incarnation.is_none_or(|value| value == 0)
                        || runtime.host_started_at.is_none_or(|value| value == 0)
                        || runtime.child_pid.is_none_or(|value| value == 0)
                        || runtime.child_started_at.is_none_or(|value| value == 0)
                        || crate::process::host_process_start_time(proof.host_pid)
                            != runtime.host_started_at
                        || runtime.execution_identity
                            != gwt_agent::SessionExecutionIdentity::from_session(session)
                                .map_err(io::Error::other)?
                    {
                        return refused("the exact runtime identity is missing or changed");
                    }
                    runtimes.push(runtime);
                }
                // Reobserve under owner and Session leases: a stale two-row input
                // cannot hide a third runtime, PID reuse, or an unreadable sidecar.
                let observed =
                    crate::session_inventory::observe_sessions(project_root, sessions_dir);
                if observed.uncertainties.iter().any(|uncertainty| {
                    monitor_runtime_uncertainty_affects_issue(
                        uncertainty,
                        sessions_dir,
                        issue_number,
                    )
                }) {
                    return refused("runtime inventory contains uncertain evidence");
                }
                let matching = observed
                    .sessions
                    .iter()
                    .filter(|row| row.issue_number == Some(issue_number))
                    .collect::<Vec<_>>();
                if matching.len() != 2
                    || expected_pair.iter().any(|proof| {
                        !matching.iter().any(|row| {
                            row.session_id == proof.session.id
                                && row.host_pid == proof.host_pid
                                && Some(row.child_pid) == proof.runtime.child_pid
                                && Some(row.child_started_at) == proof.runtime.child_started_at
                        })
                    })
                {
                    return refused("the live runtime inventory no longer matches the exact pair");
                }
                let mut holders = Vec::new();
                for (index, session) in sessions.iter().enumerate() {
                    let mut holder = false;
                    for generation in &ledger.generations {
                        if ledger.effective_status_for(generation) != ExecutionControlStatus::Active
                        {
                            continue;
                        }
                        let projection: ExecutionControlRecord =
                            serde_json::from_str(ledger.effective_projection_for(generation))
                                .map_err(io::Error::other)?;
                        if projection.primary_session_id != session.id {
                            continue;
                        }
                        let Some(binding) = session.execution_binding.as_ref() else {
                            return refused("an Active holder lacks its durable execution binding");
                        };
                        if binding.owner_number != issue_number
                            || binding.owner_kind != ledger.owner.kind.as_str()
                            || generation.identity.worktree_binding_hash
                                != worktree_binding_hash(&session.worktree_path)
                            || !execution_binding_authorizes_lifecycle_descendant(
                                &ledger,
                                generation,
                                &session.id,
                                &binding.identity,
                            )
                        {
                            return refused("Active holder ownership evidence is inconsistent");
                        }
                        holder = true;
                    }
                    if let Some(binding) = session.execution_binding.as_ref() {
                        if binding.owner_number != issue_number
                            || binding.owner_kind != ledger.owner.kind.as_str()
                        {
                            return refused("a Session has authority for another owner");
                        }
                        if !ledger.generations.iter().any(|generation| {
                            execution_binding_matches_historical_prefix(
                                &ledger,
                                generation,
                                &session.id,
                                &binding.identity,
                            )
                        }) {
                            return refused("a Session binding has no authentic ownership history");
                        }
                    }
                    if holder {
                        holders.push(index);
                    }
                }
                if holders.len() != 1 || holders[0] == target_index {
                    return refused(
                        "recovery requires one live holder and one nonholding stop target",
                    );
                }
                let holder = &sessions[holders[0]];
                let holder_context =
                    GenerationTransactionContext::resolve(&holder.worktree_path, owner)?;
                if !current_active_execution_binding_matches_context(
                    &holder_context,
                    &holder.id,
                    &holder
                        .execution_binding
                        .as_ref()
                        .expect("holder binding checked above")
                        .identity,
                )? {
                    return refused("the live holder no longer owns the current Active ECR");
                }
                // The inventory and leases prove both children are still exact.
                // Stopping uses the caller's captured handle, never a PID signal.
                if !stop()? {
                    return refused("the stop callback did not prove child exit");
                }
                let target = &expected_pair[target_index];
                let runtime = &runtimes[target_index];
                if crate::process::exact_pty_process_tree_is_alive(
                    runtime.child_pid.unwrap(),
                    runtime.child_started_at.unwrap(),
                ) {
                    return refused("the exact child process tree is still live after stop");
                }
                if !gwt_agent::session::persist_observed_session_runtime_stopped_under_lease(
                    sessions_dir,
                    &sessions[target_index],
                    target.host_pid,
                    runtime,
                )? {
                    return refused(
                        "the exact stopped runtime changed before terminal persistence",
                    );
                }
                Ok(MonitorDuplicateStopOutcome::Stopped)
            },
        )
    })
}

fn refused(reason: &str) -> io::Result<MonitorDuplicateStopOutcome> {
    Ok(MonitorDuplicateStopOutcome::Refused {
        reason: reason.into(),
    })
}

fn same_session_authority(left: &gwt_agent::Session, right: &gwt_agent::Session) -> bool {
    left.id == right.id
        && left.created_at == right.created_at
        && left.worktree_path == right.worktree_path
        && left.project_state_root == right.project_state_root
        && left.repo_hash == right.repo_hash
        && left.branch == right.branch
        && left.agent_id == right.agent_id
        && left.linked_issue_number == right.linked_issue_number
        && left.execution_binding == right.execution_binding
}

fn same_runtime_identity(
    left: &gwt_agent::SessionRuntimeState,
    right: &gwt_agent::SessionRuntimeState,
) -> bool {
    left.execution_identity == right.execution_identity
        && left.runtime_incarnation == right.runtime_incarnation
        && left.host_started_at == right.host_started_at
        && left.child_pid == right.child_pid
        && left.child_started_at == right.child_started_at
}

#[cfg(all(test, unix))]
pub(crate) fn seed_monitor_pair_for_test(
    project: &Path,
    sessions: &Path,
) -> [gwt_agent::Session; 2] {
    let owner = ExecutionOwnerKey {
        kind: ExecutionOwnerKind::Issue,
        number: 4466,
    };
    seed_monitor_pair_with_owner(project, sessions, owner)
}

#[cfg(all(test, unix))]
fn seed_monitor_pair_with_owner(
    project: &Path,
    sessions: &Path,
    owner: ExecutionOwnerKey,
) -> [gwt_agent::Session; 2] {
    assert_eq!(sessions, gwt_core::paths::gwt_sessions_dir());
    let mut record = super::tests::active_record("holder");
    record.owner_kind = owner.kind;
    record.owner_number = owner.number;
    save(project, &record).unwrap();
    ensure_generation_ledger(project, owner, LegacyActiveDisposition::Live).unwrap();
    let binding = current_execution_binding(project, owner).unwrap().unwrap();
    super::tests::persist_generation_session_binding(project, owner, "holder", binding);
    let holder = gwt_agent::Session::load(&sessions.join("holder.toml")).unwrap();
    let mut duplicate = holder.clone();
    duplicate.id = "nonholder".into();
    duplicate.execution_binding = None;
    duplicate.restore_window_on_startup = true;
    duplicate.save(sessions).unwrap();
    [holder, duplicate]
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;
    use std::{cell::Cell, process::Child};

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn fixture(
        test: impl FnOnce(&Path, &Path, &mut [ChildGuard; 2], &[MonitorDuplicateRuntimeProof; 2]),
    ) {
        fixture_for_kind(ExecutionOwnerKind::Issue, test);
    }

    fn fixture_for_kind(
        kind: ExecutionOwnerKind,
        test: impl FnOnce(&Path, &Path, &mut [ChildGuard; 2], &[MonitorDuplicateRuntimeProof; 2]),
    ) {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = super::super::tests::unset_live_session_env();
        let project = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(project.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let pair = seed_monitor_pair_with_owner(
            project.path(),
            &sessions,
            ExecutionOwnerKey { kind, number: 4466 },
        );
        let mut children = [
            ChildGuard(
                gwt_core::process::hidden_command("sleep")
                    .arg("60")
                    .spawn()
                    .unwrap(),
            ),
            ChildGuard(
                gwt_core::process::hidden_command("sleep")
                    .arg("60")
                    .spawn()
                    .unwrap(),
            ),
        ];
        let proofs = pair.map(|session| {
            let index = usize::from(session.id != "holder");
            let mut runtime = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
            runtime.execution_identity =
                gwt_agent::SessionExecutionIdentity::from_session(&session).unwrap();
            runtime.runtime_incarnation = Some(index as u64 + 1);
            runtime.host_started_at = crate::process::host_process_start_time(std::process::id());
            runtime.child_pid = Some(children[index].0.id());
            runtime.child_started_at =
                crate::process::host_process_start_time(children[index].0.id());
            assert!(runtime.host_started_at.is_some() && runtime.child_started_at.is_some());
            runtime
                .save(&gwt_agent::runtime_state_path_for_pid(
                    &sessions,
                    std::process::id(),
                    &session.id,
                ))
                .unwrap();
            MonitorDuplicateRuntimeProof {
                session,
                host_pid: std::process::id(),
                runtime,
            }
        });
        test(project.path(), &sessions, &mut children, &proofs);
    }

    #[test]
    fn monitor_duplicate_stops_only_nonholder_and_preserves_holder_authority() {
        fixture(|project, sessions, children, proofs| {
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Issue,
                number: 4466,
            };
            let owner_dir = generation_owner_dir(project, owner).unwrap();
            let ledger_before = read_owner_ledger_from_dir(&owner_dir).unwrap();
            let holder_before = fs::read(sessions.join("holder.toml")).unwrap();
            let result =
                stop_monitor_duplicate(project, 4466, sessions, proofs, "nonholder", || {
                    children[1].0.kill()?;
                    children[1].0.wait()?;
                    Ok(true)
                })
                .unwrap();
            assert_eq!(result, MonitorDuplicateStopOutcome::Stopped);
            assert!(children[0].0.try_wait().unwrap().is_none());
            assert_eq!(
                read_owner_ledger_from_dir(&owner_dir).unwrap(),
                ledger_before
            );
            assert_eq!(
                fs::read(sessions.join("holder.toml")).unwrap(),
                holder_before
            );
            let stopped = gwt_agent::Session::load(&sessions.join("nonholder.toml")).unwrap();
            assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
            assert!(!stopped.restore_window_on_startup);
            let runtime = gwt_agent::SessionRuntimeState::load(
                &gwt_agent::runtime_state_path_for_pid(sessions, std::process::id(), "nonholder"),
            )
            .unwrap();
            assert_eq!(runtime.status, gwt_agent::AgentStatus::Stopped);
        });
    }

    #[test]
    fn monitor_duplicate_ambiguous_or_changed_proof_never_stops_a_process() {
        fixture(|project, sessions, _, proofs| {
            let calls = Cell::new(0);
            let stop = || {
                calls.set(calls.get() + 1);
                Ok(true)
            };
            for pair in [
                vec![proofs[0].clone()],
                vec![proofs[0].clone(), proofs[0].clone()],
                vec![proofs[0].clone(), proofs[1].clone(), proofs[1].clone()],
            ] {
                assert!(matches!(
                    stop_monitor_duplicate(project, 4466, sessions, &pair, "nonholder", stop)
                        .unwrap(),
                    MonitorDuplicateStopOutcome::Refused { .. }
                ));
            }
            let mut unbound = proofs.clone();
            unbound[0].session.execution_binding = None;
            assert!(matches!(
                stop_monitor_duplicate(project, 4466, sessions, &unbound, "nonholder", stop)
                    .unwrap(),
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
            assert!(matches!(
                stop_monitor_duplicate(project, 4466, sessions, proofs, "holder", stop).unwrap(),
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
            let mut changed = proofs[1].runtime.clone();
            changed.runtime_incarnation = Some(99);
            changed
                .save(&gwt_agent::runtime_state_path_for_pid(
                    sessions,
                    proofs[1].host_pid,
                    "nonholder",
                ))
                .unwrap();
            assert!(matches!(
                stop_monitor_duplicate(project, 4466, sessions, proofs, "nonholder", stop).unwrap(),
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
            assert_eq!(calls.get(), 0);
        });
    }

    #[test]
    fn monitor_duplicate_spec_owner_uses_the_same_exact_stop_guard() {
        fixture_for_kind(ExecutionOwnerKind::Spec, |project, sessions, _, proofs| {
            let called = Cell::new(false);
            let result =
                stop_monitor_duplicate(project, 4466, sessions, proofs, "nonholder", || {
                    called.set(true);
                    Ok(false)
                })
                .unwrap();
            assert!(
                called.get(),
                "a SPEC holder authorizes the same nonholder-only path"
            );
            assert!(matches!(
                result,
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
        });
    }

    #[test]
    fn monitor_duplicate_unobserved_third_or_unknown_runtime_never_reaches_stop() {
        fixture(|project, sessions, _, proofs| {
            let called = Cell::new(false);
            let stop = || {
                called.set(true);
                Ok(true)
            };
            let mut third = proofs[1].session.clone();
            third.id = "third".into();
            third.save(sessions).unwrap();
            let path = gwt_agent::runtime_state_path_for_pid(sessions, std::process::id(), "third");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "invalid runtime json").unwrap();
            assert!(matches!(
                stop_monitor_duplicate(project, 4466, sessions, proofs, "nonholder", stop).unwrap(),
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
            let mut runtime = proofs[1].runtime.clone();
            runtime.child_pid = Some(std::process::id());
            runtime.child_started_at = runtime.host_started_at;
            runtime.save(&path).unwrap();
            assert!(matches!(
                stop_monitor_duplicate(project, 4466, sessions, proofs, "nonholder", stop).unwrap(),
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
            assert!(!called.get());
            third.linked_issue_number = Some(1234);
            third.save(sessions).unwrap();
            fs::write(&path, "invalid unrelated legacy runtime").unwrap();
            let _ =
                stop_monitor_duplicate(project, 4466, sessions, proofs, "nonholder", stop).unwrap();
            assert!(
                called.get(),
                "a known unrelated Issue must not fence this exact pair"
            );
        });
    }

    #[test]
    fn monitor_duplicate_preserves_two_distinct_active_generation_holders() {
        fixture(|project, sessions, _, proofs| {
            let owner = ExecutionOwnerKey {
                kind: ExecutionOwnerKind::Issue,
                number: 4466,
            };
            let request = SuccessorRequest {
                operation_id: "concurrent-monitor-pair".into(),
                principal_id: "host".into(),
                work_id: None,
                source: CONCURRENT_LINKED_OWNER_LAUNCH_SOURCE.into(),
                session_binding_id: "second-binding".into(),
                initial_session_id: "nonholder".into(),
                entrypoint: "gwt-execute".into(),
                requested_at: Utc::now(),
            };
            prepare_concurrent_linked_owner_launch_successor(project, owner, &request).unwrap();
            activate_successor(project, owner, &request).unwrap();
            let binding = current_execution_binding(project, owner).unwrap().unwrap();
            super::super::tests::persist_generation_session_binding(
                project,
                owner,
                "nonholder",
                binding,
            );
            let mut pair = proofs.clone();
            pair[1].session = gwt_agent::Session::load(&sessions.join("nonholder.toml")).unwrap();
            pair[1].runtime.execution_identity =
                gwt_agent::SessionExecutionIdentity::from_session(&pair[1].session).unwrap();
            pair[1]
                .runtime
                .save(&gwt_agent::runtime_state_path_for_pid(
                    sessions,
                    pair[1].host_pid,
                    "nonholder",
                ))
                .unwrap();
            let called = Cell::new(false);
            let result = stop_monitor_duplicate(project, 4466, sessions, &pair, "holder", || {
                called.set(true);
                Ok(true)
            })
            .unwrap();
            assert!(matches!(
                result,
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
            assert!(!called.get());
        });
    }

    #[test]
    fn monitor_duplicate_unknown_binding_is_not_proof_of_nonownership() {
        fixture(|project, sessions, _, proofs| {
            let mut pair = proofs.clone();
            let mut binding = pair[0].session.execution_binding.clone().unwrap();
            binding.session_id = "nonholder".into();
            binding.identity.generation_id = "unknown-generation".into();
            pair[1].session.execution_binding = Some(binding);
            pair[1].session.save(sessions).unwrap();
            pair[1].runtime.execution_identity =
                gwt_agent::SessionExecutionIdentity::from_session(&pair[1].session).unwrap();
            pair[1]
                .runtime
                .save(&gwt_agent::runtime_state_path_for_pid(
                    sessions,
                    pair[1].host_pid,
                    "nonholder",
                ))
                .unwrap();
            let called = Cell::new(false);
            let result =
                stop_monitor_duplicate(project, 4466, sessions, &pair, "nonholder", || {
                    called.set(true);
                    Ok(false)
                })
                .unwrap();
            assert!(matches!(
                result,
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
            assert!(
                !called.get(),
                "unknown ownership must be refused before attempting a stop"
            );
        });
    }

    #[test]
    fn monitor_duplicate_failed_exit_proof_keeps_durable_runtime_alive() {
        fixture(|project, sessions, _, proofs| {
            let called = Cell::new(false);
            let before = fs::read(sessions.join("nonholder.toml")).unwrap();
            let result =
                stop_monitor_duplicate(project, 4466, sessions, proofs, "nonholder", || {
                    called.set(true);
                    Ok(false)
                })
                .unwrap();
            assert!(called.get(), "an exact nonholder reaches the stop callback");
            assert!(matches!(
                result,
                MonitorDuplicateStopOutcome::Refused { .. }
            ));
            assert_eq!(fs::read(sessions.join("nonholder.toml")).unwrap(), before);
        });
    }
}
