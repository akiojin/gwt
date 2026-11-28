//! Project-wide decision queue owned by gwtd (Issue #7742).
//!
//! The queue lives in the machine-local project store, never in a PM session:
//! a reassigned or stopped PM leaves every waiting decision and its count
//! untouched. Agents enter decisions through the typed registration below,
//! which is the only entrance for them; decisions the PM raised through its
//! reports are bridged in so no waiting decision is lost.

use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::{
    pm_report::{self, PmReportKind, ReportStatus},
    timestamp::UtcTimestamp,
    GwtError, Result,
};

const SCHEMA_VERSION: u32 = 1;
const MAX_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Product,
    Scope,
    Technical,
    Process,
    /// A decision the PM raised to the user through a report. Bridged in by
    /// [`sync_pm_report_decisions`]; agents cannot register it.
    PmDecision,
}

impl DecisionKind {
    /// Kinds an agent may register.
    pub fn parse_registrable(value: &str) -> Option<Self> {
        match value {
            "product" => Some(Self::Product),
            "scope" => Some(Self::Scope),
            "technical" => Some(Self::Technical),
            "process" => Some(Self::Process),
            _ => None,
        }
    }

    /// Only product and scope decisions reach the user; the PM closes the rest.
    pub fn reaches_user(self) -> bool {
        matches!(self, Self::Product | Self::Scope | Self::PmDecision)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Open,
    Answered,
    /// Bundled into another decision; its waiters resume with that answer.
    Merged,
    /// The PM report behind a bridged decision was resolved or superseded.
    Withdrawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredBy {
    User,
    Pm,
}

/// Who resumes when the decision is answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Waiter {
    Session {
        session_id: String,
    },
    /// Whichever PM is registered when the answer is delivered.
    Pm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionAnswer {
    pub text: String,
    pub by: AnsweredBy,
    pub answered_at: UtcTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub waiter: Waiter,
    #[serde(default)]
    pub delivered_at: Option<UtcTimestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionItem {
    pub id: String,
    #[serde(default)]
    pub issue: Option<u64>,
    pub kind: DecisionKind,
    pub target: String,
    pub question: String,
    pub materials: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub recommendation: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub impact: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub detail_text: Option<String>,
    #[serde(default)]
    pub source_report_id: Option<String>,
    #[serde(default)]
    pub waiters: Vec<Waiter>,
    pub status: DecisionStatus,
    #[serde(default)]
    pub answer: Option<DecisionAnswer>,
    #[serde(default)]
    pub merged_into: Option<String>,
    /// Set by "decide later": deferred decisions keep waiting at the tail.
    #[serde(default)]
    pub deferred_seq: Option<u64>,
    pub created_at: UtcTimestamp,
    #[serde(default)]
    pub deliveries: Vec<Delivery>,
}

impl DecisionItem {
    fn waits_for_user(&self) -> bool {
        self.status == DecisionStatus::Open && self.kind.reaches_user()
    }
}

/// What an agent submits. Every field the kind needs must be present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registration {
    pub issue: Option<u64>,
    pub kind: Option<String>,
    pub target: Option<String>,
    pub question: Option<String>,
    pub materials: Option<String>,
    pub options: Vec<String>,
    pub recommendation: Option<String>,
    pub reason: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegisterOutcome {
    /// `false` when an open decision with the same issue, kind and target
    /// already existed: that one is returned and the caller waits on it.
    pub created: bool,
    pub item: DecisionItem,
}

/// The one projection every surface reads: header, status bar and the
/// waiting-decisions screen all take `waiting_count` from here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub waiting_count: usize,
    pub items: Vec<DecisionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDelivery {
    pub decision_id: String,
    pub waiter: Waiter,
    pub message: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct QueueFile {
    schema_version: u32,
    #[serde(default)]
    next_seq: u64,
    #[serde(default)]
    items: Vec<DecisionItem>,
}

pub fn queue_path(repo_path: &Path) -> PathBuf {
    crate::paths::gwt_project_dir_for_repo_path(repo_path).join("project-state/decision-queue.json")
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(GwtError::Config(message.into()))
}

fn read_queue(path: &Path) -> Result<QueueFile> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(QueueFile {
                schema_version: SCHEMA_VERSION,
                ..Default::default()
            })
        }
        Err(error) => return Err(error.into()),
    };
    let queue: QueueFile = serde_json::from_slice(&bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("decision queue {} is unreadable: {error}", path.display()),
        )
    })?;
    if queue.schema_version != SCHEMA_VERSION {
        return invalid(format!(
            "decision queue schema {} is not supported by this binary",
            queue.schema_version
        ));
    }
    Ok(queue)
}

