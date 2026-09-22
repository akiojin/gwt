//! Intake Inspection Snapshot and Finding Disposition Ledger
//! (Issue #4541, SPEC-3248 FR-171 / FR-175 / FR-188 / FR-190).
//!
//! An intake agent that is about to finalize an Issue / SPEC / plan / tasks
//! update captures an immutable snapshot of what it is about to hand off, and
//! an independent reviewer inspects that snapshot. Two rules shape this
//! module:
//!
//! - **Lint runs first.** [`IntakeInspectionSnapshot::capture`] takes a
//!   [`LintReport`] by value, so a snapshot cannot exist without one, and
//!   [`load_snapshot`] rejects a persisted snapshot missing its `lint` field.
//!   The reviewer never spends budget rediscovering a numbering gap.
//! - **Machine findings bypass the reviewer.** Lint findings are seeded
//!   straight into the ledger with `consumes_reviewer_budget: false`; only
//!   findings an independent reviewer actually raised count against it.
//!
//! Completion is gated on GitHub-side readback: a local cache re-read is not
//! evidence that the artifact landed, and a section written through a bypass
//! path (manual escaping, a user-run script) has no write receipt at all, so
//! it can only clear the gate with a fresh readback.
//!
//! State is machine-local and repo-scoped, next to the operability ledger:
//! `~/.gwt/projects/<repo-hash>/inspection/issue-<owner>.{snapshot,ledger}.json`.

use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::spec_artifact_lint::{LintFinding, LintReport, LintSeverity};

/// What the deterministic lint found, recorded inside the snapshot so the
/// reviewer and the completion gate read the same run (AC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintOutcome {
    pub ran_at: DateTime<Utc>,
    pub sections_scanned: Vec<String>,
    pub finding_count: usize,
    pub critical_count: usize,
    pub findings: Vec<LintFinding>,
}

impl From<&LintReport> for LintOutcome {
    fn from(report: &LintReport) -> Self {
        Self {
            ran_at: report.ran_at,
            sections_scanned: report.sections_scanned.clone(),
            finding_count: report.findings.len(),
            critical_count: report.critical_count(),
            findings: report.findings.clone(),
        }
    }
}

/// The immutable description of what intake is about to hand off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeInspectionSnapshot {
    pub owner_number: u64,
    /// sha256 per section, in section-name order.
    pub section_hashes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readback_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_slice: Option<String>,
    pub captured_at: DateTime<Utc>,
    /// AC-3: no `#[serde(default)]` — a snapshot without a lint outcome is
    /// not a snapshot, on the wire or in memory.
    pub lint: LintOutcome,
}

impl IntakeInspectionSnapshot {
    /// Capture a snapshot. The [`LintReport`] argument is what makes the lint
    /// a pre-gate rather than a parallel step: there is no constructor that
    /// skips it (AC-3).
    #[must_use]
    pub fn capture(
        owner_number: u64,
        section_hashes: BTreeMap<String, String>,
        lint: &LintReport,
        captured_at: DateTime<Utc>,
    ) -> Self {
        Self {
            owner_number,
            section_hashes,
            readback_refs: Vec::new(),
            directive_epoch: None,
            phase_slice: None,
            captured_at,
            lint: LintOutcome::from(lint),
        }
    }

    #[must_use]
    pub fn with_context(
        mut self,
        directive_epoch: Option<String>,
        phase_slice: Option<String>,
        readback_refs: Vec<String>,
    ) -> Self {
        self.directive_epoch = directive_epoch;
        self.phase_slice = phase_slice;
        self.readback_refs = readback_refs;
        self
    }
}

/// Who raised a finding. Only [`FindingSource::IndependentInspection`]
/// consumes reviewer budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSource {
    MachineLint,
    IndependentInspection,
}

/// The dispositions FR-175 allows, plus the `Pending` state a seeded finding
/// starts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Pending,
    Accepted,
    AcceptedWithModification,
    RejectedWithRationale,
    DeferredWithOwner,
    Duplicate,
}

impl Disposition {
    #[must_use]
    pub fn is_closed(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// One finding and how intake disposed of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: String,
    pub source: FindingSource,
    pub severity: LintSeverity,
    pub section: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    pub message: String,
    pub disposition: Disposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readback_refs: Vec<String>,
    /// AC-4: machine-decidable findings never bill the reviewer.
    pub consumes_reviewer_budget: bool,
}

/// Every finding for one owner, machine and human alike.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingDispositionLedger {
    pub owner_number: u64,
    pub entries: Vec<LedgerEntry>,
    pub updated_at: DateTime<Utc>,
}

