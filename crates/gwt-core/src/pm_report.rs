//! Durable PM-authored reports, separate from terminal and conversation history.

use std::{
    cell::Cell,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{GwtError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmReportKind {
    Progress,
    Decision,
    Blocker,
    Qa,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmReport {
    pub id: String,
    pub kind: PmReportKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

impl From<&crate::user_instruction::UserInstruction> for PmReport {
    fn from(record: &crate::user_instruction::UserInstruction) -> Self {
        Self {
            id: record.id.clone(),
            kind: PmReportKind::Qa,
            question: Some(record.summary.clone()),
            body: record.answer.clone(),
            created_at: record.created_at,
        }
    }
}

/// Reports share the repository's machine-local project store and never expire.
pub fn reports_path(repo_path: &Path) -> PathBuf {
    crate::paths::gwt_project_dir_for_repo_path(repo_path).join("project-state/pm-reports.jsonl")
}

/// Validate fields shared by legacy Reports and instruction-backed Q&A.
pub fn validate_report_input(kind: PmReportKind, body: &str, question: Option<&str>) -> Result<()> {
    if kind == PmReportKind::Qa && question.is_none_or(|value| value.trim().is_empty()) {
        return Err(GwtError::Config(
            "PM Q&A report question must be nonempty".into(),
        ));
    }
    if question.is_some_and(|value| value.len() > 64 * 1024) {
        return Err(GwtError::Config(
            "PM report question must not exceed 64 KiB".into(),
        ));
    }
    if body.trim().is_empty() {
        return Err(GwtError::Config("PM report body must be nonempty".into()));
    }
    if body.len() > 64 * 1024 {
        return Err(GwtError::Config(
            "PM report body must not exceed 64 KiB".into(),
        ));
    }
    Ok(())
}

/// A successful post has been appended, synced and read back while holding the lock.
pub fn post_report(
    repo_path: &Path,
    kind: PmReportKind,
    body: &str,
    question: Option<&str>,
) -> Result<PmReport> {
    validate_report_input(kind, body, question)?;
    let path = reports_path(repo_path);
    fs::create_dir_all(path.parent().expect("report store has a parent"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    file.lock_exclusive()?;
    // Reject damaged history before appending a success the reader cannot display.
    read_reports(&mut file, &path)?;
    let report = PmReport {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        question: question.map(str::to_owned),
        body: body.to_owned(),
        created_at: Utc::now(),
    };
    append_report(&mut file, &path, &report, |file, encoded| {
        file.write_all(encoded)
    })?;
    Ok(report)
}

fn append_report(
    file: &mut File,
    path: &Path,
    report: &PmReport,
    append: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<()> {
    let offset = file.seek(SeekFrom::End(0))?;
    let result = (|| -> Result<()> {
        let mut encoded = serde_json::to_vec(report).map_err(invalid_report_data)?;
        encoded.push(b'\n');
        append(file, &encoded)?;
        file.sync_all()?;
        sync_report_parent(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut persisted = String::new();
        file.read_to_string(&mut persisted)?;
        let readback: PmReport = serde_json::from_str(&persisted).map_err(invalid_report_data)?;
        if &readback != report {
            return Err(invalid_report_data(
                "PM report readback did not match the submitted report",
            )
            .into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        // The caller still holds the exclusive lock, so no other writer can
        // append between this transaction and restoring the previous length.
        if let Err(rollback) = file.set_len(offset).and_then(|()| file.sync_all()) {
            return Err(io::Error::other(format!(
                "PM report append failed: {error}; rollback to byte {offset} failed: {rollback}"
            ))
            .into());
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(unix)]
fn sync_report_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().expect("report store has a parent"))?.sync_all()
}

#[cfg(not(unix))]
fn sync_report_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub fn load_reports(repo_path: &Path) -> Result<Vec<PmReport>> {
    let path = reports_path(repo_path);
    let mut reports = match File::open(&path) {
        Ok(mut file) => {
            FileExt::lock_shared(&file)?;
            read_reports(&mut file, &path)?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    // Captured Q&A and its instruction share one durable source. Legacy
    // reports remain readable without introducing a second write transaction.
    reports.extend(
        crate::user_instruction::load(repo_path)?
            .iter()
            .map(PmReport::from),
    );
    reports.sort_by_key(|report| report.created_at);
    Ok(reports)
}

fn read_reports(file: &mut File, path: &Path) -> Result<Vec<PmReport>> {
    let mut reports = Vec::new();
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_number = 0;
    while reader.read_line(&mut line)? != 0 {
        line_number += 1;
        if !line.ends_with('\n') {
            return Err(invalid_report_data(format!(
                "{} line {line_number}: incomplete PM report record (missing newline)",
                path.display()
            ))
            .into());
        }
        if !line.trim().is_empty() {
            reports.push(serde_json::from_str(&line).map_err(|error| {
                invalid_report_data(format!("{} line {line_number}: {error}", path.display()))
            })?);
        }
        line.clear();
    }
    Ok(reports)
}

fn invalid_report_data(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmReportsSource {
    Legacy,
    Instruction,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PmReportsHistoryCursor {
    pub created_at: DateTime<Utc>,
    pub source: PmReportsSource,
    pub ordinal: u64,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmReportsUpdate {
    Unchanged,
    Delta {
        reports: Vec<PmReport>,
        removed_ids: Vec<String>,
        ordered_ids: Vec<String>,
    },
    Reset {
        reports: Vec<PmReport>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmReportsPage {
    pub reports: Vec<PmReport>,
    pub before: Option<PmReportsHistoryCursor>,
}

#[derive(Debug, Default)]
pub struct PmReportsReader {
    reports: Vec<PmReport>,
    cursors: Vec<PmReportsHistoryCursor>,
    initialized: bool,
    legacy_stamp: Option<ReportSourceStamp>,
    legacy_offset: u64,
    legacy_anchor: Vec<u8>,
    legacy_count: u64,
    instruction_stamp: Option<ReportSourceStamp>,
    instruction_count: u64,
    instruction_digest: [u8; 32],
}

impl PmReportsReader {
    /// Refresh only the append suffix for JSONL. Atomic instruction snapshots
    /// are streamed, comparing their old projection prefix without retaining it.
    /// No cached state moves until both sources have been read successfully.
    pub fn refresh(&mut self, repo_path: &Path) -> Result<PmReportsUpdate> {
        let mut legacy = open_report_source(&reports_path(repo_path), true)?;
        let mut instructions = open_report_source(
            &crate::user_instruction::instructions_path(repo_path),
            false,
        )?;
        let legacy_stamp = source_stamp(legacy.as_ref())?;
        let instruction_stamp = source_stamp(instructions.as_ref())?;
        if self.initialized
            && legacy_stamp == self.legacy_stamp
            && instruction_stamp == self.instruction_stamp
        {
            return Ok(PmReportsUpdate::Unchanged);
        }
        let legacy_append = match (&self.legacy_stamp, &legacy_stamp) {
            (Some(old), Some(new)) => {
                old.same_file(new)
                    && new.len > old.len
                    && legacy_anchor(legacy.as_mut(), self.legacy_offset)? == self.legacy_anchor
            }
            (None, None) => true,
            _ => false,
        };
        let mut reset = !self.initialized || (legacy_stamp != self.legacy_stamp && !legacy_append);
        let mut instruction_scan = if reset || instruction_stamp != self.instruction_stamp {
            Some(scan_instructions(
                instructions.as_mut(),
                None,
                self.initialized
                    .then_some((self.instruction_count, self.instruction_digest)),
            )?)
        } else {
            None
        };
        if instruction_scan
            .as_ref()
            .is_some_and(|scan| !scan.prefix_matches)
        {
            reset = true;
        }
        let legacy_scan = if reset || legacy_stamp != self.legacy_stamp {
            Some(scan_legacy(
                legacy.as_mut(),
                if reset { 0 } else { self.legacy_offset },
                if reset { 0 } else { self.legacy_count },
                None,
            )?)
        } else {
            None
        };
        if reset && instruction_scan.is_none() {
            instruction_scan = Some(scan_instructions(instructions.as_mut(), None, None)?);
        }
        let mut tail = ReportTail::default();
        if !reset {
            for (report, cursor) in self.reports.iter().zip(&self.cursors) {
                tail.push(report.clone(), cursor.clone());
            }
        }
        if let Some(scan) = &legacy_scan {
            tail.extend(&scan.tail);
        }
        if let Some(scan) = &instruction_scan {
            tail.extend(if reset { &scan.tail } else { &scan.appended });
        }
        let (reports, cursors) = tail.into_parts();
        let legacy_count = legacy_scan
            .as_ref()
            .map_or(self.legacy_count, |scan| scan.count);
        let instruction_count = instruction_scan
            .as_ref()
            .map_or(self.instruction_count, |scan| scan.count);
        let old_more = self.legacy_count + self.instruction_count > self.reports.len() as u64;
        let new_more = legacy_count + instruction_count > reports.len() as u64;
        let update = if self.initialized
            && reports == self.reports
            && cursors == self.cursors
            && old_more == new_more
        {
            PmReportsUpdate::Unchanged
        } else if reset {
            PmReportsUpdate::Reset {
                reports: reports.clone(),
            }
        } else {
            PmReportsUpdate::Delta {
                reports: reports
                    .iter()
                    .filter(|report| !self.reports.iter().any(|old| old == *report))
                    .cloned()
                    .collect(),
                removed_ids: self
                    .reports
                    .iter()
                    .filter(|old| !reports.iter().any(|report| report.id == old.id))
                    .map(|report| report.id.clone())
                    .collect(),
                ordered_ids: reports.iter().map(|report| report.id.clone()).collect(),
            }
        };
        let new_legacy_offset = legacy_scan
            .as_ref()
            .map_or(self.legacy_offset, |scan| scan.offset);
        let new_legacy_anchor = legacy_anchor(legacy.as_mut(), new_legacy_offset)?;
        if let Some(scan) = legacy_scan {
            self.legacy_count = scan.count;
            self.legacy_offset = scan.offset;
        }
        self.legacy_anchor = new_legacy_anchor;
        if let Some(scan) = instruction_scan {
            self.instruction_count = scan.count;
            self.instruction_digest = scan.digest;
        }
        self.legacy_stamp = legacy_stamp;
        self.instruction_stamp = instruction_stamp;
        self.reports = reports;
        self.cursors = cursors;
        self.initialized = true;
        Ok(update)
    }

    pub fn tail(&self) -> &[PmReport] {
        &self.reports
    }

    pub fn history_cursor(&self) -> Option<PmReportsHistoryCursor> {
        (self.legacy_count + self.instruction_count > self.reports.len() as u64)
            .then(|| self.cursors.first().cloned())
            .flatten()
    }
}

pub fn load_reports_before(
    repo_path: &Path,
    before: Option<&PmReportsHistoryCursor>,
) -> Result<PmReportsPage> {
    let mut legacy = open_report_source(&reports_path(repo_path), true)?;
    let mut instructions = open_report_source(
        &crate::user_instruction::instructions_path(repo_path),
        false,
    )?;
    let legacy = scan_legacy(legacy.as_mut(), 0, 0, before)?;
    let instructions = scan_instructions(instructions.as_mut(), before, None)?;
    if before.is_some() && !legacy.anchor_found && !instructions.anchor_found {
        return Err(invalid_report_data("PM report history cursor is stale or invalid").into());
    }
    let mut tail = legacy.tail;
    tail.extend(&instructions.tail);
    let more = legacy.eligible + instructions.eligible > tail.entries.len() as u64;
    let (reports, cursors) = tail.into_parts();
    Ok(PmReportsPage {
        reports,
        before: more.then(|| cursors.first().cloned()).flatten(),
    })
}

const REPORT_TAIL_COUNT: usize = 200;
const REPORT_TAIL_BODY_BYTES: usize = 2 * 1024 * 1024;
const REPORT_FIELD_BYTES: usize = 64 * 1024;
// Each decoded byte can occupy at most six JSON bytes (a Unicode escape).
const REPORT_LINE_BYTES: usize = 6 * REPORT_FIELD_BYTES * 3 + 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportSourceStamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl ReportSourceStamp {
    fn same_file(&self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            self.identity == other.identity
        }
        #[cfg(not(unix))]
        {
            self.created.is_some() && self.created == other.created
        }
    }
}

fn open_report_source(path: &Path, locked: bool) -> Result<Option<File>> {
    match File::open(path) {
        Ok(file) => {
            if locked {
                FileExt::lock_shared(&file)?;
            }
            Ok(Some(file))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn source_stamp(file: Option<&File>) -> Result<Option<ReportSourceStamp>> {
    file.map(|file| {
        let metadata = file.metadata()?;
        Ok(ReportSourceStamp {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino())
            },
        })
    })
    .transpose()
}

fn legacy_anchor(file: Option<&mut File>, offset: u64) -> Result<Vec<u8>> {
    let Some(file) = file else {
        return Ok(Vec::new());
    };
    let start = offset.saturating_sub(64);
    file.seek(SeekFrom::Start(start))?;
    let mut anchor = vec![0; (offset - start) as usize];
    file.read_exact(&mut anchor)?;
    Ok(anchor)
}

#[derive(Default)]
struct ReportTail {
    entries: Vec<(PmReportsHistoryCursor, PmReport)>,
    body_bytes: usize,
}

impl ReportTail {
    fn push(&mut self, report: PmReport, cursor: PmReportsHistoryCursor) {
        self.body_bytes += report.body.len();
        let position = self.entries.partition_point(|(key, _)| key <= &cursor);
        self.entries.insert(position, (cursor, report));
        while self.entries.len() > REPORT_TAIL_COUNT || self.body_bytes > REPORT_TAIL_BODY_BYTES {
            self.body_bytes -= self.entries.remove(0).1.body.len();
        }
    }

    fn extend(&mut self, other: &Self) {
        for (cursor, report) in &other.entries {
            self.push(report.clone(), cursor.clone());
        }
    }

    fn into_parts(self) -> (Vec<PmReport>, Vec<PmReportsHistoryCursor>) {
        let (cursors, reports) = self.entries.into_iter().unzip();
        (reports, cursors)
    }
}

fn report_cursor(
    report: &PmReport,
    source: PmReportsSource,
    ordinal: u64,
) -> PmReportsHistoryCursor {
    PmReportsHistoryCursor {
        created_at: report.created_at,
        source,
        ordinal,
        id: report.id.clone(),
    }
}

fn validate_read_fields(report: &PmReport) -> Result<()> {
    if report.id.len() > REPORT_FIELD_BYTES
        || report.body.len() > REPORT_FIELD_BYTES
        || report
            .question
            .as_ref()
            .is_some_and(|question| question.len() > REPORT_FIELD_BYTES)
    {
        return Err(invalid_report_data("PM report fields must not exceed 64 KiB").into());
    }
    Ok(())
}

#[derive(Default)]
struct LegacyScan {
    tail: ReportTail,
    count: u64,
    offset: u64,
    eligible: u64,
    anchor_found: bool,
}

fn scan_legacy(
    file: Option<&mut File>,
    offset: u64,
    count: u64,
    before: Option<&PmReportsHistoryCursor>,
) -> Result<LegacyScan> {
    let mut scan = LegacyScan {
        count,
        offset,
        ..LegacyScan::default()
    };
    let Some(file) = file else {
        return Ok(LegacyScan::default());
    };
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        let read = reader
            .by_ref()
            .take(REPORT_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if line.len() > REPORT_LINE_BYTES {
            return Err(invalid_report_data(
                "PM report record exceeds its bounded 64 KiB field envelope",
            )
            .into());
        }
        if line.last() != Some(&b'\n') {
            return Err(
                invalid_report_data("incomplete PM report record (missing newline)").into(),
            );
        }
        scan.offset += read as u64;
        if !line.iter().all(u8::is_ascii_whitespace) {
            let report: PmReport = serde_json::from_slice(&line).map_err(invalid_report_data)?;
            validate_read_fields(&report)?;
            let cursor = report_cursor(&report, PmReportsSource::Legacy, scan.count);
            scan.count += 1;
            scan.anchor_found |= before == Some(&cursor);
            if before.is_none_or(|before| cursor < *before) {
                scan.eligible += 1;
                scan.tail.push(report, cursor);
            }
        }
        line.clear();
    }
    Ok(scan)
}

struct InstructionScan {
    tail: ReportTail,
    appended: ReportTail,
    count: u64,
    eligible: u64,
    anchor_found: bool,
    prefix_matches: bool,
    digest: [u8; 32],
}

/// The budget is enabled only around a required string. serde_json's ignored
/// values stream past original/actions without accumulating their string bytes.
struct ReportBudgetReader<R> {
    inner: R,
    budget: Rc<Cell<usize>>,
    depth: usize,
    quoted: bool,
    escaped: bool,
}

impl<R: Read> Read for ReportBudgetReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let remaining = self.budget.get();
        if remaining == 0 && !bytes.is_empty() {
            return Err(invalid_report_data(
                "PM report fields must not exceed 64 KiB",
            ));
        }
        let limit = bytes.len().min(remaining);
        let count = self.inner.read(&mut bytes[..limit])?;
        if remaining != usize::MAX {
            self.budget.set(remaining - count);
        }
        // IgnoredAny uses an iterative stack rather than serde_json's normal
        // recursion guard. Bound that stack too, without retaining skipped text.
        for byte in &bytes[..count] {
            if self.quoted {
                if self.escaped {
                    self.escaped = false;
                } else if *byte == b'\\' {
                    self.escaped = true;
                } else if *byte == b'"' {
                    self.quoted = false;
                }
            } else {
                match byte {
                    b'"' => self.quoted = true,
                    b'[' | b'{' => {
                        self.depth += 1;
                        if self.depth > 128 {
                            return Err(invalid_report_data(
                                "PM report JSON nesting exceeds 128 levels",
                            ));
                        }
                    }
                    b']' | b'}' => self.depth = self.depth.saturating_sub(1),
                    _ => {}
                }
            }
        }
        Ok(count)
    }
}

struct ReportTextSeed(Rc<Cell<usize>>);

impl<'de> DeserializeSeed<'de> for ReportTextSeed {
    type Value = String;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<String, D::Error> {
        self.0.set(6 * REPORT_FIELD_BYTES + 16);
        let value = String::deserialize(deserializer);
        self.0.set(usize::MAX);
        let value = value?;
        if value.len() > REPORT_FIELD_BYTES {
            return Err(serde::de::Error::custom(
                "PM report fields must not exceed 64 KiB",
            ));
        }
        Ok(value)
    }
}

struct InstructionReportSeed(Rc<Cell<usize>>);

impl<'de> DeserializeSeed<'de> for InstructionReportSeed {
    type Value = PmReport;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<PmReport, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for InstructionReportSeed {
    type Value = PmReport;
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a user instruction report projection")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> std::result::Result<PmReport, M::Error> {
        let (mut id, mut created_at, mut summary, mut answer) = (None, None, None, None);
        while let Some(key) = map.next_key_seed(ReportTextSeed(self.0.clone()))? {
            let field = match key.as_str() {
                "id" => &mut id,
                "created_at" => &mut created_at,
                "summary" => &mut summary,
                "answer" => &mut answer,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                    continue;
                }
            };
            if field.is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate instruction field: {key}"
                )));
            }
            *field = Some(map.next_value_seed(ReportTextSeed(self.0.clone()))?);
        }
        let required = |value: Option<String>, field: &str| {
            value.ok_or_else(|| {
                serde::de::Error::custom(format!("missing instruction field: {field}"))
            })
        };
        let timestamp = required(created_at, "created_at")?;
        Ok(PmReport {
            id: required(id, "id")?,
            kind: PmReportKind::Qa,
            question: Some(required(summary, "summary")?),
            body: required(answer, "answer")?,
            created_at: DateTime::parse_from_rfc3339(&timestamp)
                .map_err(serde::de::Error::custom)?
                .with_timezone(&Utc),
        })
    }
}

struct InstructionArraySeed<'a> {
    budget: Rc<Cell<usize>>,
    before: Option<&'a PmReportsHistoryCursor>,
    previous: Option<(u64, [u8; 32])>,
}

impl<'de> DeserializeSeed<'de> for InstructionArraySeed<'_> {
    type Value = InstructionScan;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<InstructionScan, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for InstructionArraySeed<'_> {
    type Value = InstructionScan;
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a user instruction array")
    }
    fn visit_seq<S: SeqAccess<'de>>(
        self,
        mut sequence: S,
    ) -> std::result::Result<InstructionScan, S::Error> {
        let mut hash = Sha256::new();
        let mut scan = InstructionScan {
            tail: ReportTail::default(),
            appended: ReportTail::default(),
            count: 0,
            eligible: 0,
            anchor_found: false,
            prefix_matches: self.previous.is_none_or(|(count, _)| count == 0),
            digest: [0; 32],
        };
        while let Some(report) =
            sequence.next_element_seed(InstructionReportSeed(self.budget.clone()))?
        {
            let cursor = report_cursor(&report, PmReportsSource::Instruction, scan.count);
            scan.anchor_found |= self.before == Some(&cursor);
            for field in [
                &report.id,
                report.question.as_ref().expect("Q&A question"),
                &report.body,
            ] {
                hash.update((field.len() as u64).to_le_bytes());
                hash.update(field.as_bytes());
            }
            hash.update(report.created_at.timestamp().to_le_bytes());
            hash.update(report.created_at.timestamp_subsec_nanos().to_le_bytes());
            if self.previous.is_some_and(|(count, _)| scan.count >= count) {
                scan.appended.push(report.clone(), cursor.clone());
            }
            scan.count += 1;
            if let Some((count, digest)) = self.previous {
                if scan.count == count {
                    scan.prefix_matches = <[u8; 32]>::from(hash.clone().finalize()) == digest;
                }
            }
            if self.before.is_none_or(|before| cursor < *before) {
                scan.eligible += 1;
                scan.tail.push(report, cursor);
            }
        }
        scan.digest = hash.finalize().into();
        Ok(scan)
    }
}

fn scan_instructions(
    file: Option<&mut File>,
    before: Option<&PmReportsHistoryCursor>,
    previous: Option<(u64, [u8; 32])>,
) -> Result<InstructionScan> {
    let budget = Rc::new(Cell::new(usize::MAX));
    let seed = InstructionArraySeed {
        budget: budget.clone(),
        before,
        previous,
    };
    // An absent source is the same projection as an empty array.
    let mut empty = &b"[]"[..];
    let reader: &mut dyn Read = match file {
        Some(file) => {
            file.seek(SeekFrom::Start(0))?;
            file
        }
        None => &mut empty,
    };
    let reader = ReportBudgetReader {
        inner: BufReader::new(reader),
        budget,
        depth: 0,
        quoted: false,
        escaped: false,
    };
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let scan = seed
        .deserialize(&mut deserializer)
        .map_err(invalid_report_data)?;
    deserializer.end().map_err(invalid_report_data)?;
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedGwtHome;

    fn fixture(index: u64, body_bytes: usize) -> PmReport {
        PmReport {
            id: format!("report-{index:03}"),
            kind: PmReportKind::Progress,
            question: None,
            body: "x".repeat(body_bytes),
            created_at: DateTime::from_timestamp(1_800_000_000 + index as i64, 0).unwrap(),
        }
    }

    fn write_legacy(repo: &Path, reports: impl IntoIterator<Item = PmReport>) {
        let path = reports_path(repo);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = File::create(path).unwrap();
        for report in reports {
            serde_json::to_writer(&mut file, &report).unwrap();
            file.write_all(b"\n").unwrap();
        }
    }

    fn instruction(index: u64) -> serde_json::Value {
        serde_json::json!({
            "id": format!("instruction-{index}"),
            "created_at": fixture(index, 1).created_at,
            "summary": "Question", "answer": "Answer", "original": "Original",
            "actions": [], "issue_numbers": [], "kind": "standing", "superseded_by": null
        })
    }

    fn write_instructions(repo: &Path, records: &[serde_json::Value]) {
        let path = crate::user_instruction::instructions_path(repo);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        crate::workspace_projection::write_atomic(&path, &serde_json::to_vec(records).unwrap())
            .unwrap();
    }

    #[test]
    fn reader_bounds_chronological_tail_and_reads_older_history() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        write_legacy(&repo, (0..205).rev().map(|index| fixture(index, 1)));
        let mut reader = PmReportsReader::default();
        assert!(matches!(
            reader.refresh(&repo).unwrap(),
            PmReportsUpdate::Reset { .. }
        ));
        assert_eq!(reader.tail().len(), 200);
        assert_eq!(reader.tail()[0].id, "report-005");
        let page = load_reports_before(&repo, reader.history_cursor().as_ref()).unwrap();
        assert_eq!(page.reports.len(), 5);
        assert!(page.before.is_none());
        write_legacy(&repo, (0..40).rev().map(|index| fixture(index, 65_536)));
        assert!(matches!(
            reader.refresh(&repo).unwrap(),
            PmReportsUpdate::Reset { .. }
        ));
        assert_eq!(reader.tail().len(), 32);
        assert_eq!(reader.tail()[0].id, "report-008");
        assert_eq!(
            reader
                .tail()
                .iter()
                .map(|report| report.body.len())
                .sum::<usize>(),
            2 * 1024 * 1024
        );
        // A newly available older page changes the projection even when a
        // backdated append cannot enter the full body-byte tail.
        let page = load_reports_before(&repo, reader.history_cursor().as_ref()).unwrap();
        assert_eq!(page.reports.len(), 8);
        write_legacy(&repo, (10..42).map(|index| fixture(index, 65_536)));
        reader.refresh(&repo).unwrap();
        assert!(reader.history_cursor().is_none());
        let mut file = OpenOptions::new()
            .append(true)
            .open(reports_path(&repo))
            .unwrap();
        serde_json::to_writer(&mut file, &fixture(1, 1)).unwrap();
        file.write_all(b"\n").unwrap();
        assert!(
            matches!(reader.refresh(&repo).unwrap(), PmReportsUpdate::Delta { reports, .. } if reports.is_empty())
        );
        assert!(reader.history_cursor().is_some());
    }

    #[test]
    fn reader_appends_delta_and_preserves_state_on_incomplete_record() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        write_legacy(&repo, [fixture(2, 1)]);
        let mut reader = PmReportsReader::default();
        reader.refresh(&repo).unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .open(reports_path(&repo))
            .unwrap();
        for report in [fixture(1, 1), fixture(3, 1)] {
            serde_json::to_writer(&mut file, &report).unwrap();
            file.write_all(b"\n").unwrap();
        }
        let PmReportsUpdate::Delta {
            reports,
            removed_ids,
            ordered_ids,
        } = reader.refresh(&repo).unwrap()
        else {
            panic!("a normal append must be a delta");
        };
        assert_eq!(reports, [fixture(1, 1), fixture(3, 1)]);
        assert!(removed_ids.is_empty());
        assert_eq!(ordered_ids, ["report-001", "report-002", "report-003"]);
        assert!(matches!(
            reader.refresh(&repo).unwrap(),
            PmReportsUpdate::Unchanged
        ));
        let saved = reader.tail().to_vec();
        file.write_all(b"{").unwrap();
        assert!(reader.refresh(&repo).is_err());
        assert_eq!(reader.tail(), saved);
    }

    #[test]
    fn instruction_rewrites_append_delta_skip_unused_fields_and_reset_changed_projection() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let mut first = instruction(1);
        first["original"] = serde_json::Value::String("x".repeat(2 * 1024 * 1024));
        first["actions"] = serde_json::json!(["x".repeat(2 * 1024 * 1024)]);
        write_instructions(&repo, &[first.clone()]);
        let mut reader = PmReportsReader::default();
        assert!(matches!(
            reader.refresh(&repo).unwrap(),
            PmReportsUpdate::Reset { .. }
        ));
        first["superseded_by"] = serde_json::json!("instruction-2");
        write_instructions(&repo, &[first.clone(), instruction(2)]);
        let PmReportsUpdate::Delta { reports, .. } = reader.refresh(&repo).unwrap() else {
            panic!("an atomic instruction append must be a delta");
        };
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, "instruction-2");
        first["original"] = serde_json::json!("Changed unused source");
        write_instructions(&repo, &[first.clone(), instruction(2)]);
        assert!(matches!(
            reader.refresh(&repo).unwrap(),
            PmReportsUpdate::Unchanged
        ));
        first["answer"] = serde_json::json!("Changed answer");
        write_instructions(&repo, &[first.clone(), instruction(2)]);
        assert!(matches!(
            reader.refresh(&repo).unwrap(),
            PmReportsUpdate::Reset { .. }
        ));
        let saved = reader.tail().to_vec();
        first["summary"] = serde_json::Value::String("x".repeat(1024 * 1024));
        write_instructions(&repo, &[first, instruction(2)]);
        assert!(reader
            .refresh(&repo)
            .unwrap_err()
            .to_string()
            .contains("64 KiB"));
        assert_eq!(reader.tail(), saved);
    }

    #[test]
    fn ignored_instruction_values_cannot_grow_an_unbounded_parser_stack() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let mut value = serde_json::Value::Null;
        for _ in 0..160 {
            value = serde_json::Value::Array(vec![value]);
        }
        let mut record = instruction(1);
        record["actions"] = value;
        write_instructions(&repo, &[record]);
        let mut reader = PmReportsReader::default();
        assert!(reader
            .refresh(&repo)
            .unwrap_err()
            .to_string()
            .contains("nesting"));
        assert!(reader.tail().is_empty());
    }

    #[test]
    fn reports_survive_reload_in_project_order() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        assert!(load_reports(&repo).unwrap().is_empty());
        let first =
            post_report(&repo, PmReportKind::Progress, "# Progress\n\n- Done", None).unwrap();
        let second = post_report(
            &repo,
            PmReportKind::Decision,
            "Keep the existing terminal",
            None,
        )
        .unwrap();
        assert_ne!(first.id, second.id);
        assert!(reports_path(&repo).starts_with(home.path().join(".gwt/projects")));
        assert!(reports_path(&repo).ends_with("project-state/pm-reports.jsonl"));
        assert_eq!(load_reports(&repo).unwrap(), [first, second]);
        assert_eq!(
            std::fs::read_to_string(reports_path(&repo))
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[test]
    fn qa_reports_preserve_question_with_legacy_records() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let path = reports_path(&repo);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let legacy = ["progress", "decision", "blocker"]
            .map(|kind| {
                serde_json::json!({
                    "id": kind, "kind": kind, "body": "Existing report",
                    "created_at": "2026-10-13T00:00:00Z"
                })
                .to_string()
            })
            .join("\n");
        fs::write(&path, format!("{legacy}\n")).unwrap();
        let qa = post_report(
            &repo,
            PmReportKind::Qa,
            "Yes. Verified the checks.",
            Some("Can we ship?"),
        )
        .unwrap();

        let reports = load_reports(&repo).unwrap();
        assert_eq!(reports.len(), 4);
        for report in &reports[..3] {
            assert!(serde_json::to_value(report)
                .unwrap()
                .get("question")
                .is_none());
        }
        assert_eq!(reports[3], qa);
        assert_eq!(
            serde_json::to_value(&reports[3]).unwrap()["question"],
            "Can we ship?"
        );
    }

    #[test]
    fn report_validation_and_storage_failures_are_explicit() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        for question in [None, Some(" \n ")] {
            let error = post_report(&repo, PmReportKind::Qa, "Answer", question).unwrap_err();
            assert!(
                error.to_string().contains("question must be nonempty"),
                "{error}"
            );
        }
        let blank = post_report(&repo, PmReportKind::Progress, " \n ", None).unwrap_err();
        assert!(blank.to_string().contains("nonempty"), "{blank}");
        let oversized =
            post_report(&repo, PmReportKind::Progress, &"x".repeat(65_537), None).unwrap_err();
        assert!(oversized.to_string().contains("64 KiB"), "{oversized}");
        let oversized_question = "あ".repeat(21_846);
        for kind in [PmReportKind::Qa, PmReportKind::Progress] {
            let error = post_report(&repo, kind, "Answer", Some(&oversized_question)).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("question must not exceed 64 KiB"),
                "{error}"
            );
        }
        let path = reports_path(&repo);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(post_report(&repo, PmReportKind::Blocker, "Cannot write", None).is_err());
        assert!(load_reports(&repo).is_err());
        std::fs::remove_dir(&path).unwrap();
        post_report(&repo, PmReportKind::Progress, "Complete record", None).unwrap();
        let mut truncated = std::fs::read_to_string(&path).unwrap();
        truncated.pop();
        std::fs::write(&path, &truncated).unwrap();
        assert!(
            load_reports(&repo).is_err(),
            "an incomplete final record must be reported"
        );
        assert!(post_report(&repo, PmReportKind::Blocker, "Must not append", None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), truncated);
    }

    #[test]
    fn failed_partial_append_preserves_previously_saved_reports() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let previous =
            post_report(&repo, PmReportKind::Progress, "Previously saved", None).unwrap();
        let path = reports_path(&repo);
        let original = fs::read(&path).unwrap();
        let report = PmReport {
            id: "failed".into(),
            body: "Not committed".into(),
            ..previous.clone()
        };
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.lock_exclusive().unwrap();
        let error = append_report(&mut file, &path, &report, |file, encoded| {
            file.write_all(&encoded[..7])?;
            Err(io::Error::other("injected partial write failure"))
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("injected partial write failure"),
            "{error}"
        );
        drop(file);
        assert_eq!(load_reports(&repo).unwrap(), [previous]);
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