/// Run one read-modify-write under the queue's exclusive lock. The file is
/// rewritten only when `change` reports a modification.
fn update<T>(
    repo_path: &Path,
    change: impl FnOnce(&mut QueueFile) -> Result<(T, bool)>,
) -> Result<T> {
    let path = queue_path(repo_path);
    let parent = path.parent().expect("decision queue has a parent");
    fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join("decision-queue.lock"))?;
    lock.lock_exclusive()?;
    let mut queue = read_queue(&path)?;
    let (value, changed) = change(&mut queue)?;
    if changed {
        let bytes = serde_json::to_vec_pretty(&queue)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        crate::atomic_file::write_atomic(&path, &bytes)?;
    }
    Ok(value)
}

fn required_text(name: &str, value: Option<&str>, missing: &mut Vec<String>) -> Result<String> {
    let text = value.map(str::trim).unwrap_or_default();
    if text.is_empty() {
        missing.push(name.to_string());
    }
    if text.len() > MAX_TEXT_BYTES {
        return invalid(format!("decision {name} must not exceed 16 KiB"));
    }
    Ok(text.to_string())
}

fn optional_text(name: &str, value: Option<&str>) -> Result<Option<String>> {
    let text = value.map(str::trim).filter(|text| !text.is_empty());
    if text.is_some_and(|text| text.len() > MAX_TEXT_BYTES) {
        return invalid(format!("decision {name} must not exceed 16 KiB"));
    }
    Ok(text.map(str::to_owned))
}

fn target_key(target: &str) -> String {
    target
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn add_waiter(item: &mut DecisionItem, waiter: Waiter) -> bool {
    if item.waiters.contains(&waiter) {
        return false;
    }
    item.waiters.push(waiter);
    true
}

fn new_item(
    kind: DecisionKind,
    target: String,
    question: String,
    materials: String,
) -> DecisionItem {
    DecisionItem {
        id: uuid::Uuid::new_v4().to_string(),
        issue: None,
        kind,
        target,
        question,
        materials,
        options: Vec::new(),
        recommendation: None,
        reason: None,
        impact: None,
        owner: None,
        detail_text: None,
        source_report_id: None,
        waiters: Vec::new(),
        status: DecisionStatus::Open,
        answer: None,
        merged_into: None,
        deferred_seq: None,
        created_at: UtcTimestamp::now(),
        deliveries: Vec::new(),
    }
}

/// Register a decision. A registration missing any field its kind needs is
/// refused; an open decision with the same issue, kind and target is returned
/// instead of a second one, with the caller attached to it.
pub fn register(repo_path: &Path, input: &Registration) -> Result<RegisterOutcome> {
    let mut missing = Vec::new();
    let issue = input.issue.filter(|number| *number > 0);
    if issue.is_none() {
        missing.push("issue".to_string());
    }
    let kind = match input.kind.as_deref().map(str::trim) {
        None | Some("") => {
            missing.push("kind".to_string());
            None
        }
        Some(value) => match DecisionKind::parse_registrable(value) {
            Some(kind) => Some(kind),
            None => {
                return invalid(format!(
                "decision kind `{value}` is not registrable; use product, scope, technical or process"
            ))
            }
        },
    };
    let target = required_text("target", input.target.as_deref(), &mut missing)?;
    let question = required_text("question", input.question.as_deref(), &mut missing)?;
    let materials = required_text("materials", input.materials.as_deref(), &mut missing)?;
    let session_id = required_text("session_id", input.session_id.as_deref(), &mut missing)?;
    let recommendation = optional_text("recommendation", input.recommendation.as_deref())?;
    let reason = optional_text("reason", input.reason.as_deref())?;
    let mut options = Vec::new();
    for option in &input.options {
        if let Some(option) = optional_text("option", Some(option))? {
            options.push(option);
        }
    }
    if kind.is_some_and(DecisionKind::reaches_user) {
        // The user decides from the card alone, so it must carry the choices.
        if options.len() < 2 {
            missing.push("options (at least two)".to_string());
        }
        if recommendation.is_none() {
            missing.push("recommendation".to_string());
        }
    }
    if !missing.is_empty() {
        return invalid(format!(
            "decision registration is missing required fields: {}",
            missing.join(", ")
        ));
    }
    let (issue, kind) = (issue.expect("validated"), kind.expect("validated"));
    let waiter = Waiter::Session { session_id };
    update(repo_path, |queue| {
        let key = target_key(&target);
        if let Some(existing) = queue.items.iter_mut().find(|item| {
            item.status == DecisionStatus::Open
                && item.issue == Some(issue)
                && item.kind == kind
                && target_key(&item.target) == key
        }) {
            let changed = add_waiter(existing, waiter);
            return Ok((
                RegisterOutcome {
                    created: false,
                    item: existing.clone(),
                },
                changed,
            ));
        }
        let mut item = new_item(kind, target, question, materials);
        item.issue = Some(issue);
        item.options = options;
        item.recommendation = recommendation;
        item.reason = reason;
        item.waiters.push(waiter);
        queue.items.push(item.clone());
        Ok((
            RegisterOutcome {
                created: true,
                item,
            },
            true,
        ))
    })
}

/// Options of the keep-or-undo card a PM operation leaves (Issue #8083).
pub const PM_RATIFICATION_OPTIONS: [&str; 2] = [
    "Keep — ratify what the PM did",
    "Undo — the PM rolls the change back",
];

/// A decision gwtd raises for the user on its own behalf (Issue #8083):
/// something that would otherwise stop work waits here instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerDecision<'a> {
    pub target: &'a str,
    pub question: &'a str,
    pub materials: &'a str,
    pub options: &'a [&'a str],
    pub reason: &'a str,
}

