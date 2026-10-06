//! Strict admission-deferred continuation and trusted command provenance (#5035).

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Continuation {
    pub commands: Vec<String>,
    pub headed_e2e_commands: Vec<String>,
    pub authority_hash: String,
    /// Occurrence in the complete matrix for each recorded result. Strings
    /// alone cannot distinguish repeated commands or Light-first execution.
    pub command_indices: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<PreviousRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousRun {
    pub record_id: String,
    pub content_hash: String,
    pub reused_commands: usize,
}

impl Continuation {
    pub(super) fn new(
        commands: &[String],
        headed: &[String],
        authority: &VerificationCallerAuthority,
    ) -> Self {
        Self {
            commands: commands.to_vec(),
            headed_e2e_commands: headed.to_vec(),
            authority_hash: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(authority).expect("serializable authority"))
            ),
            command_indices: Vec::new(),
            previous: None,
        }
    }

    pub(super) fn remaining(&self, resumed: bool) -> Vec<usize> {
        let mut remaining: Vec<_> = (0..self.commands.len())
            .filter(|index| !self.command_indices.contains(index))
            .collect();
        if resumed {
            // Stable sorting keeps the Heavy order, including the final
            // restoration command. No command is moved across another Heavy.
            remaining.sort_by_key(|index| {
                crate::cli::verification_lease::classify_command(&self.commands[*index])
                    == crate::cli::verification_lease::CommandWeight::Heavy
            });
        }
        remaining
    }

    pub(super) fn missing(&self) -> Vec<String> {
        self.remaining(false)
            .into_iter()
            .map(|index| self.commands[index].clone())
            .collect()
    }
}

fn inputs_match(before: &Continuation, after: &Continuation) -> bool {
    before.commands == after.commands
        && before.headed_e2e_commands == after.headed_e2e_commands
        && !before.authority_hash.is_empty()
        && before.authority_hash == after.authority_hash
}

fn occurrences_valid(record: &VerificationRunRecord) -> bool {
    let Some(context) = &record.continuation else {
        return false;
    };
    let mut indices = BTreeSet::new();
    context.command_indices.len() == record.commands.len()
        && context
            .command_indices
            .iter()
            .zip(&record.commands)
            .all(|(index, result)| {
                indices.insert(*index)
                    && context.commands.get(*index) == Some(&result.command)
                    && (!context.headed_e2e_commands.contains(&result.command)
                        || result.headed_e2e.is_some())
            })
}

fn deferred_passes(record: &VerificationRunRecord) -> bool {
    !record.content_hash.is_empty()
        && integrity_ok(record)
        && record
            .lifecycle
            .as_ref()
            .is_some_and(|state| state.status == interruption::RunStatus::Deferred)
        && !record.all_passed
        && !record.plan_covered
        && record.started_at.is_some()
        && !record.commands.is_empty()
        && record.commands.iter().all(|result| {
            result.exit_code == 0
                && result.terminated_by_signal.is_none()
                && result
                    .headed_e2e
                    .as_ref()
                    .is_none_or(|headed| headed.passed())
        })
        && occurrences_valid(record)
        && record.continuation.as_ref().is_some_and(|context| {
            !context.missing().is_empty() && context.missing() == record.planned_missing
        })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn eligible(
    worktree: &Path,
    record: &VerificationRunRecord,
    session: &str,
    owner: Option<u64>,
    binding: Option<&ExecutionBindingIdentity>,
    plan: Option<&VerificationPlanRecord>,
    fingerprint: &str,
    request: &Continuation,
) -> bool {
    let Some(plan) = plan else {
        return false;
    };
    // A diagnostic mirror is never evidence for resumption, even when a
    // trusted directory exists but its authoritative latest record is absent.
    let trusted_record = crate::cli::trusted_store::read(worktree, "verification-run.json")
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_str::<VerificationRunRecord>(&bytes).ok());
    fingerprint != "no-git"
        && trusted_record.as_ref() == Some(record)
        && deferred_passes(record)
        && record.session_id == session
        && record.owner_number == owner
        && record.execution_binding.as_ref() == binding
        && record.worktree_fingerprint == fingerprint
        && !plan.content_hash.is_empty()
        && plan_integrity_ok(plan)
        && record.verification_plan_hash == plan.content_hash
        && record.verification_plan_snapshot.as_ref() == Some(plan)
        && record
            .continuation
            .as_ref()
            .is_some_and(|prior| inputs_match(prior, request))
        && chain_valid(worktree, record)
}