impl FindingDispositionLedger {
    /// Seed the ledger from a lint run. Lint findings land here directly —
    /// they are never routed through the independent reviewer (AC-4).
    #[must_use]
    pub fn seed_from_lint(report: &LintReport, updated_at: DateTime<Utc>) -> Self {
        let entries = report
            .findings
            .iter()
            .map(|finding| LedgerEntry {
                id: finding.id.clone(),
                source: FindingSource::MachineLint,
                severity: finding.severity,
                section: finding.section.clone(),
                refs: finding.refs.clone(),
                message: finding.message.clone(),
                disposition: Disposition::Pending,
                rationale: None,
                readback_refs: Vec::new(),
                consumes_reviewer_budget: false,
            })
            .collect();
        Self {
            owner_number: report.owner_number,
            entries,
            updated_at,
        }
    }

    /// The findings the independent reviewer is billed for — machine lint
    /// findings are not among them.
    #[must_use]
    pub fn reviewer_budget_entries(&self) -> Vec<&LedgerEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.consumes_reviewer_budget)
            .collect()
    }

    /// Critical findings still waiting for a disposition. FR-175 blocks
    /// completion while any remain.
    #[must_use]
    pub fn undisposed_critical(&self) -> Vec<&LedgerEntry> {
        self.entries
            .iter()
            .filter(|entry| {
                entry.severity == LintSeverity::Critical && !entry.disposition.is_closed()
            })
            .collect()
    }
}

// --- completion evidence (AC-6) ----------------------------------------

/// Where a readback came from. Only [`ReadbackSource::GitHubEntity`] proves
/// the artifact landed; the local cache can be a stale copy of a write that
/// never reached GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadbackSource {
    GitHubEntity,
    LocalCache,
}

/// One observation of stored content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadbackEvidence {
    pub source: ReadbackSource,
    pub sha256: String,
    pub observed_at: DateTime<Utc>,
    /// Where the observation came from — comment ids, a commit, a pull ref.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
}

/// Completion evidence for one section of the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionCompletion {
    pub section: String,
    /// The section was written outside the verified write path (manual
    /// escaping, a user-run script), so no write receipt exists for it.
    #[serde(default)]
    pub bypass_path: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readback: Option<ReadbackEvidence>,
}

/// The evidence intake presents when it claims the artifact is complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionEvidence {
    pub sections: Vec<SectionCompletion>,
}

/// Why completion evidence is not sufficient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionEvidenceError {
    /// FR-190: no readback at all for a section in the snapshot.
    MissingGitHubReadback { section: String, bypass_path: bool },
    /// FR-190: a local cache re-read is not completion evidence.
    CacheOnlyReadback { section: String, bypass_path: bool },
    /// The stored artifact is not what the snapshot describes.
    ReadbackHashMismatch {
        section: String,
        expected: String,
        observed: String,
    },
    /// A critical finding has no disposition yet (FR-175).
    UndisposedCriticalFinding { id: String, message: String },
}

impl fmt::Display for CompletionEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingGitHubReadback {
                section,
                bypass_path,
            } => {
                write!(
                    formatter,
                    "section '{section}' has no GitHub readback evidence"
                )?;
                if *bypass_path {
                    write!(
                        formatter,
                        "; it was written through a bypass path, so a fresh pull + verify is mandatory"
                    )?;
                }
                Ok(())
            }
            Self::CacheOnlyReadback {
                section,
                bypass_path,
            } => {
                write!(
                    formatter,
                    "section '{section}' is backed only by a local cache re-read, which is not completion evidence"
                )?;
                if *bypass_path {
                    write!(
                        formatter,
                        "; a bypass-path write must be confirmed against the GitHub entity"
                    )?;
                }
                Ok(())
            }
            Self::ReadbackHashMismatch {
                section,
                expected,
                observed,
            } => write!(
                formatter,
                "section '{section}' readback is {observed} but the snapshot recorded {expected}"
            ),
            Self::UndisposedCriticalFinding { id, message } => {
                write!(
                    formatter,
                    "critical finding {id} has no disposition: {message}"
                )
            }
        }
    }
}

