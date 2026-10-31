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

pub mod auto_report;
use auto_report::{AutoFeedbackRecord, AutoReason, SafeAutoReport};

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
    Withheld,
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
    pub fn public_snapshot(&self) -> PublicFeedback {
        let mut input = self.input.clone();
        let mut privacy = PrivacyMasks::new(self);
        for text in [
            &mut input.title,
            &mut input.operation,
            &mut input.refusal,
            &mut input.reproduction,
        ]
        .into_iter()
        .chain(input.workaround.iter_mut())
        {
            *text = privacy.text(text);
        }
        // These local fields have no role in the upstream diagnostic contract.
        // Never serialize the raw context, arbitrary body, labels or ledger id.
        input.body.clear();
        input.labels.clear();
        input.error_record_id = None;
        PublicFeedback {
            id: self.id.clone(),
            input,
            gwt_version: self.context.gwt_version.clone(),
            gwtd_version: self.context.gwtd_version.clone(),
            os: self.context.os.clone(),
            removed: privacy.removed.into_iter().collect(),
        }
    }

    pub fn render_issue(&self) -> String {
        self.public_snapshot().render_issue()
    }
}

/// The only payload accepted by the public sender. Source context cannot be
/// serialized accidentally because it is absent from this transport type.
#[derive(Debug, Clone)]
pub struct PublicFeedback {
    pub id: String,
    pub input: FeedbackInput,
    pub gwt_version: String,
    pub gwtd_version: String,
    pub os: String,
    pub removed: Vec<&'static str>,
}

impl PublicFeedback {
    pub fn marker(&self) -> String {
        format!("<!-- gwt-feedback:{} -->", self.id)
    }

    pub fn render_issue(&self) -> String {
        let kind = match self.input.kind {
            FeedbackKind::Bug => "bug",
            FeedbackKind::Request => "request",
            FeedbackKind::Friction => "friction",
        };
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
            "environment": {
                "gwt_version": self.gwt_version,
                "gwtd_version": self.gwtd_version,
                "os": self.os,
            },
            "reproduction": self.input.reproduction,
            "workaround": self.input.workaround,
        }))
        .expect("public feedback metadata serializes");
        format!("{}\n<!-- gwt-feedback-kind:{kind} -->\n\n## Structured report\n\n```json\n{}\n```\n\n## Observed refusal (verbatim except privacy masks)\n\n{}text\n{}\n{}\n",
            self.marker(), details, fence, self.input.refusal, fence)
    }

    pub fn privacy_summary(&self) -> String {
        let text = format!(
            "{}\n{}\n{}\n{}\n{}",
            self.input.title,
            self.input.operation,
            self.input.refusal,
            self.input.reproduction,
            self.input.workaround.as_deref().unwrap_or("")
        );
        let masks = regex::Regex::new(r"\[REDACTED(?: [A-Z]+)?\]|\*\*\*redacted\*\*\*")
            .expect("privacy masks");
        let masked: usize = masks
            .find_iter(&text)
            .map(|m| m.as_str().chars().count())
            .sum();
        let total = text.chars().count().max(1);
        let percent = masked * 100 / total;
        format!("Removed items: local metadata and unstructured context{}\nPrivacy masks: {percent}% of diagnostic text.{}\nUnknown project-specific names may remain; review the complete report before sending.",
            if self.removed.is_empty() { String::new() } else { format!(", {}", self.removed.join(", ")) },
            if masked * 2 > total { " The report may be unreadable because more than half is masked." } else { "" })
    }
}

struct PrivacyMasks {
    identifiers: Vec<String>,
    root: String,
    removed: std::collections::BTreeSet<&'static str>,
}