fn archive_name(id: &str) -> io::Result<String> {
    if !id.strip_prefix("vrr-").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid continuation record ID",
        ));
    }
    Ok(format!("verification-run-{id}.json"))
}

/// Called under the existing trusted write lease before replacing the latest
/// record. An existing snapshot must be identical; it is never overwritten.
pub(super) fn archive(worktree: &Path, record: &VerificationRunRecord) -> io::Result<()> {
    let name = archive_name(&record.record_id)?;
    let bytes = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    match crate::cli::trusted_store::read(worktree, &name)? {
        Some(existing) if existing.as_bytes() == bytes => Ok(()),
        Some(_) => Err(io::Error::new(
            ErrorKind::InvalidData,
            "continuation predecessor snapshot changed",
        )),
        None => crate::cli::trusted_store::write(worktree, &name, &bytes),
    }
}

/// Every carried prefix must exactly match its immutable, hashed predecessor.
/// This check is shared by resumption and downstream delivery gates.
pub(super) fn chain_valid(worktree: &Path, record: &VerificationRunRecord) -> bool {
    if record.continuation.is_none() {
        return true;
    }
    let mut current = record.clone();
    let mut seen = BTreeSet::new();
    loop {
        if !occurrences_valid(&current) || !seen.insert(current.record_id.clone()) {
            return false;
        }
        let context = current.continuation.as_ref().expect("validated context");
        if current.lifecycle.is_none() && !context.missing().is_empty() {
            return false;
        }
        let Some(link) = &context.previous else {
            return true;
        };
        let predecessor = archive_name(&link.record_id)
            .ok()
            .and_then(|name| {
                crate::cli::trusted_store::read(worktree, &name)
                    .ok()
                    .flatten()
            })
            .and_then(|bytes| serde_json::from_str::<VerificationRunRecord>(&bytes).ok());
        let Some(previous) = predecessor else {
            return false;
        };
        if previous.record_id != link.record_id
            || previous.content_hash != link.content_hash
            || !deferred_passes(&previous)
            || previous.session_id != current.session_id
            || previous.owner_number != current.owner_number
            || previous.execution_binding != current.execution_binding
            || previous.verification_plan_hash != current.verification_plan_hash
            || previous.worktree_fingerprint != current.worktree_fingerprint
            || previous.started_at != current.started_at
            || previous.commands.len() != link.reused_commands
            || current.commands.get(..link.reused_commands) != Some(previous.commands.as_slice())
            || !previous.continuation.as_ref().is_some_and(|prior| {
                inputs_match(prior, context)
                    && context.command_indices.get(..link.reused_commands)
                        == Some(prior.command_indices.as_slice())
            })
        {
            return false;
        }
        current = previous;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_requires_every_identity_and_only_successful_deferred_measurements() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        let authority = snapshot_verification_caller_authority(dir.path(), "resume").unwrap();
        let commands = vec!["git --version".to_string(), "git --version".to_string()];
        let plan = register_plan_for_caller(
            dir.path(),
            "resume",
            commands.clone(),
            vec![],
            vec![],
            false,
            &authority,
        )
        .unwrap();
        let mut calls = 0;
        let mut admission = |_: &str, _: &VerificationHost| {
            calls += 1;
            if calls == 2 {
                Err("verify: deferred — admission timeout".into())
            } else {
                Ok(None)
            }
        };
        assert!(run_verification_for_caller(
            dir.path(),
            "resume",
            &commands,
            &authority,
            &[],
            RunOptions {
                admit_command: Some(&mut admission),
                ..RunOptions::default()
            }
        )
        .unwrap_err()
        .contains("deferred"));
        let record = load(dir.path()).unwrap().unwrap();
        let request = Continuation::new(&commands, &[], &authority);
        let matches = |candidate: &VerificationRunRecord,
                       request: &Continuation,
                       plan: &VerificationPlanRecord| {
            eligible(
                dir.path(),
                candidate,
                "resume",
                None,
                None,
                Some(plan),
                &plan.worktree_fingerprint,
                request,
            )
        };
        assert!(matches(&record, &request, &plan));
        let trusted = crate::cli::trusted_store::trusted_dir_for_worktree(dir.path()).unwrap();
        fs::remove_file(trusted.join("verification-run.json")).unwrap();
        assert!(
            !matches(&record, &request, &plan),
            "a diagnostic mirror cannot authorize resumption"
        );
        save(dir.path(), &record).unwrap();
        for changed in [
            "owner",
            "session",
            "plan",
            "source",
            "authority",
            "failure",
            "signal",
            "interrupted",
            "running",
            "hashless",
            "corrupt",
            "occurrence",
        ] {
            let mut candidate = record.clone();
            match changed {
                "owner" => candidate.owner_number = Some(1),
                "session" => candidate.session_id = "another-session".into(),
                "plan" => candidate.verification_plan_hash = "another-plan".into(),
                "source" => candidate.worktree_fingerprint = "another-source".into(),
                "authority" => {
                    candidate.continuation.as_mut().unwrap().authority_hash =
                        "rotated-authority".into()
                }
                "failure" => candidate.commands[0].exit_code = 7,
                "signal" => candidate.commands[0].terminated_by_signal = Some(15),
                "interrupted" => {
                    candidate.lifecycle.as_mut().unwrap().status =
                        interruption::RunStatus::Interrupted
                }
                "running" => {
                    candidate.lifecycle.as_mut().unwrap().status = interruption::RunStatus::Running
                }
                "occurrence" => candidate.continuation.as_mut().unwrap().command_indices = vec![2],
                _ => {}
            }
            candidate.content_hash = compute_content_hash(&candidate);
            if changed == "hashless" {
                candidate.content_hash.clear();
            }
            if changed == "corrupt" {
                candidate.content_hash = "corrupt".into();
            }
            crate::cli::trusted_store::write(
                dir.path(),
                "verification-run.json",
                &serde_json::to_vec_pretty(&candidate).unwrap(),
            )
            .unwrap();
            assert!(
                !matches(&candidate, &request, &plan),
                "must run fresh for {changed}"
            );
        }
        save(dir.path(), &record).unwrap();
        let mut changed_plan = plan.clone();
        changed_plan.created_at += chrono::Duration::seconds(1);
        changed_plan.content_hash = compute_plan_hash(&changed_plan);
        assert!(
            !matches(&record, &request, &changed_plan),
            "semantic plan equality is insufficient"
        );
        let mut changed_request = request.clone();
        changed_request.commands.push(commands[0].clone());
        assert!(!matches(&record, &changed_request, &plan));
        changed_request = request.clone();
        changed_request
            .headed_e2e_commands
            .push(commands[0].clone());
        assert!(
            !matches(&record, &changed_request, &plan),
            "plain PASS cannot become headed evidence"
        );
        save(dir.path(), &record).unwrap();
        let (resumed, _) = run_verification_for_caller(
            dir.path(),
            "resume",
            &commands,
            &authority,
            &[],
            RunOptions::default(),
        )
        .unwrap();
        assert_eq!(
            resumed.commands.len(),
            2,
            "repeated command occurrences remain separate"
        );
        assert_eq!(
            resumed.continuation.as_ref().unwrap().command_indices,
            vec![0, 1]
        );
        assert!(chain_valid(dir.path(), &resumed));
        fs::remove_file(trusted.join(archive_name(&record.record_id).unwrap())).unwrap();
        assert_eq!(
            evaluate_evidence(dir.path(), "resume", None),
            EvidenceStatus::Tampered,
            "missing command provenance must refuse delivery"
        );
    }
}