/// Register an [`OwnerDecision`] answered by the user and delivered to the
/// registered PM. An open card with the same target is reused; with
/// `append` its materials gain the new entry (newest last, oldest dropped
/// past 16 KiB), otherwise they are left as they are.
pub fn register_owner_decision(
    repo_path: &Path,
    decision: &OwnerDecision<'_>,
    append: bool,
) -> Result<DecisionItem> {
    let target = decision.target.trim();
    let materials = decision.materials.trim();
    if target.is_empty() || decision.question.trim().is_empty() || materials.is_empty() {
        return invalid("an owner decision requires a target, question and materials");
    }
    if decision.options.len() < 2 {
        return invalid("an owner decision requires at least two options");
    }
    update(repo_path, |queue| {
        let key = target_key(target);
        if let Some(existing) = queue.items.iter_mut().find(|item| {
            item.status == DecisionStatus::Open
                && item.kind == DecisionKind::PmDecision
                && target_key(&item.target) == key
        }) {
            if !append {
                return Ok((existing.clone(), false));
            }
            existing.materials = keep_tail(format!("{}\n{materials}", existing.materials));
            return Ok((existing.clone(), true));
        }
        let mut item = new_item(
            DecisionKind::PmDecision,
            target.to_string(),
            decision.question.trim().to_string(),
            keep_tail(materials.to_string()),
        );
        item.options = decision
            .options
            .iter()
            .map(|option| (*option).to_owned())
            .collect();
        item.recommendation = Some(decision.options[0].to_string());
        item.reason = Some(decision.reason.to_string());
        item.waiters.push(Waiter::Pm);
        queue.items.push(item.clone());
        Ok((item, true))
    })
}

/// The newest decision for `target`, following bundling to its answer.
pub fn latest_for_target(repo_path: &Path, target: &str) -> Result<Option<DecisionItem>> {
    let queue = read_queue(&queue_path(repo_path))?;
    let key = target_key(target);
    let Some(item) = queue
        .items
        .iter()
        .rev()
        .find(|item| target_key(&item.target) == key)
    else {
        return Ok(None);
    };
    Ok(Some(queue.items[resolve_merged(&queue, &item.id)?].clone()))
}

/// Leave an autonomous PM operation for the user to keep or undo afterwards.
///
/// The PM ran the operation without stopping for permission; this card is
/// where the user decides on it later. One open card per operation kind
/// collects every run, and the answer reaches whichever PM is registered.
pub fn register_pm_ratification(
    repo_path: &Path,
    operation: &str,
    entry: &str,
) -> Result<DecisionItem> {
    let operation = operation.trim();
    if operation.is_empty() {
        return invalid("PM ratification requires an operation");
    }
    register_owner_decision(
        repo_path,
        &OwnerDecision {
            target: &format!("pm-operation:{operation}"),
            question: &format!("Keep or undo the PM's `{operation}` changes?"),
            materials: entry,
            options: &PM_RATIFICATION_OPTIONS,
            reason: "The PM already ran this without waiting; CI, branch protection and ownership gates still applied.",
        },
        true,
    )
}

fn keep_tail(mut text: String) -> String {
    while text.len() > MAX_TEXT_BYTES {
        match text.find('\n') {
            Some(end) => {
                text.drain(..=end);
            }
            None => {
                let mut start = text.len() - MAX_TEXT_BYTES;
                while !text.is_char_boundary(start) {
                    start += 1;
                }
                text.drain(..start);
            }
        }
    }
    text
}