impl PrivacyMasks {
    fn new(record: &FeedbackRecord) -> Self {
        let root = &record.context.project_root;
        let mut identifiers = vec![record.context.project_hash.clone()];
        if let Some(name) = root.file_name().and_then(|s| s.to_str()) {
            identifiers.push(name.into());
        }
        if let Some(origin) = crate::repo_hash::repository_local_origin(root) {
            let parts: Vec<_> = origin
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .rsplit(['/', ':'])
                .take(2)
                .collect();
            identifiers.extend(parts.into_iter().map(str::to_owned));
        }
        let dot_git = root.join(".git");
        let git_dir = if dot_git.is_dir() {
            dot_git
        } else {
            fs::read_to_string(&dot_git)
                .ok()
                .and_then(|s| {
                    s.trim()
                        .strip_prefix("gitdir:")
                        .map(str::trim)
                        .map(PathBuf::from)
                })
                .map(|p| if p.is_absolute() { p } else { root.join(p) })
                .unwrap_or_default()
        };
        if let Ok(head) = fs::read_to_string(git_dir.join("HEAD")) {
            if let Some(branch) = head.trim().strip_prefix("ref: refs/heads/") {
                identifiers.push(branch.into());
            }
        }
        identifiers
            .retain(|s| !s.is_empty() && !matches!(s.as_str(), "gwt" | "akiojin" | "github.com"));
        identifiers.sort_by_key(|s| std::cmp::Reverse(s.len()));
        identifiers.dedup();
        Self {
            identifiers,
            root: root.to_string_lossy().into_owned(),
            removed: Default::default(),
        }
    }

    fn text(&mut self, text: &str) -> String {
        use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};
        use regex::Regex;
        let mut masked = String::new();
        let mut copied = 0;
        for (event, range) in Parser::new(text).into_offset_iter() {
            // Preserve diagnostic literals such as `workspace.update`, status
            // codes and CLI flags. Only source-shaped spans are code masks.
            let source = match &event {
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language)))
                    if !language.is_empty()
                        && !matches!(
                            language.as_ref(),
                            "text" | "log" | "console" | "json" | "sh" | "bash" | "shell"
                        ) =>
                {
                    true
                }
                Event::Start(Tag::CodeBlock(_)) | Event::Code(_) => {
                    looks_like_source(&text[range.clone()])
                }
                _ => false,
            };
            if source && range.start >= copied {
                masked.push_str(&text[copied..range.start]);
                masked.push_str("[REDACTED CODE]");
                masked.extend(
                    text[range.clone()]
                        .chars()
                        .filter(|c| matches!(c, '\r' | '\n')),
                );
                copied = range.end;
                self.removed.insert("source code");
            }
        }
        masked.push_str(&text[copied..]);
        if !self.root.is_empty() && masked.contains(&self.root) {
            masked = masked.replace(&self.root, "[REDACTED PATH]");
            self.removed.insert("file paths");
        }
        for identifier in &self.identifiers {
            let boundary = |c: Option<char>| {
                if c.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    r"\b"
                } else {
                    ""
                }
            };
            let pattern = Regex::new(&format!(
                "(?i){}{}{}",
                boundary(identifier.chars().next()),
                regex::escape(identifier),
                boundary(identifier.chars().last())
            ))
            .expect("source identifier");
            if pattern.is_match(&masked) {
                masked = pattern
                    .replace_all(&masked, "[REDACTED PROJECT]")
                    .into_owned();
                self.removed.insert("project and repository identifiers");
            }
        }
        static REFS: OnceLock<Regex> = OnceLock::new();
        let refs = REFS.get_or_init(|| Regex::new(r"(?i)\b(?:commit|sha|head(?: stayed at)?)[ \t:=]+[0-9a-f]{7,64}\b|\b[0-9a-f]{40,64}\b|(?:\b(?:issue|pr)[ \t]*|#)#?[0-9]+\b|\b(?:branch|ref)[ \t:=]+[^\s,;`]+" ).expect("source references"));
        if refs.is_match(&masked) {
            masked = refs
                .replace_all(&masked, |c: &regex::Captures<'_>| {
                    if c[0].chars().all(|c| c.is_ascii_digit()) {
                        c[0].to_owned()
                    } else {
                        "[REDACTED REFERENCE]".into()
                    }
                })
                .into_owned();
            self.removed
                .insert("commit hashes, branches and issue references");
        }
        let masked = mask_environment(&masked, &mut self.removed);
        static BEARER: OnceLock<Regex> = OnceLock::new();
        let bearer =
            BEARER.get_or_init(|| Regex::new(r"(?i)\bBearer[ \t]+[^\s]+").expect("bearer values"));
        static PATHS: OnceLock<Regex> = OnceLock::new();
        let paths = PATHS.get_or_init(|| {
            Regex::new(r#"(?x)["'](?:[A-Za-z]:[\\/]|\\\\|~[\\/]|\.{0,2}/|[\p{L}\p{N}_.-]+[\\/])[^"'\r\n]*["']|[A-Za-z][A-Za-z0-9+.-]*://[^\s<>"'`]+|(?:[A-Za-z]:[\\/]|\\\\|~[\\/]|\.{0,2}/|[\p{L}\p{N}_.-]+[\\/])[^\s<>"'`]*"#).expect("private paths")
        });
        let mut public = String::new();
        for line in masked.split_inclusive(['\r', '\n']) {
            let content_len = line.trim_end_matches(['\r', '\n']).len();
            let (content, ending) = line.split_at(content_len);
            let content = bearer.replace_all(content, "[REDACTED]");
            let hidden = crate::error_ledger::redact_credentials(&content);
            if hidden != content {
                self.removed.insert("credentials");
            }
            if paths.is_match(&hidden) {
                self.removed.insert("file paths and branch paths");
            }
            public.push_str(&paths.replace_all(&hidden, "[REDACTED PATH]"));
            public.push_str(ending);
        }
        public
    }
}