/// Decide whether intake may declare the artifact complete.
///
/// Every section the snapshot covers needs a GitHub-entity readback whose
/// hash matches, and every critical finding needs a disposition. A section
/// written through a bypass path is held to the same rule — the flag only
/// sharpens the error message, because a bypass write is exactly the case
/// where no write receipt exists to fall back on.
pub fn validate_completion_evidence(
    snapshot: &IntakeInspectionSnapshot,
    ledger: &FindingDispositionLedger,
    evidence: &CompletionEvidence,
) -> Result<(), Vec<CompletionEvidenceError>> {
    let mut errors = Vec::new();

    for entry in ledger.undisposed_critical() {
        errors.push(CompletionEvidenceError::UndisposedCriticalFinding {
            id: entry.id.clone(),
            message: entry.message.clone(),
        });
    }

    let by_section: BTreeMap<&str, &SectionCompletion> = evidence
        .sections
        .iter()
        .map(|section| (section.section.as_str(), section))
        .collect();

    for (section, expected) in &snapshot.section_hashes {
        let Some(completion) = by_section.get(section.as_str()) else {
            errors.push(CompletionEvidenceError::MissingGitHubReadback {
                section: section.clone(),
                bypass_path: false,
            });
            continue;
        };
        let Some(readback) = completion.readback.as_ref() else {
            errors.push(CompletionEvidenceError::MissingGitHubReadback {
                section: section.clone(),
                bypass_path: completion.bypass_path,
            });
            continue;
        };
        if readback.source == ReadbackSource::LocalCache {
            errors.push(CompletionEvidenceError::CacheOnlyReadback {
                section: section.clone(),
                bypass_path: completion.bypass_path,
            });
            continue;
        }
        if &readback.sha256 != expected {
            errors.push(CompletionEvidenceError::ReadbackHashMismatch {
                section: section.clone(),
                expected: expected.clone(),
                observed: readback.sha256.clone(),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// --- persistence -------------------------------------------------------

/// Directory holding one owner's inspection state, or `None` when the repo
/// hash is unresolvable (a non-git directory).
#[must_use]
pub fn state_dir(repo_path: &Path) -> Option<PathBuf> {
    let repo_hash = crate::index_worker::detect_repo_hash(repo_path)?;
    Some(
        gwt_core::paths::gwt_projects_dir()
            .join(repo_hash.as_str())
            .join("inspection"),
    )
}

#[must_use]
pub fn snapshot_path(repo_path: &Path, owner_number: u64) -> Option<PathBuf> {
    state_dir(repo_path).map(|dir| dir.join(format!("issue-{owner_number}.snapshot.json")))
}

#[must_use]
pub fn ledger_path(repo_path: &Path, owner_number: u64) -> Option<PathBuf> {
    state_dir(repo_path).map(|dir| dir.join(format!("issue-{owner_number}.ledger.json")))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = serde_json::to_string_pretty(value)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    fs::write(path, format!("{encoded}\n"))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<T>(&contents)
            .map(Some)
            .map_err(|err| io::Error::new(ErrorKind::InvalidData, err)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Persist snapshot and ledger together. Returns the two paths.
pub fn save(
    repo_path: &Path,
    snapshot: &IntakeInspectionSnapshot,
    ledger: &FindingDispositionLedger,
) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let (Some(snapshot_path), Some(ledger_path)) = (
        snapshot_path(repo_path, snapshot.owner_number),
        ledger_path(repo_path, snapshot.owner_number),
    ) else {
        return Ok(None);
    };
    write_json(&snapshot_path, snapshot)?;
    write_json(&ledger_path, ledger)?;
    Ok(Some((snapshot_path, ledger_path)))
}

/// Load the owner's snapshot. A stored snapshot with no `lint` field fails to
/// deserialize, which is how AC-3 survives a hand-edited state file.
pub fn load_snapshot(
    repo_path: &Path,
    owner_number: u64,
) -> io::Result<Option<IntakeInspectionSnapshot>> {
    let Some(path) = snapshot_path(repo_path, owner_number) else {
        return Ok(None);
    };
    read_json(&path)
}

pub fn load_ledger(
    repo_path: &Path,
    owner_number: u64,
) -> io::Result<Option<FindingDispositionLedger>> {
    let Some(path) = ledger_path(repo_path, owner_number) else {
        return Ok(None);
    };
    read_json(&path)
}

#[cfg(test)]
mod tests;