fn open_index(queue: &QueueFile, id: &str) -> Result<usize> {
    let Some(index) = queue.items.iter().position(|item| item.id == id) else {
        return invalid(format!("decision {id} was not found"));
    };
    match queue.items[index].status {
        DecisionStatus::Open => Ok(index),
        status => invalid(format!(
            "decision {id} is no longer open ({})",
            serde_json::to_value(status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default()
        )),
    }
}

/// Follow bundling to the decision that actually carries the answer.
fn resolve_merged(queue: &QueueFile, id: &str) -> Result<usize> {
    let mut current = id.to_string();
    for _ in 0..=queue.items.len() {
        let Some(index) = queue.items.iter().position(|item| item.id == current) else {
            return invalid(format!("decision {current} was not found"));
        };
        match &queue.items[index].merged_into {
            Some(next) if queue.items[index].status == DecisionStatus::Merged => {
                current = next.clone()
            }
            _ => return Ok(index),
        }
    }
    invalid(format!("decision {id} has a cyclic merge chain"))
}

/// Wait on an existing decision instead of registering the same topic again.
/// An already answered decision is returned as-is so the caller reads its answer.
pub fn attach(repo_path: &Path, id: &str, session_id: &str) -> Result<DecisionItem> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return invalid("decision attach requires a session identity");
    }
    update(repo_path, |queue| {
        let index = resolve_merged(queue, id)?;
        let item = &mut queue.items[index];
        let changed = item.status == DecisionStatus::Open
            && add_waiter(
                item,
                Waiter::Session {
                    session_id: session_id.to_string(),
                },
            );
        Ok((item.clone(), changed))
    })
}

/// Bundle decisions that differ only in wording. Sessions waiting on the
/// bundled ones resume together with the answer to `into`.
pub fn merge(repo_path: &Path, into: &str, ids: &[String]) -> Result<DecisionItem> {
    if ids.is_empty() {
        return invalid("decision merge requires at least one decision to bundle");
    }
    update(repo_path, |queue| {
        let target = open_index(queue, into)?;
        let mut moved = Vec::new();
        for id in ids {
            if id == into {
                return invalid("a decision cannot be merged into itself");
            }
            let index = open_index(queue, id)?;
            let item = &mut queue.items[index];
            item.status = DecisionStatus::Merged;
            item.merged_into = Some(into.to_string());
            moved.append(&mut item.waiters);
        }
        for waiter in moved {
            add_waiter(&mut queue.items[target], waiter);
        }
        Ok((queue.items[target].clone(), true))
    })
}

/// Record the answer. Every waiter gets a pending delivery, which gwtd sends
/// straight to the waiting session — never through the PM.
pub fn answer(repo_path: &Path, id: &str, text: &str, by: AnsweredBy) -> Result<DecisionItem> {
    let Some(text) = optional_text("answer", Some(text))? else {
        return invalid("decision answer must be nonempty");
    };
    update(repo_path, |queue| {
        let index = open_index(queue, id)?;
        let item = &mut queue.items[index];
        item.status = DecisionStatus::Answered;
        item.answer = Some(DecisionAnswer {
            text,
            by,
            answered_at: UtcTimestamp::now(),
        });
        item.deliveries = item
            .waiters
            .iter()
            .cloned()
            .map(|waiter| Delivery {
                waiter,
                delivered_at: None,
            })
            .collect();
        Ok((item.clone(), true))
    })
}

/// "Decide later": the decision keeps waiting and moves to the queue's tail.
pub fn defer(repo_path: &Path, id: &str) -> Result<DecisionItem> {
    update(repo_path, |queue| {
        let index = open_index(queue, id)?;
        queue.next_seq += 1;
        queue.items[index].deferred_seq = Some(queue.next_seq);
        Ok((queue.items[index].clone(), true))
    })
}

pub fn delivery_message(item: &DecisionItem, waiter: &Waiter) -> String {
    let answer = item.answer.as_ref();
    let text = answer
        .map(|answer| answer.text.as_str())
        .unwrap_or_default();
    if *waiter == Waiter::Pm {
        return format!("Answer to \"{}\": {text}", item.question);
    }
    let by = match answer.map(|answer| answer.by) {
        Some(AnsweredBy::Pm) => "the PM",
        _ => "the user",
    };
    let issue = item
        .issue
        .map(|number| format!("Issue #{number}, "))
        .unwrap_or_default();
    format!(
        "[gwt decision queue] The decision you were waiting for was answered by {by}.\n\
         Decision {} ({issue}{}).\n\
         Question: {}\n\
         Answer: {text}\n\
         Continue the work with this answer.",
        item.id, item.target, item.question
    )
}

