//! Versioned report decisions live in the same journal transaction as their cards.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

const MAX_REVISION: u64 = (1_u64 << 53) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Active,
    Waiting,
    Completed,
    Resolved,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSeverity {
    Routine,
    Important,
    Fatal,
    Blackout,
    NeedsHuman,
    StaleUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportImpact {
    None,
    Local,
    Project,
    Service,
    DataLoss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportActionKind {
    Monitor,
    Retry,
    Respond,
    Decide,
    Intervene,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportAction {
    pub id: String,
    pub kind: ReportActionKind,
    #[serde(default)]
    pub deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialState {
    pub status: ReportStatus,
    pub severity: ReportSeverity,
    pub impact: ReportImpact,
    #[serde(default)]
    pub action: Option<ReportAction>,
    /// Stable milestone identity also names a correction; wording is not state.
    #[serde(default)]
    pub milestone_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportDecision {
    pub question: String,
    pub recommendation: String,
    pub reason: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportSummary {
    pub result: String,
    #[serde(default)]
    pub impact: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub next_action: Option<String>,
    #[serde(default)]
    pub decision: Option<ReportDecision>,
}

impl ReportSummary {
    pub fn render(&self) -> String {
        let mut lines = vec![self.result.clone()];
        for (label, value) in [
            ("Impact", &self.impact),
            ("Owner", &self.owner),
            ("Next", &self.next_action),
        ] {
            if let Some(value) = value {
                lines.push(format!("{label}: {value}"));
            }
        }
        if let Some(decision) = &self.decision {
            lines.push(format!(
                "Decision: {} — {} ({}) [{}]",
                decision.question,
                decision.recommendation,
                decision.reason,
                decision.options.join(" / ")
            ));
        }
        lines.join("\n")
    }

    fn validate(&self) -> Result<()> {
        short_line(&self.result)?;
        for value in [&self.impact, &self.owner, &self.next_action]
            .into_iter()
            .flatten()
        {
            short_line(value)?;
        }
        if let Some(decision) = &self.decision {
            if !(2..=3).contains(&decision.options.len()) {
                return invalid("decision needs 2..3 options");
            }
            for value in [
                &decision.question,
                &decision.recommendation,
                &decision.reason,
            ]
            .into_iter()
            .chain(&decision.options)
            {
                short_line(value)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportEvent {
    pub episode_id: String,
    pub event_id: String,
    pub expected_revision: u64,
    pub kind: PmReportKind,
    pub state: MaterialState,
    pub summary: ReportSummary,
    pub details: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub observed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub schema_version: u32,
    pub request_id: String,
    pub events: Vec<ReportEvent>,
    #[serde(default)]
    pub digest: Option<ReportSummary>,
}

impl Submission {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return invalid("unsupported report submission schema_version");
        }
        identifier(&self.request_id)?;
        if self.events.is_empty() || self.events.len() > 32 {
            return invalid("report submission needs 1..32 events");
        }
        if serde_json::to_vec(self).map_err(invalid_report_data)?.len() > 256 * 1024 {
            return invalid("report submission exceeds 256 KiB");
        }
        if let Some(digest) = &self.digest {
            digest.validate()?;
            if digest.decision.is_some() {
                return invalid("digest cannot hide a decision; put it in an event");
            }
        }
        let mut keys = BTreeSet::new();
        for event in &self.events {
            identifier(&event.episode_id)?;
            identifier(&event.event_id)?;
            if !keys.insert((&event.episode_id, &event.event_id)) {
                return invalid("duplicate report event key in submission");
            }
            if event.expected_revision >= MAX_REVISION {
                return invalid("report revision is out of range");
            }
            if event.kind == PmReportKind::Qa {
                return invalid("Q&A requires joined instruction capture, not a submission");
            }
            event.summary.validate()?;
            if event.details.len() > 64 * 1024 {
                return invalid("report details exceed 64 KiB");
            }
            if event.refs.len() > 16
                || event.refs.iter().any(|value| {
                    value.trim().is_empty()
                        || value.len() > 2048
                        || value
                            .chars()
                            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
                })
            {
                return invalid(
                    "report refs need at most 16 nonempty single-line values of at most 2 KiB",
                );
            }
            if let Some(id) = &event.state.milestone_id {
                identifier(id)?;
            }
            if let Some(action) = &event.state.action {
                identifier(&action.id)?;
            }
            let decision = event.kind == PmReportKind::Decision;
            if decision != event.summary.decision.is_some()
                || decision
                    != event
                        .state
                        .action
                        .as_ref()
                        .is_some_and(|action| action.kind == ReportActionKind::Decide)
            {
                return invalid("decision kind requires a decision packet and a decide action; use them together");
            }
        }
        Ok(())
    }
}

fn short_line(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.chars().count() > 240
        || value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return invalid(
            "summary fields must be nonempty single lines of at most 240 Unicode characters",
        );
    }
    Ok(())
}

fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:/".contains(&c))
    {
        return invalid("report identities require 1..128 ASCII letters, digits or -_.:/");
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T> {
    Err(GwtError::Config(message.into()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredReport {
    pub summary: ReportSummary,
    pub details: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostingReason {
    Unchanged,
    Immediate,
    First,
    Changed,
    Resolved,
    Recurrence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventDecision {
    pub episode_id: String,
    pub event_id: String,
    pub revision: u64,
    pub reason: PostingReason,
    pub report_id: Option<String>,
}

/// Acceptance/suppression is not a read receipt, approval, or completion proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostingReceipt {
    pub schema_version: u32,
    pub request_id: String,
    pub created_at: DateTime<Utc>,
    pub decisions: Vec<EventDecision>,
    pub reports: Vec<PmReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRevision {
    pub episode_id: String,
    pub event_id: String,
    pub revision: u64,
    pub kind: PmReportKind,
    pub state: MaterialState,
    pub decision: Option<ReportDecision>,
}

#[derive(Default)]
pub struct ReportJournal {
    pub reports: Vec<PmReport>,
    pub receipts: Vec<PostingReceipt>,
    states: BTreeMap<(String, String), EventRevision>,
    requests: BTreeMap<String, Submission>,
}

impl ReportJournal {
    pub fn events(&self) -> Vec<&EventRevision> {
        self.states.values().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transaction {
    submission: Submission,
    receipt: PostingReceipt,
}

/// The hash detects damage in exact bytes; it grants no authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalEntry {
    journal_schema: u32,
    payload: String,
    payload_sha256: String,
}

fn closed(status: ReportStatus) -> bool {
    matches!(status, ReportStatus::Completed | ReportStatus::Resolved)
}

/// Pure policy: no clock comparison, provider/session identity, I/O or scheduler.
fn decide_submission(
    input: &Submission,
    states: &BTreeMap<(String, String), EventRevision>,
    now: DateTime<Utc>,
) -> Result<PostingReceipt> {
    input.validate()?;
    let mut decisions = Vec::new();
    let mut urgent = Vec::new();
    let mut routine = Vec::new();
    for (index, event) in input.events.iter().enumerate() {
        let old = states.get(&(event.episode_id.clone(), event.event_id.clone()));
        let revision = old.map_or(0, |old| old.revision);
        if event.expected_revision != revision {
            return invalid(
                "stale report expected_revision; read pm.report.list events before retrying",
            );
        }
        let same = old.is_some_and(|old| {
            old.state == event.state
                && old.kind == event.kind
                && old.decision == event.summary.decision
        });
        let immediate = event.kind == PmReportKind::Decision
            || matches!(
                event.state.severity,
                ReportSeverity::Fatal
                    | ReportSeverity::Blackout
                    | ReportSeverity::NeedsHuman
                    | ReportSeverity::StaleUnknown
            )
            || event.state.action.as_ref().is_some_and(|action| {
                matches!(
                    action.kind,
                    ReportActionKind::Respond
                        | ReportActionKind::Decide
                        | ReportActionKind::Intervene
                )
            });
        let reason = if same {
            PostingReason::Unchanged
        } else if immediate {
            PostingReason::Immediate
        } else if closed(event.state.status) {
            PostingReason::Resolved
        } else if old.is_some_and(|old| closed(old.state.status)) {
            PostingReason::Recurrence
        } else if old.is_none() {
            PostingReason::First
        } else {
            PostingReason::Changed
        };
        if !same {
            if immediate {
                urgent.push(index);
            } else {
                routine.push(index);
            }
        }
        decisions.push(EventDecision {
            episode_id: event.episode_id.clone(),
            event_id: event.event_id.clone(),
            revision: revision + u64::from(!same),
            reason,
            report_id: None,
        });
    }
    if routine.len() > 1 && input.digest.is_none() {
        return invalid("multiple changed routine events require a typed digest summary");
    }
    let mut reports = Vec::new();
    let mut groups: Vec<Vec<usize>> = urgent.into_iter().map(|index| vec![index]).collect();
    if !routine.is_empty() {
        groups.push(routine);
    }
    for group in groups {
        let first = &input.events[group[0]];
        let summary = if group.len() == 1 {
            first.summary.clone()
        } else {
            input.digest.clone().expect("checked digest")
        };
        let details = if group.len() == 1 {
            first.details.clone()
        } else {
            group
                .iter()
                .map(|index| {
                    let event = &input.events[*index];
                    format!("{}\n\n{}", event.summary.render(), event.details)
                })
                .collect::<Vec<_>>()
                .join("\n\n---\n\n")
        };
        let refs = group
            .iter()
            .flat_map(|index| input.events[*index].refs.iter().cloned())
            .collect();
        let id = format!(
            "pm-rpt-{}-{}",
            crate::pm_authorization::digest(input.request_id.as_bytes()),
            reports.len()
        );
        for index in &group {
            decisions[*index].report_id = Some(id.clone());
        }
        let kind = if group.len() == 1 {
            first.kind
        } else if group.iter().any(|index| {
            input.events[*index].kind == PmReportKind::Blocker
                && !closed(input.events[*index].state.status)
        }) {
            PmReportKind::Blocker
        } else {
            PmReportKind::Progress
        };
        reports.push(PmReport {
            id,
            kind,
            question: None,
            body: summary.render(),
            created_at: now,
            structured: Some(StructuredReport {
                summary,
                details,
                refs,
            }),
        });
    }
    Ok(PostingReceipt {
        schema_version: 1,
        request_id: input.request_id.clone(),
        created_at: now,
        decisions,
        reports,
    })
}

fn apply_transaction(journal: &mut ReportJournal, transaction: Transaction) -> Result<()> {
    let input = transaction.submission;
    if journal.requests.contains_key(&input.request_id) {
        return invalid("duplicate PM report request in journal");
    }
    let expected = decide_submission(&input, &journal.states, transaction.receipt.created_at)?;
    if expected != transaction.receipt {
        return invalid("PM report transaction receipt does not match its source decision");
    }
    for (event, decision) in input.events.iter().zip(&transaction.receipt.decisions) {
        if decision.reason == PostingReason::Unchanged {
            continue;
        }
        journal.states.insert(
            (event.episode_id.clone(), event.event_id.clone()),
            EventRevision {
                episode_id: event.episode_id.clone(),
                event_id: event.event_id.clone(),
                revision: decision.revision,
                kind: event.kind,
                state: event.state.clone(),
                decision: event.summary.decision.clone(),
            },
        );
    }
    journal.reports.extend(transaction.receipt.reports.clone());
    journal.receipts.push(transaction.receipt);
    journal.requests.insert(input.request_id.clone(), input);
    Ok(())
}

pub(super) fn read_journal(file: &mut File, path: &Path) -> Result<ReportJournal> {
    check_tail(file)?;
    file.seek(SeekFrom::Start(0))?;
    let mut journal = ReportJournal::default();
    let mut ids = BTreeSet::new();
    for (index, line) in BufReader::new(file).split(b'\n').enumerate() {
        let line = line?;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(&line).map_err(|error| {
            invalid_report_data(format!("{} line {}: {error}", path.display(), index + 1))
        })?;
        let before = journal.reports.len();
        if value.get("journal_schema").is_some() {
            let entry: JournalEntry = serde_json::from_slice(&line).map_err(invalid_report_data)?;
            if entry.journal_schema != 2
                || crate::pm_authorization::digest(entry.payload.as_bytes()) != entry.payload_sha256
            {
                return invalid("unsupported or damaged PM report journal entry; preserve bytes and use a compatible binary");
            }
            apply_transaction(
                &mut journal,
                serde_json::from_str(&entry.payload).map_err(invalid_report_data)?,
            )?;
        } else {
            journal
                .reports
                .push(serde_json::from_slice(&line).map_err(invalid_report_data)?);
        }
        for report in &journal.reports[before..] {
            if !ids.insert(report.id.clone()) {
                return invalid("duplicate report identity in journal");
            }
        }
    }
    Ok(journal)
}

fn check_tail(file: &mut File) -> Result<()> {
    if file.metadata()?.len() != 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut byte = [0];
        file.read_exact(&mut byte)?;
        if byte != [b'\n'] {
            return invalid("incomplete PM report record (missing newline); preserve journal and recover explicitly");
        }
    }
    Ok(())
}

pub fn load_report_journal(repo: &Path) -> Result<ReportJournal> {
    let path = reports_path(repo);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ReportJournal::default())
        }
        Err(error) => return Err(error.into()),
    };
    FileExt::lock_shared(&file)?;
    check_tail(&mut file)?;
    read_journal(&mut file, &path)
}

pub fn post_submission(repo: &Path, input: &Submission) -> Result<PostingReceipt> {
    input.validate()?;
    let path = reports_path(repo);
    fs::create_dir_all(path.parent().expect("report store parent"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    file.lock_exclusive()?;
    check_tail(&mut file)?;
    let journal = read_journal(&mut file, &path)?;
    if let Some(previous) = journal.requests.get(&input.request_id) {
        if previous != input {
            return invalid("report request_id was already used with a different payload");
        }
        // Also commit a complete line left by process death before its fsync.
        file.sync_all()?;
        sync_report_parent(&path)?;
        return Ok(journal
            .receipts
            .into_iter()
            .find(|receipt| receipt.request_id == input.request_id)
            .expect("validated receipt"));
    }
    let receipt = decide_submission(input, &journal.states, Utc::now())?;
    let transaction = Transaction {
        submission: input.clone(),
        receipt: receipt.clone(),
    };
    let payload = serde_json::to_string(&transaction).map_err(invalid_report_data)?;
    let entry = JournalEntry {
        journal_schema: 2,
        payload_sha256: crate::pm_authorization::digest(payload.as_bytes()),
        payload,
    };
    append_record(&mut file, &path, &entry, |file, encoded| {
        file.write_all(encoded)
    })?;
    Ok(receipt)
}
