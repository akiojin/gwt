//! Durable project window choices. Runtime and CLI callers establish authority
//! before entering this store; a saved choice never replaces execution gates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Continue,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Pending,
    Confirmed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceRequest {
    pub operation_id: String,
    pub expected_revision: u64,
    pub generation: u64,
    pub choice: Choice,
    pub consent_source_id: String,
    pub actor: String,
    pub authorized_work_lineage: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub operation_id: String,
    pub request_digest: String,
    #[serde(default)]
    pub recovered_policy_digest: Option<String>,
    pub before_revision: u64,
    pub after_revision: u64,
    pub generation: u64,
    pub choice: Choice,
    pub consent_source_id: String,
    pub actor: String,
    pub authorized_work_lineage: Vec<String>,
    pub effect_state: EffectState,
    pub reason: Option<String>,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub project_key: String,
    pub revision: u64,
    pub choice: Option<Choice>,
    pub consent_source_id: Option<String>,
    pub actor: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub authorized_work_lineage: Vec<String>,
    pub receipts: Vec<Receipt>,
}

impl Policy {
    fn unselected(project_key: String) -> Self {
        Self {
            schema_version: 1,
            project_key,
            revision: 0,
            choice: None,
            consent_source_id: None,
            actor: None,
            decided_at: None,
            authorized_work_lineage: Vec::new(),
            receipts: Vec::new(),
        }
    }

    pub fn last_operation(&self) -> Option<&Receipt> {
        self.receipts.last()
    }

    pub fn operation(&self, operation_id: &str) -> Option<&Receipt> {
        self.receipts
            .iter()
            .find(|receipt| receipt.operation_id == operation_id)
    }
}

pub fn policy_path(repo: &Path) -> PathBuf {
    resolve(repo).1
}

fn resolve(repo: &Path) -> (String, PathBuf) {
    let key = crate::paths::project_scope_hash(repo);
    let path = crate::paths::gwt_project_dir(&key).join("project-state/project-window-policy.json");
    (key.to_string(), path)
}

/// A missing policy is unselected. Old auto-start preferences and saved Work
/// are deliberately not read here. Unreadable or inconsistent data is an error,
/// so callers can distinguish Unknown from a valid unselected project.
pub fn load(repo: &Path) -> io::Result<Policy> {
    let (project_key, path) = resolve(repo);
    load_path(&path, &project_key)
}

fn load_path(path: &Path, project_key: &str) -> io::Result<Policy> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Policy::unselected(project_key.to_string()))
        }
        Err(error) => return Err(error),
    };
    decode_policy(&bytes, project_key)
}

fn decode_policy(bytes: &[u8], project_key: &str) -> io::Result<Policy> {
    let policy: Policy = serde_json::from_slice(bytes).map_err(invalid_data)?;
    if policy.schema_version != 1 || policy.project_key != project_key {
        return Err(invalid_data("unsupported or foreign project window policy"));
    }
    let mut operations = HashSet::new();
    for (index, receipt) in policy.receipts.iter().enumerate() {
        let request = ChoiceRequest {
            operation_id: receipt.operation_id.clone(),
            expected_revision: receipt.before_revision,
            generation: receipt.generation,
            choice: receipt.choice,
            consent_source_id: receipt.consent_source_id.clone(),
            actor: receipt.actor.clone(),
            authorized_work_lineage: receipt.authorized_work_lineage.clone(),
        };
        if !canonical_uuid(&receipt.operation_id)
            || !operations.insert(&receipt.operation_id)
            || receipt.before_revision != index as u64
            || receipt.after_revision != receipt.before_revision + 1
            || receipt.generation == 0
            || receipt.request_digest.len() != 64
            || !receipt
                .request_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || validate_request(&request).is_err()
            || request_digest(project_key, &request)? != receipt.request_digest
            || receipt
                .recovered_policy_digest
                .as_ref()
                .is_some_and(|digest| !is_digest(digest))
        {
            return Err(invalid_data(
                "inconsistent project window operation history",
            ));
        }
    }
    if let Some(last) = policy.last_operation() {
        if policy.revision != last.after_revision
            || policy.choice != Some(last.choice)
            || policy.decided_at != Some(last.decided_at)
            || policy.consent_source_id.as_ref() != Some(&last.consent_source_id)
            || policy.actor.as_ref() != Some(&last.actor)
            || policy.authorized_work_lineage != last.authorized_work_lineage
        {
            return Err(invalid_data(
                "project window choice does not match its receipt",
            ));
        }
    } else if policy.revision != 0
        || policy.choice.is_some()
        || policy.consent_source_id.is_some()
        || policy.actor.is_some()
        || policy.decided_at.is_some()
        || !policy.authorized_work_lineage.is_empty()
    {
        return Err(invalid_data(
            "project window choice has no operation receipt",
        ));
    }
    Ok(policy)
}