fn mask_environment(text: &str, removed: &mut std::collections::BTreeSet<&'static str>) -> String {
    use regex::Regex;
    static ENVIRONMENT: OnceLock<Regex> = OnceLock::new();
    let environment = ENVIRONMENT.get_or_init(|| {
            Regex::new(r#"(?P<key>\b[A-Za-z_][A-Za-z0-9_]*[ \t]*=[ \t]*|["'][A-Z_][A-Z0-9_]*["'][ \t]*:\s*)(?P<value>(?:"(?:\\[\s\S]|[^"\\])*"?|'[^']*'?|(?:\\(?:\r\n|[\s\S])?|[^"'\\\r\n])+)+)"#).expect("environment values")
        });
    let mut public = String::new();
    let mut rest = text;
    while let Some(c) = environment.captures(rest) {
        let span = c.get(0).expect("whole assignment");
        public.push_str(&rest[..span.start()]);
        let key = c["key"].trim().trim_end_matches('=').trim();
        let value = c.name("value").expect("assignment value");
        let token_len = value
            .as_str()
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':')))
            .unwrap_or(value.as_str().len());
        if diagnostic_key(key) && token_len > 0 {
            public.push_str(&rest[span.start()..value.start() + token_len]);
            rest = &rest[value.start() + token_len..];
            continue;
        }
        removed.insert("environment values");
        if matches!(key, "project_hash" | "project_root" | "active_agents") {
            public.push_str("[REDACTED PROJECT]");
        } else {
            public.push_str(&c["key"]);
            public.push_str("[REDACTED]");
        }
        public.extend(value.as_str().chars().filter(|c| matches!(c, '\r' | '\n')));
        rest = &rest[span.end()..];
    }
    public.push_str(rest);
    public
}

fn diagnostic_key(key: &str) -> bool {
    matches!(
        key,
        "http_status"
            | "exit_code"
            | "code"
            | "reason"
            | "status"
            | "state"
            | "operation"
            | "launch_route"
            | "binding_state"
            | "diagnostic_reason"
            | "bridge_reason"
            | "schema_version"
            | "execution_id"
            | "generation_id"
            | "request_id"
            | "timeout_ns"
            | "timeout_ms"
            | "retry_after_ms"
            | "timestamp"
    )
}