/// Answers that have not reached their waiter yet.
pub fn pending_deliveries(repo_path: &Path) -> Result<Vec<PendingDelivery>> {
    let queue = read_queue(&queue_path(repo_path))?;
    Ok(queue
        .items
        .iter()
        .filter(|item| item.status == DecisionStatus::Answered)
        .flat_map(|item| {
            item.deliveries
                .iter()
                .filter(|delivery| delivery.delivered_at.is_none())
                .map(|delivery| PendingDelivery {
                    decision_id: item.id.clone(),
                    waiter: delivery.waiter.clone(),
                    message: delivery_message(item, &delivery.waiter),
                })
        })
        .collect())
}

pub fn mark_delivered(repo_path: &Path, id: &str, waiter: &Waiter) -> Result<()> {
    update(repo_path, |queue| {
        let mut changed = false;
        for delivery in queue
            .items
            .iter_mut()
            .filter(|item| item.id == id)
            .flat_map(|item| item.deliveries.iter_mut())
            .filter(|delivery| delivery.waiter == *waiter && delivery.delivered_at.is_none())
        {
            delivery.delivered_at = Some(UtcTimestamp::now());
            changed = true;
        }
        Ok(((), changed))
    })
}

/// The single place the waiting count is computed.
fn snapshot_of(queue: &QueueFile) -> QueueSnapshot {
    let mut fresh: Vec<(usize, &DecisionItem)> = queue
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.waits_for_user())
        .collect();
    // Newest first; a deferred decision keeps waiting behind all of them.
    fresh.sort_by_key(|(index, item)| {
        (
            item.deferred_seq.unwrap_or(0),
            std::cmp::Reverse((item.created_at, *index)),
        )
    });
    let items: Vec<DecisionItem> = fresh.into_iter().map(|(_, item)| item.clone()).collect();
    QueueSnapshot {
        waiting_count: items.len(),
        items,
    }
}

pub fn snapshot(repo_path: &Path) -> Result<QueueSnapshot> {
    Ok(snapshot_of(&read_queue(&queue_path(repo_path))?))
}

/// Decisions for one issue (or all), so an agent checks before registering.
pub fn list(
    repo_path: &Path,
    issue: Option<u64>,
    include_closed: bool,
) -> Result<Vec<DecisionItem>> {
    Ok(read_queue(&queue_path(repo_path))?
        .items
        .into_iter()
        .filter(|item| issue.is_none() || item.issue == issue)
        .filter(|item| include_closed || item.status == DecisionStatus::Open)
        .collect())
}