/// Persist one exact intent before its runtime effect. Replaying an operation
/// returns its current receipt before checking CAS; it never reapplies a choice.
/// A returned Pending/Unknown receipt requires status reconciliation by the
/// executor, not a new operation ID or an assumption that its effect is absent.
pub fn choose(repo: &Path, request: &ChoiceRequest) -> io::Result<Receipt> {
    validate_request(request)?;
    let (project_key, path) = resolve(repo);
    let _lock = lock(&path)?;
    let mut policy = load_path(&path, &project_key)?;
    let digest = request_digest(&policy.project_key, request)?;
    if let Some(receipt) = policy.operation(&request.operation_id) {
        return if receipt.request_digest == digest {
            Ok(receipt.clone())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "operation ID already belongs to a different project window intent",
            ))
        };
    }
    persist_choice(&path, &mut policy, request, digest, None)
}

fn persist_choice(
    path: &Path,
    policy: &mut Policy,
    request: &ChoiceRequest,
    digest: String,
    recovered_policy_digest: Option<String>,
) -> io::Result<Receipt> {
    if policy.revision != request.expected_revision {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!(
                "project window policy revision changed: expected {}, current {}",
                request.expected_revision, policy.revision
            ),
        ));
    }
    let revision = policy
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid_data("project window policy revision exhausted"))?;
    let receipt = Receipt {
        operation_id: request.operation_id.clone(),
        request_digest: digest,
        recovered_policy_digest,
        before_revision: policy.revision,
        after_revision: revision,
        generation: request.generation,
        choice: request.choice,
        consent_source_id: request.consent_source_id.clone(),
        actor: request.actor.clone(),
        authorized_work_lineage: request.authorized_work_lineage.clone(),
        effect_state: EffectState::Pending,
        reason: None,
        decided_at: Utc::now(),
    };
    policy.revision = revision;
    policy.choice = Some(request.choice);
    policy.consent_source_id = Some(request.consent_source_id.clone());
    policy.actor = Some(request.actor.clone());
    policy.decided_at = Some(receipt.decided_at);
    policy.authorized_work_lineage = request.authorized_work_lineage.clone();
    policy.receipts.push(receipt.clone());
    save(path, policy)?;
    Ok(receipt)
}