fn looks_like_source(text: &str) -> bool {
    let assignment = text.lines().any(|line| {
        line.split_once('=').is_some_and(|(key, _)| {
            let key = key.trim().trim_start_matches('`').trim();
            !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !diagnostic_key(key)
        })
    });
    assignment
        || text.contains("()")
        || text.contains("=>")
        || text.contains("{\n")
        || text.contains("{\r\n")
        || text.lines().any(|line| {
            [
                "fn ",
                "pub fn ",
                "function ",
                "const ",
                "let ",
                "class ",
                "def ",
                "import ",
            ]
            .iter()
            .any(|prefix| line.trim_start().starts_with(prefix))
        })
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

pub enum StoredFeedback {
    Manual(Box<FeedbackRecord>),
    Auto(AutoFeedbackRecord),
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

    /// A tag is never interpreted as a legacy raw receipt, even if invalid.
    pub fn read_entry(&self, id: &str) -> io::Result<StoredFeedback> {
        let bytes = fs::read(self.path(id)?)?;
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid feedback receipt");
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if value.get("mode").is_some() {
            let record: AutoFeedbackRecord =
                serde_json::from_value(value).map_err(|_| invalid())?;
            if record.snapshot.id() != id {
                return Err(invalid());
            }
            Ok(StoredFeedback::Auto(record))
        } else {
            let record: FeedbackRecord = serde_json::from_value(value).map_err(|_| invalid())?;
            if record.id != id {
                return Err(invalid());
            }
            Ok(StoredFeedback::Manual(Box::new(record)))
        }
    }

    pub fn read_auto(&self, id: &str) -> io::Result<AutoFeedbackRecord> {
        match self.read_entry(id)? {
            StoredFeedback::Auto(record) => Ok(record),
            StoredFeedback::Manual(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an automatic feedback receipt",
            )),
        }
    }

    pub fn save_auto(&self, record: &AutoFeedbackRecord) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        durable_write(
            &self.path(&record.snapshot.id())?,
            &serde_json::to_vec_pretty(record)?,
        )
    }

    pub fn submit_auto(
        &self,
        report: SafeAutoReport,
        now: DateTime<Utc>,
    ) -> io::Result<(AutoFeedbackRecord, bool)> {
        self.submit_auto_intake(report, now, None)
    }

    /// Unavailable authority permits safe local storage, never a sender. Set
    /// the initial state inside the intake lock, before any reader sees it.
    pub fn submit_auto_withheld(
        &self,
        report: SafeAutoReport,
        now: DateTime<Utc>,
        reason: AutoReason,
    ) -> io::Result<(AutoFeedbackRecord, bool)> {
        self.submit_auto_intake(report, now, Some(reason))
    }

    fn submit_auto_intake(
        &self,
        report: SafeAutoReport,
        now: DateTime<Utc>,
        withheld: Option<AutoReason>,
    ) -> io::Result<(AutoFeedbackRecord, bool)> {
        fs::create_dir_all(self.root.join("fingerprints"))?;
        let lock = self.open_lock("intake")?;
        lock.lock_exclusive()?;
        // No raw fields, source identity, clock or request id enters this key.
        let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&report)?));
        let index = self
            .root
            .join("fingerprints")
            .join(format!("auto-{fingerprint}.json"));
        if index.exists() {
            let receipt: Receipt = serde_json::from_slice(&fs::read(&index)?).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid automatic feedback index",
                )
            })?;
            // Auto snapshots remain deduplicated in every terminal/uncertain
            // state. Only explicit retry can restart a known failed attempt.
            return Ok((self.read_auto(&receipt.id)?, false));
        }
        let mut record = AutoFeedbackRecord::new(report, now);
        if let Some(reason) = withheld {
            record.state = FeedbackState::Withheld;
            record.error = Some(reason);
        }
        self.save_auto(&record)?;
        durable_write(
            &index,
            &serde_json::to_vec(&Receipt {
                id: record.snapshot.id(),
                created_at: now,
            })?,
        )?;
        Ok((record, true))
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
            &input.error_record_id,
            &input.labels,
            &context,
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
