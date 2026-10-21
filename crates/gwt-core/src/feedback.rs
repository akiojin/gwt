//! Durable feedback intake. No GitHub or human-consent side effects live here.

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use chrono::{DateTime, TimeDelta, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    Bug,
    Request,
    Friction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackInput {
    pub title: String,
    pub kind: FeedbackKind,
    pub operation: String,
    pub refusal: String,
    pub reproduction: String,
    pub workaround: Option<String>,
    #[serde(default)]
    pub body: String,
    pub error_record_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackContext {
    pub project_root: PathBuf,
    pub project_hash: String,
    pub gwt_version: String,
    pub gwtd_version: String,
    pub os: String,
    pub active_agents: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackState {
    PendingApproval,
    Queued,
    Sending,
    Sent,
    Declined,
    Failed,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub input: FeedbackInput,
    pub context: FeedbackContext,
    pub state: FeedbackState,
    pub issue_number: Option<u64>,
    pub issue_url: Option<String>,
    pub error: Option<String>,
}

impl FeedbackRecord {
    pub fn marker(&self) -> String {
        format!("<!-- gwt-feedback:{} -->", self.id)
    }

    /// Derive the immutable public preview/send payload. The stored receipt
    /// remains the complete original, including evidence hidden by these masks.
    pub fn public_snapshot(&self) -> Self {
        let mut public = self.clone();
        let root = self.context.project_root.to_string_lossy();
        for text in [
            &mut public.input.title,
            &mut public.input.operation,
            &mut public.input.refusal,
            &mut public.input.reproduction,
            &mut public.input.body,
        ]
        .into_iter()
        .chain(public.input.workaround.iter_mut())
        .chain(public.input.error_record_id.iter_mut())
        .chain(public.input.labels.iter_mut())
        {
            *text = public_text(text, &root);
        }
        public.context.project_root = PathBuf::from("[REDACTED PATH]");
        public
    }

    pub fn render_issue(&self) -> String {
        let kind = match self.input.kind {
            FeedbackKind::Bug => "bug",
            FeedbackKind::Request => "request",
            FeedbackKind::Friction => "friction",
        };
        // A longer fence prevents refusal text containing Markdown fences from
        // swallowing the structured context. Evidence itself is never trimmed.
        let fence = "`".repeat(
            self.input
                .refusal
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                .max(2)
                + 1,
        );
        let details = serde_json::to_string_pretty(&serde_json::json!({
            "kind": self.input.kind, "operation": self.input.operation,
            "environment": self.context, "reproduction": self.input.reproduction,
            "workaround": self.input.workaround, "error_record_id": self.input.error_record_id,
            "requested_labels": self.input.labels,
        }))
        .expect("feedback metadata serializes");
        format!("{}\n<!-- gwt-feedback-kind:{kind} -->\n\n{}\n\n## Structured report\n\n```json\n{}\n```\n\n## Observed refusal (verbatim except privacy masks)\n\n{}text\n{}\n{}\n",
            self.marker(), self.input.body, details, fence, self.input.refusal, fence)
    }
}

fn public_text(text: &str, project_root: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag};
    use regex::Regex;

    // Offset ranges retain the authored bytes; event text normalizes Markdown.
    let mut masked = String::new();
    let mut copied = 0;
    for (event, range) in Parser::new(text).into_offset_iter() {
        if matches!(event, Event::Start(Tag::CodeBlock(_)) | Event::Code(_))
            && range.start >= copied
        {
            masked.push_str(&text[copied..range.start]);
            masked.push_str("[REDACTED CODE]");
            masked.extend(
                text[range.clone()]
                    .chars()
                    .filter(|c| matches!(c, '\r' | '\n')),
            );
            copied = range.end;
        }
    }
    masked.push_str(&text[copied..]);
    if !project_root.is_empty() {
        masked = masked.replace(project_root, "[REDACTED PATH]");
    }

    static ENVIRONMENT: OnceLock<Regex> = OnceLock::new();
    let environment = ENVIRONMENT.get_or_init(|| {
        Regex::new(r#"(?P<key>\b[A-Za-z_][A-Za-z0-9_]*[ \t]*=[ \t]*|["'][A-Z_][A-Z0-9_]*["'][ \t]*:\s*)(?P<value>(?:"(?:\\[\s\S]|[^"\\])*"?|'[^']*'?|(?:\\(?:\r\n|[\s\S])?|[^"'\\\r\n])+)+)"#)
            .expect("feedback environment value pattern")
    });
    // Quoted assignments can span physical lines. Mask their complete source
    // value before the line-scoped credential pass, retaining all line endings.
    // Treat the rest of an assignment line, including unquoted continuations,
    // conservatively as part of its value.
    let masked = environment.replace_all(&masked, |captures: &regex::Captures<'_>| {
        let mut value = format!("{}[REDACTED]", &captures["key"]);
        value.extend(
            captures["value"]
                .chars()
                .filter(|c| matches!(c, '\r' | '\n')),
        );
        value
    });
    static BEARER: OnceLock<Regex> = OnceLock::new();
    let bearer = BEARER.get_or_init(|| {
        Regex::new(r"(?i)\bBearer[ \t]+[^\s]+").expect("feedback bearer value pattern")
    });
    static PATHS: OnceLock<Regex> = OnceLock::new();
    let paths = PATHS.get_or_init(|| {
        // Conservatively hide path-shaped strings, including URLs, because
        // their public/private visibility cannot be established locally.
        Regex::new(r#"(?x)["'](?:[A-Za-z]:[\\/]|\\\\|~[\\/]|\.{0,2}/|[\p{L}\p{N}_.-]+[\\/])[^"'\r\n]*["']|[A-Za-z][A-Za-z0-9+.-]*://[^\s<>"'`]+|(?:[A-Za-z]:[\\/]|\\\\|~[\\/]|\.{0,2}/|[\p{L}\p{N}_.-]+[\\/])[^\s<>"'`]*"#)
            .expect("feedback private path pattern")
    });
    let mut public = String::new();
    // Credential patterns can consume whitespace: never let them cross the
    // original physical line boundaries, including bare CR and CRLF.
    for line in masked.split_inclusive(['\r', '\n']) {
        let content_len = line.trim_end_matches(['\r', '\n']).len();
        let (content, ending) = line.split_at(content_len);
        let content = bearer.replace_all(content, "[REDACTED]");
        let content = crate::error_ledger::redact_credentials(&content);
        public.push_str(&paths.replace_all(&content, "[REDACTED PATH]"));
        public.push_str(ending);
    }
    public
}

/// Unknown identities also require approval. Global/user configuration never
/// grants the upstream exemption, even when it rewrites a foreign origin.
pub fn requires_approval(project_root: &Path) -> bool {
    let Some(origin) = crate::repo_hash::repository_local_origin(project_root) else {
        return true;
    };
    // Consent authority accepts only canonical upstream clone forms. The
    // general repository-hash normalizer is intentionally more permissive.
    let origin = origin.trim().trim_end_matches('/').to_ascii_lowercase();
    let origin = origin.strip_suffix(".git").unwrap_or(&origin);
    !matches!(
        origin,
        "https://github.com/akiojin/gwt"
            | "ssh://git@github.com/akiojin/gwt"
            | "git@github.com:akiojin/gwt"
    )
}

pub struct FeedbackStore {
    root: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    id: String,
    created_at: DateTime<Utc>,
}

impl FeedbackStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn path(&self, id: &str) -> io::Result<PathBuf> {
        uuid::Uuid::parse_str(id).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid feedback request id")
        })?;
        Ok(self.root.join(format!("{id}.json")))
    }

    pub fn read(&self, id: &str) -> io::Result<FeedbackRecord> {
        let record: FeedbackRecord = serde_json::from_slice(&fs::read(self.path(id)?)?)?;
        if record.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "feedback receipt identity mismatch",
            ));
        }
        Ok(record)
    }

    pub fn save(&self, record: &FeedbackRecord) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let path = self.path(&record.id)?;
        durable_write(&path, &serde_json::to_vec_pretty(record)?)
    }

    /// Intake holds only a short filesystem lock, never a human/network wait.
    /// Repeats return their existing receipt without starting another sender.
    pub fn submit(
        &self,
        input: FeedbackInput,
        context: FeedbackContext,
        now: DateTime<Utc>,
    ) -> io::Result<(FeedbackRecord, bool)> {
        if input.title.trim().is_empty()
            || input.operation.trim().is_empty()
            || input.reproduction.trim().is_empty()
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "feedback requires title, operation, and reproduction (use unconfirmed when unknown)"));
        }
        fs::create_dir_all(self.root.join("fingerprints"))?;
        let lock = self.open_lock("intake")?;
        lock.lock_exclusive()?;
        let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&(
            &input.kind,
            input.title.to_lowercase(),
            &input.operation,
            &input.refusal,
            &input.reproduction,
            &input.workaround,
            &input.body,
            &input.labels,
        ))?));
        let index = self
            .root
            .join("fingerprints")
            .join(format!("{fingerprint}.json"));
        if index.exists() {
            let receipt: Receipt = serde_json::from_slice(&fs::read(&index)?)?;
            if now.signed_duration_since(receipt.created_at) < TimeDelta::hours(1) {
                return Ok((self.read(&receipt.id)?, false));
            }
        }
        let state = if requires_approval(&context.project_root) {
            FeedbackState::PendingApproval
        } else {
            FeedbackState::Queued
        };
        let record = FeedbackRecord {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: now,
            input,
            context,
            state,
            issue_number: None,
            issue_url: None,
            error: None,
        };
        self.save(&record)?;
        durable_write(
            &index,
            &serde_json::to_vec(&Receipt {
                id: record.id.clone(),
                created_at: now,
            })?,
        )?;
        Ok((record, true))
    }

    pub fn try_sender_lock(&self, id: &str) -> io::Result<File> {
        self.path(id)?;
        let lock = self.open_lock(id)?;
        lock.try_lock_exclusive()?;
        Ok(lock)
    }

    fn open_lock(&self, name: &str) -> io::Result<File> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join(format!("{name}.lock")))
    }
}

fn durable_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    crate::atomic_file::write_atomic(path, bytes)?;
    // Windows FlushFileBuffers requires write access; preserve the payload.
    OpenOptions::new().write(true).open(path)?.sync_all()?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