/// Recover an unreadable V1 policy after the caller has verified an exact human
/// choice bound to the observed corrupt bytes. Ordinary callers use `choose`.
pub fn choose_recovering(
    repo: &Path,
    request: &ChoiceRequest,
    expected_corrupt_digest: &str,
) -> io::Result<Receipt> {
    validate_request(request)?;
    if request.expected_revision != 0 || !is_digest(expected_corrupt_digest) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "corrupt policy recovery requires revision zero and its exact SHA256",
        ));
    }
    let (project_key, path) = resolve(repo);
    let _lock = lock(&path)?;
    let bytes = fs::read(&path)?;
    let digest = request_digest(&project_key, request)?;
    if let Ok(policy) = decode_policy(&bytes, &project_key) {
        if let Some(receipt) = policy.operation(&request.operation_id) {
            if receipt.request_digest == digest
                && receipt.recovered_policy_digest.as_deref() == Some(expected_corrupt_digest)
            {
                return Ok(receipt.clone());
            }
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a valid project policy cannot be replaced through corrupt policy recovery",
        ));
    }
    if hex::encode(Sha256::digest(&bytes)) != expected_corrupt_digest {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "corrupt policy changed; inspect project.execution.status before choosing again",
        ));
    }
    // A newer schema or another project's readable data is not a V1 repair.
    // Preserve it and require the matching runtime/version instead of guessing.
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if value["schema_version"].as_u64() != Some(1)
            || value["project_key"].as_str() != Some(project_key.as_str())
        {
            return Err(invalid_data(
                "unsupported or foreign policy cannot be repaired as this project's V1 policy",
            ));
        }
    }
    let quarantine = path.with_file_name(format!(
        "project-window-policy.corrupt-{expected_corrupt_digest}.json"
    ));
    match fs::read(&quarantine) {
        Ok(saved) if saved == bytes => {}
        Ok(_) => {
            return Err(invalid_data(
                "corrupt policy archive does not match the observed bytes",
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Copy, never move the live file. A failed publication must leave
            // the same corrupt policy fail-closed, not a missing-policy default.
            crate::workspace_projection::write_atomic(&quarantine, &bytes)
                .map_err(io::Error::other)?;
        }
        Err(error) => return Err(error),
    }
    if fs::read(&quarantine)? != bytes {
        return Err(invalid_data(
            "corrupt policy archive readback did not match",
        ));
    }
    let mut policy = Policy::unselected(project_key);
    persist_choice(
        &path,
        &mut policy,
        request,
        digest,
        Some(expected_corrupt_digest.to_string()),
    )
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Only the executor's observation confirms a runtime effect. This update does
/// not change policy revision or authority and cannot regress a confirmed result.
pub fn record_effect(
    repo: &Path,
    operation_id: &str,
    state: EffectState,
    reason: Option<&str>,
) -> io::Result<Receipt> {
    if state == EffectState::Pending {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only a new choice may create a pending effect",
        ));
    }
    let (project_key, path) = resolve(repo);
    let _lock = lock(&path)?;
    let mut policy = load_path(&path, &project_key)?;
    let receipt = policy
        .receipts
        .iter_mut()
        .find(|receipt| receipt.operation_id == operation_id)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "project window operation not found",
            )
        })?;
    if receipt.effect_state == EffectState::Confirmed && state != EffectState::Confirmed {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a confirmed runtime effect cannot be made unknown",
        ));
    }
    receipt.effect_state = state;
    receipt.reason = reason.map(str::to_owned);
    let receipt = receipt.clone();
    save(&path, &policy)?;
    Ok(receipt)
}

fn canonical_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

fn validate_request(request: &ChoiceRequest) -> io::Result<()> {
    let mut lineage = HashSet::new();
    if !canonical_uuid(&request.operation_id)
        || request.generation == 0
        || request.consent_source_id.trim().is_empty()
        || request.actor.trim().is_empty()
        || request
            .authorized_work_lineage
            .iter()
            .any(|id| id.trim().is_empty() || !lineage.insert(id))
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "project window choice requires a canonical operation UUID, generation, consent source, actor and distinct lineage IDs"));
    }
    Ok(())
}