/// Bridge decisions the PM raised through its reports into the queue.
///
/// Idempotent: a report is imported once, keyed by its id. A report whose
/// event was later resolved, or replaced by a newer revision, withdraws its
/// still-open queue entry.
pub fn sync_pm_report_decisions(repo_path: &Path) -> Result<()> {
    let journal = pm_report::load_report_journal(repo_path)?;
    let mut waiting = Vec::new();
    let mut retired = Vec::new();
    let mut linked = std::collections::BTreeMap::new();
    for receipt in &journal.receipts {
        for decision in &receipt.decisions {
            if let Some(report_id) = &decision.report_id {
                linked.insert(
                    report_id.clone(),
                    (decision.episode_id.clone(), decision.event_id.clone()),
                );
            }
        }
    }
    let mut latest = std::collections::BTreeMap::new();
    for report in &journal.reports {
        if let Some(event) = linked.get(&report.id) {
            latest.insert(event.clone(), report.id.clone());
        }
    }
    let events = journal.events();
    for report in journal
        .reports
        .iter()
        .filter(|report| report.kind == PmReportKind::Decision)
    {
        let current = match linked.get(&report.id) {
            None => true,
            Some(event) => {
                latest.get(event) == Some(&report.id)
                    && events.iter().any(|revision| {
                        (&revision.episode_id, &revision.event_id) == (&event.0, &event.1)
                            && revision.kind == PmReportKind::Decision
                            && !matches!(
                                revision.state.status,
                                ReportStatus::Completed | ReportStatus::Resolved
                            )
                    })
            }
        };
        if current {
            waiting.push(report);
        } else {
            retired.push(report.id.as_str());
        }
    }
    if waiting.is_empty() && retired.is_empty() && !queue_path(repo_path).exists() {
        return Ok(());
    }
    update(repo_path, |queue| {
        let mut changed = false;
        for item in queue.items.iter_mut().filter(|item| {
            item.status == DecisionStatus::Open
                && item
                    .source_report_id
                    .as_deref()
                    .is_some_and(|id| retired.contains(&id))
        }) {
            item.status = DecisionStatus::Withdrawn;
            changed = true;
        }
        for report in waiting {
            if queue
                .items
                .iter()
                .any(|item| item.source_report_id.as_deref() == Some(report.id.as_str()))
            {
                continue;
            }
            let summary = report.structured.as_ref().map(|report| &report.summary);
            let packet = summary.and_then(|summary| summary.decision.as_ref());
            let first_line = report.body.lines().next().unwrap_or_default().trim();
            let question = packet
                .map(|packet| packet.question.clone())
                .or_else(|| summary.map(|summary| summary.result.clone()))
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| {
                    if first_line.is_empty() {
                        "Decision needed".to_string()
                    } else {
                        first_line.to_string()
                    }
                });
            let mut item = new_item(
                DecisionKind::PmDecision,
                format!("pm-report:{}", report.id),
                question,
                summary
                    .map(|summary| summary.result.clone())
                    .unwrap_or_default(),
            );
            item.created_at = report.created_at;
            item.source_report_id = Some(report.id.clone());
            item.impact = summary.and_then(|summary| summary.impact.clone());
            item.owner = summary.and_then(|summary| summary.owner.clone());
            match packet {
                Some(packet) => {
                    item.options = packet.options.clone();
                    item.recommendation = Some(packet.recommendation.clone());
                    item.reason = Some(packet.reason.clone());
                }
                // An untyped report has no packet: keep its text for the card.
                None => item.detail_text = Some(report.body.clone()),
            }
            item.waiters.push(Waiter::Pm);
            queue.items.push(item);
            changed = true;
        }
        Ok(((), changed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedGwtHome;

    fn registration(issue: u64, kind: &str, target: &str, session: &str) -> Registration {
        Registration {
            issue: Some(issue),
            kind: Some(kind.into()),
            target: Some(target.into()),
            question: Some("Which storage should the queue use?".into()),
            materials: Some("Both options pass the existing tests.".into()),
            options: vec!["Keep JSON".into(), "Move to SQLite".into()],
            recommendation: Some("Keep JSON".into()),
            reason: Some("Smallest change".into()),
            session_id: Some(session.into()),
        }
    }

    fn scoped() -> (tempfile::TempDir, ScopedGwtHome, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let guard = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        (home, guard, repo)
    }

    #[test]
    fn registration_missing_required_fields_is_refused_and_writes_nothing() {
        let (_home, _guard, repo) = scoped();
        let error = register(&repo, &Registration::default())
            .unwrap_err()
            .to_string();
        for field in [
            "issue",
            "kind",
            "target",
            "question",
            "materials",
            "session_id",
        ] {
            assert!(error.contains(field), "{field} missing from: {error}");
        }
        // A user-facing kind must carry the choices the user decides from.
        let mut product = registration(7742, "product", "storage", "s1");
        product.options.clear();
        product.recommendation = None;
        let error = register(&repo, &product).unwrap_err().to_string();
        assert!(
            error.contains("options") && error.contains("recommendation"),
            "{error}"
        );
        // The PM-raised kind is not an entrance for agents.
        let error = register(&repo, &registration(7742, "pm_decision", "storage", "s1"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("not registrable"), "{error}");
        let mut blank = registration(7742, "technical", "storage", "s1");
        blank.question = Some("  \n ".into());
        assert!(register(&repo, &blank).is_err());
        assert!(!queue_path(&repo).exists());
        assert_eq!(snapshot(&repo).unwrap(), QueueSnapshot::default());
    }

    #[test]
    fn technical_registration_needs_no_options() {
        let (_home, _guard, repo) = scoped();
        let mut input = registration(7742, "technical", "storage", "s1");
        input.options.clear();
        input.recommendation = None;
        let outcome = register(&repo, &input).unwrap();
        assert!(outcome.created);
        // The PM closes it; it never counts toward the user's waiting decisions.
        assert_eq!(snapshot(&repo).unwrap().waiting_count, 0);
        assert_eq!(list(&repo, Some(7742), false).unwrap(), [outcome.item]);
    }

    #[test]
    fn same_issue_kind_and_target_returns_the_existing_decision() {
        let (_home, _guard, repo) = scoped();
        let first = register(&repo, &registration(7742, "product", "Queue storage", "s1")).unwrap();
        let second = register(
            &repo,
            &registration(7742, "product", "  queue   STORAGE ", "s2"),
        )
        .unwrap();
        assert!(first.created);
        assert!(!second.created);
        assert_eq!(second.item.id, first.item.id);
        // The second caller waits on the existing decision.
        assert_eq!(second.item.waiters.len(), 2);
        assert_eq!(snapshot(&repo).unwrap().waiting_count, 1);
        // A different kind, target or issue is a different decision.
        for other in [
            registration(7742, "scope", "Queue storage", "s1"),
            registration(7742, "product", "Queue order", "s1"),
            registration(7743, "product", "Queue storage", "s1"),
        ] {
            assert!(register(&repo, &other).unwrap().created);
        }
        assert_eq!(snapshot(&repo).unwrap().waiting_count, 4);
        assert_eq!(list(&repo, Some(7742), false).unwrap().len(), 3);
    }

    #[test]
    fn answer_creates_a_pending_delivery_for_every_waiter_until_delivered() {
        let (_home, _guard, repo) = scoped();
        let item = register(&repo, &registration(7742, "product", "storage", "s1"))
            .unwrap()
            .item;
        attach(&repo, &item.id, "s2").unwrap();
        attach(&repo, &item.id, "s2").unwrap();
        assert!(pending_deliveries(&repo).unwrap().is_empty());
        assert!(answer(&repo, &item.id, "  ", AnsweredBy::User).is_err());
        let answered = answer(&repo, &item.id, "1. Keep JSON", AnsweredBy::User).unwrap();
        assert_eq!(answered.status, DecisionStatus::Answered);
        assert_eq!(snapshot(&repo).unwrap().waiting_count, 0);
        let pending = pending_deliveries(&repo).unwrap();
        assert_eq!(pending.len(), 2);
        assert!(pending[0].message.contains("1. Keep JSON"));
        assert!(pending[0].message.contains("answered by the user"));
        assert!(pending[0].message.contains("Issue #7742"));
        mark_delivered(&repo, &item.id, &pending[0].waiter).unwrap();
        let remaining = pending_deliveries(&repo).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].waiter,
            Waiter::Session {
                session_id: "s2".into()
            }
        );
        // An answered decision cannot be answered or deferred again.
        assert!(answer(&repo, &item.id, "again", AnsweredBy::Pm).is_err());
        assert!(defer(&repo, &item.id).is_err());
        // A late attach reads the recorded answer instead of waiting forever.
        let late = attach(&repo, &item.id, "s3").unwrap();
        assert_eq!(late.answer.unwrap().text, "1. Keep JSON");
        assert_eq!(late.waiters.len(), 2);
    }

    #[test]
    fn merged_decisions_resume_their_waiters_with_the_target_answer() {
        let (_home, _guard, repo) = scoped();
        let kept = register(&repo, &registration(7742, "scope", "storage", "s1"))
            .unwrap()
            .item;
        let bundled = register(&repo, &registration(7742, "scope", "persistence", "s2"))
            .unwrap()
            .item;
        assert!(merge(&repo, &kept.id, &[]).is_err());
        assert!(merge(&repo, &kept.id, std::slice::from_ref(&kept.id)).is_err());
        assert!(merge(&repo, &kept.id, &["missing".into()]).is_err());
        let merged = merge(&repo, &kept.id, std::slice::from_ref(&bundled.id)).unwrap();
        assert_eq!(merged.waiters.len(), 2);
        assert_eq!(snapshot(&repo).unwrap().waiting_count, 1);
        // Attaching to the bundled decision lands on the one that is answered.
        assert_eq!(attach(&repo, &bundled.id, "s3").unwrap().id, kept.id);
        answer(&repo, &kept.id, "Keep it", AnsweredBy::Pm).unwrap();
        let pending = pending_deliveries(&repo).unwrap();
        assert_eq!(pending.len(), 3);
        assert!(pending
            .iter()
            .all(|delivery| delivery.decision_id == kept.id));
        assert!(pending[0].message.contains("answered by the PM"));
        assert!(answer(&repo, &bundled.id, "late", AnsweredBy::User)
            .unwrap_err()
            .to_string()
            .contains("merged"));
    }

    #[test]
    fn deferring_moves_the_decision_to_the_tail_without_changing_the_count() {
        let (_home, _guard, repo) = scoped();
        let ids: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|target| {
                register(&repo, &registration(7742, "product", target, "s1"))
                    .unwrap()
                    .item
                    .id
            })
            .collect();
        let order = |repo: &Path| -> Vec<String> {
            snapshot(repo)
                .unwrap()
                .items
                .into_iter()
                .map(|item| item.id)
                .collect()
        };
        // Newest first.
        assert_eq!(
            order(&repo),
            [ids[2].as_str(), ids[1].as_str(), ids[0].as_str()]
        );
        defer(&repo, &ids[2]).unwrap();
        assert_eq!(
            order(&repo),
            [ids[1].as_str(), ids[0].as_str(), ids[2].as_str()]
        );
        defer(&repo, &ids[1]).unwrap();
        assert_eq!(
            order(&repo),
            [ids[0].as_str(), ids[2].as_str(), ids[1].as_str()]
        );
        let queue = snapshot(&repo).unwrap();
        assert_eq!(queue.waiting_count, 3);
        assert!(queue.items[1].deferred_seq.is_some());
        assert!(defer(&repo, "missing").is_err());
    }

    #[test]
    fn unanswered_pm_report_decisions_are_bridged_once_and_stay_answerable() {
        let (_home, _guard, repo) = scoped();
        sync_pm_report_decisions(&repo).unwrap();
        assert!(!queue_path(&repo).exists());
        let legacy = pm_report::post_report(
            &repo,
            PmReportKind::Decision,
            "Keep the existing terminal?\nDetails follow.",
            None,
        )
        .unwrap();
        pm_report::post_report(&repo, PmReportKind::Progress, "Not a decision", None).unwrap();
        sync_pm_report_decisions(&repo).unwrap();
        sync_pm_report_decisions(&repo).unwrap();
        let queue = snapshot(&repo).unwrap();
        assert_eq!(queue.waiting_count, 1);
        let item = &queue.items[0];
        assert_eq!(item.kind, DecisionKind::PmDecision);
        assert_eq!(item.source_report_id.as_deref(), Some(legacy.id.as_str()));
        assert_eq!(item.question, "Keep the existing terminal?");
        assert_eq!(item.created_at, legacy.created_at);
        assert_eq!(item.waiters, [Waiter::Pm]);
        // Answering reaches whichever PM is registered, in the reply wording
        // the PM already understands.
        answer(&repo, &item.id, "1. Yes", AnsweredBy::User).unwrap();
        let pending = pending_deliveries(&repo).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].waiter, Waiter::Pm);
        assert_eq!(
            pending[0].message,
            "Answer to \"Keep the existing terminal?\": 1. Yes"
        );
        // The answered report is never imported a second time.
        sync_pm_report_decisions(&repo).unwrap();
        assert_eq!(snapshot(&repo).unwrap().waiting_count, 0);
    }

    #[test]
    fn typed_pm_decisions_keep_their_packet_and_withdraw_when_resolved() {
        use crate::pm_report::*;
        let (_home, _guard, repo) = scoped();
        let submission = |request: &str, revision: u64, status: ReportStatus| Submission {
            schema_version: 1,
            request_id: request.into(),
            events: vec![ReportEvent {
                episode_id: "episode".into(),
                event_id: "event".into(),
                expected_revision: revision,
                kind: PmReportKind::Decision,
                state: MaterialState {
                    status,
                    severity: ReportSeverity::NeedsHuman,
                    impact: ReportImpact::Project,
                    action: Some(ReportAction {
                        id: "decide".into(),
                        kind: ReportActionKind::Decide,
                        deadline: None,
                    }),
                    milestone_id: None,
                },
                summary: ReportSummary {
                    result: "Release is ready".into(),
                    impact: Some("Blocks the release".into()),
                    owner: Some("PM".into()),
                    next_action: None,
                    decision: Some(ReportDecision {
                        question: "Ship today?".into(),
                        recommendation: "Ship".into(),
                        reason: "All checks pass".into(),
                        options: vec!["Ship".into(), "Wait".into()],
                    }),
                },
                details: "Details".into(),
                refs: Vec::new(),
                observed_at: None,
            }],
            digest: None,
        };
        post_submission(&repo, &submission("r1", 0, ReportStatus::Waiting)).unwrap();
        sync_pm_report_decisions(&repo).unwrap();
        let queue = snapshot(&repo).unwrap();
        assert_eq!(queue.waiting_count, 1);
        let item = &queue.items[0];
        assert_eq!(item.question, "Ship today?");
        assert_eq!(item.options, ["Ship", "Wait"]);
        assert_eq!(item.recommendation.as_deref(), Some("Ship"));
        assert_eq!(item.impact.as_deref(), Some("Blocks the release"));
        assert_eq!(item.detail_text, None);
        post_submission(&repo, &submission("r2", 1, ReportStatus::Resolved)).unwrap();
        sync_pm_report_decisions(&repo).unwrap();
        assert_eq!(snapshot(&repo).unwrap().waiting_count, 0);
        assert_eq!(
            list(&repo, None, true).unwrap()[0].status,
            DecisionStatus::Withdrawn
        );
    }

    #[test]
    fn an_unreadable_queue_is_reported_instead_of_being_replaced() {
        let (_home, _guard, repo) = scoped();
        register(&repo, &registration(7742, "product", "storage", "s1")).unwrap();
        let path = queue_path(&repo);
        fs::write(&path, "{ not json").unwrap();
        assert!(snapshot(&repo).is_err());
        assert!(register(&repo, &registration(7742, "product", "other", "s1")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
        fs::write(&path, r#"{"schema_version":99}"#).unwrap();
        assert!(snapshot(&repo)
            .unwrap_err()
            .to_string()
            .contains("not supported"));
    }
}
