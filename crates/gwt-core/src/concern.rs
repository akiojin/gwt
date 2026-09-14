//! Machine-local records for symptoms that must remain visible until verified.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{GwtError, JsonDecodeKind};
use crate::paths::gwt_project_dir_for_repo_path;
use crate::workspace_projection::write_atomic;
use crate::Result;

const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SymptomMeasurement {
    GwtdOperation { operation: String, params: Value },
    ShellCommand { command: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateOp {
    Eq,
    Lte,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationPredicate {
    pub pointer: String,
    pub op: PredicateOp,
    pub expected: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcernState {
    Open,
    FixLanded,
    Verified,
    Withdrawn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerIssueState {
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestProgress {
    pub number: u64,
    pub lifecycle: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerProgress {
    pub number: u64,
    pub state: OwnerIssueState,
    pub queue_position: Option<u64>,
    pub status: Option<String>,
    #[serde(default)]
    pub pull_requests: Vec<PullRequestProgress>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewConcern {
    pub summary: String,
    pub symptom_measurement: SymptomMeasurement,
    pub baseline: Value,
    pub verification_predicate: VerificationPredicate,
    pub owner_issues: Vec<u64>,
    #[serde(default = "default_escalate_after_cycles")]
    pub escalate_after_cycles: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcernPatch {
    pub summary: Option<String>,
    pub symptom_measurement: Option<SymptomMeasurement>,
    pub verification_predicate: Option<VerificationPredicate>,
    pub owner_issues: Option<Vec<u64>>,
    pub escalate_after_cycles: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementUpdate {
    pub cycle_id: String,
    pub measurement: Value,
    pub owner_progress: Vec<OwnerProgress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcernResolution {
    Verified,
    Open,
    Withdrawn,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcernFilter {
    pub state: Option<ConcernState>,
    pub symptom_measurement: Option<SymptomMeasurement>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Concern {
    pub id: String,
    pub summary: String,
    pub raised_at: String,
    pub symptom_measurement: SymptomMeasurement,
    pub baseline: Value,
    pub verification_predicate: VerificationPredicate,
    pub owner_issues: Vec<u64>,
    pub state: ConcernState,
    pub last_measured_at: Option<String>,
    pub last_measurement: Option<Value>,
    pub previous_measurement: Option<Value>,
    pub measurement_changed: bool,
    pub owner_progress: Vec<OwnerProgress>,
    pub owner_progress_changed: bool,
    pub last_cycle_id: Option<String>,
    pub stagnant_cycles: u32,
    pub escalate_after_cycles: u32,
    pub escalation_due: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateOutcome {
    pub concern: Concern,
    pub reused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveOutcome {
    pub concern: Concern,
    pub predicate_passed: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcernListSummary {
    pub open_count: usize,
    pub oldest_raised_at: Option<String>,
    pub unresolved_count: usize,
    pub oldest_unresolved_raised_at: Option<String>,
    pub escalation_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcernList {
    pub concerns: Vec<Concern>,
    pub summary: ConcernListSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConcernFile {
    schema_version: u32,
    concerns: Vec<Concern>,
}

pub struct ConcernStore {
    state_path: PathBuf,
    lock_path: PathBuf,
}

impl ConcernStore {
    pub fn for_repo(repo_path: &Path) -> Self {
        let directory = gwt_project_dir_for_repo_path(repo_path).join("project-state");
        Self {
            state_path: directory.join("concerns.json"),
            lock_path: directory.join("concerns.lock"),
        }
    }

    pub fn create(&self, mut input: NewConcern) -> Result<CreateOutcome> {
        validate_new(&input)?;
        normalize_owner_ids(&mut input.owner_issues);
        self.with_lock(|concerns| {
            if let Some(existing) = concerns
                .iter_mut()
                .find(|concern| concern.symptom_measurement == input.symptom_measurement)
            {
                let previous = existing
                    .last_measurement
                    .clone()
                    .unwrap_or_else(|| existing.baseline.clone());
                existing.previous_measurement = Some(previous.clone());
                existing.last_measurement = Some(input.baseline.clone());
                existing.measurement_changed = previous != input.baseline;
                existing.last_measured_at = Some(now());
                if matches!(
                    existing.state,
                    ConcernState::Verified | ConcernState::Withdrawn
                ) {
                    existing.state = ConcernState::Open;
                    existing.escalation_due = false;
                }
                return Ok(CreateOutcome {
                    concern: existing.clone(),
                    reused: true,
                });
            }

            let concern = Concern {
                id: Uuid::new_v4().to_string(),
                summary: input.summary,
                raised_at: now(),
                symptom_measurement: input.symptom_measurement,
                baseline: input.baseline,
                verification_predicate: input.verification_predicate,
                owner_issues: input.owner_issues,
                state: ConcernState::Open,
                last_measured_at: None,
                last_measurement: None,
                previous_measurement: None,
                measurement_changed: false,
                owner_progress: Vec::new(),
                owner_progress_changed: false,
                last_cycle_id: None,
                stagnant_cycles: 0,
                escalate_after_cycles: input.escalate_after_cycles,
                escalation_due: false,
            };
            concerns.push(concern.clone());
            Ok(CreateOutcome {
                concern,
                reused: false,
            })
        })
    }

    pub fn update(&self, id: &str, mut patch: ConcernPatch) -> Result<Concern> {
        validate_patch(&patch)?;
        if let Some(owner_issues) = patch.owner_issues.as_mut() {
            normalize_owner_ids(owner_issues);
        }
        self.with_lock(|concerns| {
            if let Some(definition) = patch.symptom_measurement.as_ref() {
                if concerns.iter().any(|candidate| {
                    candidate.id != id && candidate.symptom_measurement == *definition
                }) {
                    return Err(other(
                        "another concern already uses this symptom measurement",
                    ));
                }
            }
            let concern = find_mut(concerns, id)?;
            let invalidates_evidence = patch
                .symptom_measurement
                .as_ref()
                .is_some_and(|value| value != &concern.symptom_measurement)
                || patch
                    .verification_predicate
                    .as_ref()
                    .is_some_and(|value| value != &concern.verification_predicate)
                || patch
                    .owner_issues
                    .as_ref()
                    .is_some_and(|value| value != &concern.owner_issues);

            if let Some(summary) = patch.summary {
                concern.summary = summary;
            }
            if let Some(value) = patch.symptom_measurement {
                concern.symptom_measurement = value;
            }
            if let Some(value) = patch.verification_predicate {
                concern.verification_predicate = value;
            }
            if let Some(value) = patch.owner_issues {
                concern.owner_issues = value;
            }
            if let Some(value) = patch.escalate_after_cycles {
                concern.escalate_after_cycles = value;
                concern.escalation_due =
                    is_unresolved(concern.state) && concern.stagnant_cycles >= value;
            }
            if invalidates_evidence {
                invalidate_evidence(concern);
            }
            Ok(concern.clone())
        })
    }

    pub fn measure(&self, id: &str, mut update: MeasurementUpdate) -> Result<Concern> {
        validate_measurement_update(&update)?;
        normalize_owner_progress(&mut update.owner_progress);
        self.with_lock(|concerns| {
            let concern = find_mut(concerns, id)?;
            ensure_complete_owner_progress(concern, &update.owner_progress)?;
            if concern.last_cycle_id.as_deref() == Some(update.cycle_id.as_str())
                && concern.last_measurement.as_ref() == Some(&update.measurement)
                && concern.owner_progress == update.owner_progress
            {
                return Ok(concern.clone());
            }
            let prior_measurement = concern
                .last_measurement
                .clone()
                .unwrap_or_else(|| concern.baseline.clone());
            concern.previous_measurement = Some(prior_measurement.clone());
            concern.measurement_changed = prior_measurement != update.measurement;
            concern.last_measurement = Some(update.measurement);
            concern.last_measured_at = Some(now());

            let progress_changed = concern.owner_progress != update.owner_progress;
            concern.owner_progress_changed = progress_changed;
            if concern.last_cycle_id.as_deref() != Some(update.cycle_id.as_str()) {
                concern.stagnant_cycles = if concern.last_cycle_id.is_none() {
                    1
                } else if progress_changed {
                    0
                } else {
                    concern.stagnant_cycles.saturating_add(1)
                };
                concern.last_cycle_id = Some(update.cycle_id);
            } else if progress_changed {
                concern.stagnant_cycles = 0;
            }
            concern.owner_progress = update.owner_progress;

            if concern.state != ConcernState::Withdrawn {
                let all_closed = !concern.owner_progress.is_empty()
                    && concern
                        .owner_progress
                        .iter()
                        .all(|owner| owner.state == OwnerIssueState::Closed);
                if concern.state == ConcernState::Verified {
                    if !predicate_passes(
                        &concern.verification_predicate,
                        concern.last_measurement.as_ref(),
                    ) {
                        concern.state = ConcernState::Open;
                    }
                } else {
                    concern.state = if all_closed {
                        ConcernState::FixLanded
                    } else {
                        ConcernState::Open
                    };
                }
            }
            concern.escalation_due = is_unresolved(concern.state)
                && concern.stagnant_cycles >= concern.escalate_after_cycles;
            Ok(concern.clone())
        })
    }

    pub fn resolve(&self, id: &str, resolution: ConcernResolution) -> Result<ResolveOutcome> {
        self.with_lock(|concerns| {
            let concern = find_mut(concerns, id)?;
            let predicate_passed = match resolution {
                ConcernResolution::Verified => {
                    if concern.last_measurement.is_none() {
                        return Err(other("verification requires measurement evidence"));
                    }
                    let passed = predicate_passes(
                        &concern.verification_predicate,
                        concern.last_measurement.as_ref(),
                    );
                    concern.state = if passed {
                        ConcernState::Verified
                    } else {
                        ConcernState::Open
                    };
                    Some(passed)
                }
                ConcernResolution::Open => {
                    concern.state = ConcernState::Open;
                    None
                }
                ConcernResolution::Withdrawn => {
                    concern.state = ConcernState::Withdrawn;
                    None
                }
            };
            concern.escalation_due = is_unresolved(concern.state)
                && concern.stagnant_cycles >= concern.escalate_after_cycles;
            Ok(ResolveOutcome {
                concern: concern.clone(),
                predicate_passed,
            })
        })
    }

    pub fn list(&self, filter: ConcernFilter) -> Result<ConcernList> {
        self.with_read_lock(|concerns| {
            let summary = summarize(&concerns);
            let concerns = concerns
                .into_iter()
                .filter(|concern| {
                    filter.state.is_none_or(|state| concern.state == state)
                        && filter
                            .symptom_measurement
                            .as_ref()
                            .is_none_or(|definition| concern.symptom_measurement == *definition)
                })
                .collect();
            Ok(ConcernList { concerns, summary })
        })
    }

    fn with_lock<T>(&self, operation: impl FnOnce(&mut Vec<Concern>) -> Result<T>) -> Result<T> {
        self.with_file_lock(|| {
            let mut file = self.read_file()?;
            let result = operation(&mut file.concerns)?;
            let bytes = serde_json::to_vec_pretty(&file)
                .map_err(|error| other(format!("serialize concern store: {error}")))?;
            write_atomic(&self.state_path, &bytes)?;
            Ok(result)
        })
    }

    fn with_read_lock<T>(&self, operation: impl FnOnce(Vec<Concern>) -> Result<T>) -> Result<T> {
        self.with_file_lock(|| operation(self.read_file()?.concerns))
    }

    fn with_file_lock<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        if let Some(parent) = self.lock_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)?;
        crate::operation_deadline::lock_exclusive(&lock)?;
        let result = operation();
        if let Err(error) = FileExt::unlock(&lock) {
            tracing::warn!(path = %self.lock_path.display(), %error, "concern lock unlock failed");
        }
        result
    }

    fn read_file(&self) -> Result<ConcernFile> {
        let bytes = match fs::read(&self.state_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ConcernFile {
                    schema_version: SCHEMA_VERSION,
                    concerns: Vec::new(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        let file: ConcernFile =
            serde_json::from_slice(&bytes).map_err(|error| GwtError::JsonDecode {
                context: "decode concern store",
                kind: JsonDecodeKind::Malformed,
                message: error.to_string(),
            })?;
        if file.schema_version != SCHEMA_VERSION {
            return Err(GwtError::JsonDecode {
                context: "decode concern store",
                kind: JsonDecodeKind::IncompatibleSchema,
                message: format!(
                    "expected schema version {SCHEMA_VERSION}, found {}",
                    file.schema_version
                ),
            });
        }
        Ok(file)
    }
}

pub fn has_unresolved_concerns(repo_path: &Path) -> Result<bool> {
    Ok(ConcernStore::for_repo(repo_path)
        .list(ConcernFilter::default())?
        .summary
        .unresolved_count
        > 0)
}

fn validate_new(input: &NewConcern) -> Result<()> {
    validate_nonempty("summary", &input.summary)?;
    validate_measurement_definition(&input.symptom_measurement)?;
    validate_predicate(&input.verification_predicate)?;
    validate_owner_ids(&input.owner_issues)?;
    validate_threshold(input.escalate_after_cycles)
}

fn validate_patch(patch: &ConcernPatch) -> Result<()> {
    if patch.summary.is_none()
        && patch.symptom_measurement.is_none()
        && patch.verification_predicate.is_none()
        && patch.owner_issues.is_none()
        && patch.escalate_after_cycles.is_none()
    {
        return Err(other("concern patch must contain at least one field"));
    }
    if let Some(summary) = patch.summary.as_ref() {
        validate_nonempty("summary", summary)?;
    }
    if let Some(definition) = patch.symptom_measurement.as_ref() {
        validate_measurement_definition(definition)?;
    }
    if let Some(predicate) = patch.verification_predicate.as_ref() {
        validate_predicate(predicate)?;
    }
    if let Some(owner_issues) = patch.owner_issues.as_ref() {
        validate_owner_ids(owner_issues)?;
    }
    if let Some(threshold) = patch.escalate_after_cycles {
        validate_threshold(threshold)?;
    }
    Ok(())
}

fn validate_measurement_definition(definition: &SymptomMeasurement) -> Result<()> {
    match definition {
        SymptomMeasurement::GwtdOperation { operation, params } => {
            validate_nonempty("measurement operation", operation)?;
            if !params.is_object() {
                return Err(other("measurement operation params must be a JSON object"));
            }
            Ok(())
        }
        SymptomMeasurement::ShellCommand { command } => {
            validate_nonempty("measurement command", command)
        }
    }
}

fn validate_predicate(predicate: &VerificationPredicate) -> Result<()> {
    if !valid_json_pointer(&predicate.pointer) {
        return Err(other(
            "verification predicate pointer must be a valid JSON pointer",
        ));
    }
    if predicate.op == PredicateOp::Lte && !predicate.expected.is_number() {
        return Err(other(
            "lte verification predicate expected value must be numeric",
        ));
    }
    Ok(())
}

fn validate_owner_ids(owner_issues: &[u64]) -> Result<()> {
    if owner_issues.contains(&0) {
        return Err(other("owner_issues must contain positive Issue numbers"));
    }
    let mut normalized = owner_issues.to_vec();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.len() != owner_issues.len() {
        return Err(other("owner_issues must not contain duplicates"));
    }
    Ok(())
}

fn validate_threshold(value: u32) -> Result<()> {
    if value == 0 {
        Err(other("escalate_after_cycles must be positive"))
    } else {
        Ok(())
    }
}

fn validate_measurement_update(update: &MeasurementUpdate) -> Result<()> {
    validate_nonempty("cycle_id", &update.cycle_id)?;
    for owner in &update.owner_progress {
        if owner.number == 0
            || owner
                .pull_requests
                .iter()
                .any(|pull_request| pull_request.number == 0)
        {
            return Err(other("owner and pull request numbers must be positive"));
        }
    }
    Ok(())
}

fn validate_nonempty(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(other(format!("{label} must not be empty")))
    } else {
        Ok(())
    }
}

fn valid_json_pointer(pointer: &str) -> bool {
    if pointer.is_empty() {
        return true;
    }
    pointer.starts_with('/')
        && pointer.split('/').skip(1).all(|token| {
            let bytes = token.as_bytes();
            let mut index = 0;
            while index < bytes.len() {
                if bytes[index] == b'~' {
                    if index + 1 == bytes.len() || !matches!(bytes[index + 1], b'0' | b'1') {
                        return false;
                    }
                    index += 2;
                } else {
                    index += 1;
                }
            }
            true
        })
}

fn ensure_complete_owner_progress(concern: &Concern, progress: &[OwnerProgress]) -> Result<()> {
    let mut actual: Vec<_> = progress.iter().map(|owner| owner.number).collect();
    actual.sort_unstable();
    if actual.windows(2).any(|pair| pair[0] == pair[1]) || actual != concern.owner_issues {
        return Err(other(
            "owner_progress must contain each owner Issue exactly once",
        ));
    }
    Ok(())
}

fn normalize_owner_ids(owner_issues: &mut [u64]) {
    owner_issues.sort_unstable();
}

fn normalize_owner_progress(progress: &mut [OwnerProgress]) {
    for owner in progress.iter_mut() {
        owner
            .pull_requests
            .sort_by_key(|pull_request| pull_request.number);
    }
    progress.sort_by_key(|owner| owner.number);
}

fn invalidate_evidence(concern: &mut Concern) {
    concern.state = ConcernState::Open;
    concern.last_measured_at = None;
    concern.last_measurement = None;
    concern.previous_measurement = None;
    concern.measurement_changed = false;
    concern.owner_progress.clear();
    concern.owner_progress_changed = false;
    concern.last_cycle_id = None;
    concern.stagnant_cycles = 0;
    concern.escalation_due = false;
}

fn predicate_passes(predicate: &VerificationPredicate, measurement: Option<&Value>) -> bool {
    let Some(actual) = measurement.and_then(|value| value.pointer(&predicate.pointer)) else {
        return false;
    };
    match predicate.op {
        PredicateOp::Eq => actual == &predicate.expected,
        PredicateOp::Lte => numeric_lte(actual, &predicate.expected),
    }
}

fn numeric_lte(actual: &Value, expected: &Value) -> bool {
    match (actual.as_i64(), expected.as_i64()) {
        (Some(actual), Some(expected)) => return actual <= expected,
        _ => {}
    }
    match (actual.as_u64(), expected.as_u64()) {
        (Some(actual), Some(expected)) => return actual <= expected,
        _ => {}
    }
    match (actual.as_f64(), expected.as_f64()) {
        (Some(actual), Some(expected)) => actual <= expected,
        _ => false,
    }
}

fn summarize(concerns: &[Concern]) -> ConcernListSummary {
    let open: Vec<_> = concerns
        .iter()
        .filter(|concern| concern.state == ConcernState::Open)
        .collect();
    let unresolved: Vec<_> = concerns
        .iter()
        .filter(|concern| is_unresolved(concern.state))
        .collect();
    ConcernListSummary {
        open_count: open.len(),
        oldest_raised_at: open.iter().map(|concern| concern.raised_at.clone()).min(),
        unresolved_count: unresolved.len(),
        oldest_unresolved_raised_at: unresolved
            .iter()
            .map(|concern| concern.raised_at.clone())
            .min(),
        escalation_count: concerns
            .iter()
            .filter(|concern| concern.escalation_due)
            .count(),
    }
}

fn find_mut<'a>(concerns: &'a mut [Concern], id: &str) -> Result<&'a mut Concern> {
    concerns
        .iter_mut()
        .find(|concern| concern.id == id)
        .ok_or_else(|| other(format!("concern not found: {id}")))
}

fn is_unresolved(state: ConcernState) -> bool {
    matches!(state, ConcernState::Open | ConcernState::FixLanded)
}

fn default_escalate_after_cycles() -> u32 {
    10
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn other(message: impl Into<String>) -> GwtError {
    GwtError::Other(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicate_comparison_is_typed_and_missing_paths_fail_closed() {
        let measurement = serde_json::json!({"count": 2, "label": "2"});
        let predicate = VerificationPredicate {
            pointer: "/count".into(),
            op: PredicateOp::Lte,
            expected: Value::from(3),
        };
        assert!(predicate_passes(&predicate, Some(&measurement)));
        assert!(predicate_passes(
            &VerificationPredicate {
                pointer: String::new(),
                op: PredicateOp::Eq,
                expected: measurement.clone(),
            },
            Some(&measurement)
        ));
        assert!(!predicate_passes(
            &VerificationPredicate {
                pointer: "/label".into(),
                ..predicate.clone()
            },
            Some(&measurement)
        ));
        assert!(!predicate_passes(
            &VerificationPredicate {
                pointer: "/missing".into(),
                ..predicate
            },
            Some(&measurement)
        ));
    }

    #[test]
    fn gwtd_measurement_requires_object_params() {
        let error = validate_measurement_definition(&SymptomMeasurement::GwtdOperation {
            operation: "concern.list".into(),
            params: Value::Null,
        })
        .expect_err("null params are not executable as a JSON operation");
        assert!(error.to_string().contains("params"));
    }
}