fn request_digest(project_key: &str, request: &ChoiceRequest) -> io::Result<String> {
    let bytes = serde_json::to_vec(&(project_key, request)).map_err(io::Error::other)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn lock(path: &Path) -> io::Result<crate::operation_deadline::NamedFileLock> {
    fs::create_dir_all(path.parent().expect("project policy path has a parent"))?;
    crate::operation_deadline::NamedFileLock::acquire(
        &path.with_extension("lock"),
        "project_window_policy",
    )
}

fn save(path: &Path, policy: &Policy) -> io::Result<()> {
    // Reuse the existing durable atomic publication, including file/parent sync.
    crate::workspace_projection::write_atomic(
        path,
        &serde_json::to_vec(policy).map_err(io::Error::other)?,
    )
    .map_err(io::Error::other)?;
    // The lock may have created the project directory before the atomic writer
    // saw it. Sync that new directory's parents before reporting acceptance.
    #[cfg(unix)]
    for parent in path.parent().expect("project policy parent").ancestors() {
        fs::File::open(parent)?.sync_all()?;
        if parent == crate::paths::gwt_home() {
            break;
        }
    }
    Ok(())
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedGwtHome;
    use std::fs;

    fn request(revision: u64, choice: Choice) -> ChoiceRequest {
        ChoiceRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            expected_revision: revision,
            generation: 7,
            choice,
            consent_source_id: "visible-close:client:nonce".into(),
            actor: "client".into(),
            authorized_work_lineage: vec!["owner:6682".into()],
        }
    }

    #[test]
    fn missing_policy_does_not_upgrade_legacy_auto_start_or_saved_work() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(repo.join(".gwt")).unwrap();
        fs::write(repo.join(".gwt/settings.json"), br#"{"auto_start":true}"#).unwrap();
        fs::write(
            repo.join(".gwt/session.json"),
            br#"{"live_work":"preserved"}"#,
        )
        .unwrap();

        let policy = load(&repo).expect("missing policy is an unselected project");
        assert_eq!(policy.revision, 0);
        assert_eq!(policy.choice, None);
        assert!(policy.authorized_work_lineage.is_empty());
        assert!(!policy_path(&repo).exists());
        assert!(repo.join(".gwt/session.json").exists());
    }

    #[test]
    fn choice_is_durable_pending_until_the_executor_reports_the_effect() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let input = request(0, Choice::Stop);
        let receipt = choose(&repo, &input).expect("save choice before effect");
        assert_eq!(receipt.effect_state, EffectState::Pending);
        assert_eq!(receipt.after_revision, 1);
        assert_eq!(load(&repo).unwrap().choice, Some(Choice::Stop));

        let unknown = record_effect(
            &repo,
            &input.operation_id,
            EffectState::Unknown,
            Some("drain ACK missing"),
        )
        .unwrap();
        assert_eq!(unknown.effect_state, EffectState::Unknown);
        assert_eq!(load(&repo).unwrap().receipts[0], unknown);
        let confirmed =
            record_effect(&repo, &input.operation_id, EffectState::Confirmed, None).unwrap();
        assert_eq!(confirmed.effect_state, EffectState::Confirmed);
        assert_eq!(load(&repo).unwrap().revision, 1);
        assert!(record_effect(&repo, &input.operation_id, EffectState::Pending, None).is_err());
        assert_eq!(load(&repo).unwrap().receipts[0], confirmed);
    }

    #[test]
    fn replay_matches_the_original_intent_before_cas_and_never_rolls_back_choice() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let input = request(0, Choice::Continue);
        let receipt = choose(&repo, &input).unwrap();
        record_effect(&repo, &input.operation_id, EffectState::Confirmed, None).unwrap();
        let stop = request(1, Choice::Stop);
        choose(&repo, &stop).unwrap();
        let saved = fs::read(policy_path(&repo)).unwrap();

        let replay = choose(&repo, &input).expect("lost ACK reconciles the original operation");
        assert_eq!(replay.operation_id, receipt.operation_id);
        assert_eq!(replay.effect_state, EffectState::Confirmed);
        assert_eq!(replay.actor, input.actor);
        assert_eq!(replay.consent_source_id, input.consent_source_id);
        assert_eq!(
            replay.authorized_work_lineage,
            input.authorized_work_lineage
        );
        let mut conflicting = input.clone();
        conflicting
            .authorized_work_lineage
            .push("owner:foreign".into());
        assert!(choose(&repo, &conflicting).is_err());
        assert!(choose(&repo, &request(0, Choice::Continue)).is_err());
        assert_eq!(fs::read(policy_path(&repo)).unwrap(), saved);
        assert_eq!(load(&repo).unwrap().choice, Some(Choice::Stop));
    }

    #[test]
    fn continue_lineage_is_project_local_and_preserves_unrelated_records() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("one");
        let other = home.path().join("two");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&other).unwrap();
        let works = policy_path(&repo).with_file_name("works.json");
        fs::create_dir_all(works.parent().unwrap()).unwrap();
        fs::write(&works, b"keep Work and history").unwrap();
        choose(&repo, &request(0, Choice::Continue)).unwrap();

        assert_eq!(
            load(&repo).unwrap().authorized_work_lineage,
            vec!["owner:6682"]
        );
        assert_eq!(load(&other).unwrap().choice, None);
        assert_eq!(fs::read(works).unwrap(), b"keep Work and history");
    }

    #[test]
    fn foreign_unsupported_and_corrupt_policy_fail_closed_without_rewriting() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        choose(&repo, &request(0, Choice::Continue)).unwrap();
        let path = policy_path(&repo);
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut foreign = original.clone();
        foreign["project_key"] = "foreign-project".into();
        let mut unsupported = original;
        unsupported["schema_version"] = 99.into();
        for (index, bytes) in [
            serde_json::to_vec(&foreign).unwrap(),
            serde_json::to_vec(&unsupported).unwrap(),
            b"{".to_vec(),
        ]
        .into_iter()
        .enumerate()
        {
            fs::write(&path, &bytes).unwrap();
            assert!(load(&repo).is_err());
            assert!(choose(&repo, &request(1, Choice::Stop)).is_err());
            if index < 2 {
                assert!(choose_recovering(
                    &repo,
                    &request(0, Choice::Stop),
                    &hex::encode(Sha256::digest(&bytes))
                )
                .is_err());
            }
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn invalid_operation_or_unwritable_policy_never_claims_acceptance() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let mut input = request(0, Choice::Continue);
        input.operation_id = "not-a-uuid".into();
        assert!(choose(&repo, &input).is_err());
        assert!(!policy_path(&repo).exists());
        // A directory at the fixed destination deterministically fails on all
        // platforms, without permission assumptions or a failure injection API.
        fs::create_dir_all(policy_path(&repo)).unwrap();
        assert!(choose(&repo, &request(0, Choice::Continue)).is_err());
        assert!(policy_path(&repo).is_dir());
    }

    #[test]
    fn recovery_preserves_exact_corrupt_bytes_and_replays_only_the_same_intent() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let path = policy_path(&repo);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let corrupt = b"{truncated project policy";
        fs::write(&path, corrupt).unwrap();
        let digest = hex::encode(Sha256::digest(corrupt));
        let input = request(0, Choice::Continue);
        assert!(choose_recovering(&repo, &input, &"0".repeat(64)).is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupt);
        let quarantine =
            path.with_file_name(format!("project-window-policy.corrupt-{digest}.json"));
        fs::create_dir_all(&quarantine).unwrap();
        assert!(choose_recovering(&repo, &input, &digest).is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            corrupt,
            "archive failure must not remove the live fail-closed policy"
        );
        fs::remove_dir(&quarantine).unwrap();
        let receipt = choose_recovering(&repo, &input, &digest)
            .expect("exact authorized choice repairs the observed corrupt snapshot");
        assert_eq!(receipt.effect_state, EffectState::Pending);
        assert_eq!(
            receipt.recovered_policy_digest.as_deref(),
            Some(digest.as_str())
        );
        assert_eq!(load(&repo).unwrap().choice, Some(Choice::Continue));
        assert_eq!(fs::read(&quarantine).unwrap(), corrupt);
        let saved = fs::read(&path).unwrap();
        assert_eq!(choose_recovering(&repo, &input, &digest).unwrap(), receipt);
        assert!(choose_recovering(&repo, &input, &"0".repeat(64)).is_err());
        let mut changed = input.clone();
        changed.choice = Choice::Stop;
        assert!(choose_recovering(&repo, &changed, &digest).is_err());
        assert!(choose_recovering(&repo, &request(0, Choice::Stop), &digest).is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert_eq!(fs::read(&quarantine).unwrap(), corrupt);
    }
}
