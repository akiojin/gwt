//! Read-only, text-only projection of a PM's native conversation.

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use gwt_agent::{AgentId, Session};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmConversationAvailability {
    Ready,
    Waiting,
    Unsupported,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmConversationRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmConversationMessage {
    pub id: String,
    pub role: PmConversationRole,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmConversationSnapshot {
    pub conversation_id: Option<String>,
    pub availability: PmConversationAvailability,
    pub messages: Vec<PmConversationMessage>,
    pub detail: Option<String>,
}

mod claude;
mod codex;

const MAX_RECORD_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MESSAGES: usize = 200;
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

impl PmConversationSnapshot {
    fn empty(
        id: Option<&str>,
        availability: PmConversationAvailability,
        detail: Option<&str>,
    ) -> Self {
        Self {
            conversation_id: id.map(str::to_owned),
            availability,
            messages: Vec::new(),
            detail: detail.map(str::to_owned),
        }
    }
}

/// Reuses one PM's last native source, retaining only bounded visible text.
#[derive(Default)]
pub struct PmConversationReader {
    cached: Option<CachedConversation>,
    #[cfg(test)]
    processed_bytes: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct ConversationKey {
    agent: AgentId,
    id: String,
    cwd: PathBuf,
    home: Option<PathBuf>,
}

impl ConversationKey {
    fn from_session(session: &Session) -> Result<Self, PmConversationSnapshot> {
        let id = session.exact_resume_session_id();
        if !matches!(session.agent_id, AgentId::ClaudeCode | AgentId::Codex)
            || session.runtime_target != gwt_agent::LaunchRuntimeTarget::Host
        {
            return Err(PmConversationSnapshot::empty(
                id,
                PmConversationAvailability::Unsupported,
                Some("This session uses the terminal view."),
            ));
        }
        let Some(id) = id else {
            return Err(PmConversationSnapshot::empty(
                None,
                PmConversationAvailability::Waiting,
                None,
            ));
        };
        // Locators accept path fragments or filename matches. Only native IDs
        // can select a store; no client-supplied path crosses this boundary.
        if !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(PmConversationSnapshot::empty(
                Some(id),
                PmConversationAvailability::Unavailable,
                Some("Conversation identity is invalid."),
            ));
        }
        let home = match session.agent_id {
            AgentId::ClaudeCode => gwt_core::usage::claude::claude_home(),
            AgentId::Codex => session
                .codex_auth_root
                .as_ref()
                .map(|root| root.path.clone())
                .or_else(gwt_core::usage::codex::codex_home),
            _ => None,
        };
        Ok(Self {
            agent: session.agent_id.clone(),
            id: id.to_owned(),
            cwd: session.worktree_path.clone(),
            home,
        })
    }

    fn locate(&self) -> Option<PathBuf> {
        let home = self.home.as_deref()?;
        match self.agent {
            AgentId::ClaudeCode => gwt_core::usage::claude::transcript_for_session(home, &self.id),
            AgentId::Codex => gwt_core::usage::codex::rollout_for_session(home, &self.id),
            _ => None,
        }
    }

    fn unavailable(&self, detail: &'static str) -> PmConversationSnapshot {
        PmConversationSnapshot::empty(
            Some(&self.id),
            PmConversationAvailability::Unavailable,
            Some(detail),
        )
    }

    fn waiting(&self) -> PmConversationSnapshot {
        PmConversationSnapshot::empty(
            Some(&self.id),
            PmConversationAvailability::Waiting,
            Some("Waiting for the conversation store."),
        )
    }
}

#[derive(Clone, PartialEq, Eq)]
struct SourceStamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    /// Volume serial number and NTFS file index. Timestamps alone cannot
    /// distinguish a same-length replacement made within one clock tick.
    #[cfg(windows)]
    identity: Option<(u32, u64)>,
}

impl SourceStamp {
    fn from_file(
        #[cfg_attr(not(windows), allow(unused_variables))] file: &std::fs::File,
        metadata: &std::fs::Metadata,
    ) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(windows)]
            identity: windows_file_identity(file),
        }
    }

    fn replaced(&self, previous: &Self) -> bool {
        if self.created != previous.created {
            return true;
        }
        #[cfg(unix)]
        if self.device != previous.device || self.inode != previous.inode {
            return true;
        }
        #[cfg(windows)]
        if self.identity != previous.identity {
            return true;
        }
        false
    }
}

#[cfg(windows)]
fn windows_file_identity(file: &std::fs::File) -> Option<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    #[derive(Default)]
    struct WindowsFileTime {
        low_date_time: u32,
        high_date_time: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct WindowsByHandleFileInformation {
        file_attributes: u32,
        creation_time: WindowsFileTime,
        last_access_time: WindowsFileTime,
        last_write_time: WindowsFileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            file: *mut std::ffi::c_void,
            information: *mut WindowsByHandleFileInformation,
        ) -> i32;
    }

    let mut information = WindowsByHandleFileInformation::default();
    // SAFETY: `file` owns a valid Windows handle for the duration of the call,
    // and `information` is writable storage matching the Win32 structure.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    (succeeded != 0).then(|| {
        (
            information.volume_serial_number,
            (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low),
        )
    })
}

struct CachedConversation {
    key: ConversationKey,
    path: PathBuf,
    stamp: SourceStamp,
    offset: u64,
    snapshot: PmConversationSnapshot,
    identity_seen: bool,
}

impl CachedConversation {
    fn snapshot(&self) -> PmConversationSnapshot {
        if self.identity_seen {
            self.snapshot.clone()
        } else {
            self.key
                .unavailable("Conversation identity has not been verified.")
        }
    }
}

impl PmConversationReader {
    /// Call from a worker. Unchanged stores need only metadata; ordinary
    /// append reads begin at the last complete JSONL record, not at byte zero.
    pub fn read_for_session(&mut self, session: &Session) -> PmConversationSnapshot {
        let key = match ConversationKey::from_session(session) {
            Ok(key) => key,
            Err(snapshot) => {
                self.cached = None;
                return snapshot;
            }
        };
        if self.cached.as_ref().is_some_and(|cached| cached.key != key) {
            self.cached = None;
        }
        let Some(mut path) = self
            .cached
            .as_ref()
            .map(|cached| cached.path.clone())
            .or_else(|| key.locate())
        else {
            return key.waiting();
        };
        let mut opened = std::fs::File::open(&path);
        // Native stores may move the exact conversation to a new path. A
        // cached path saves directory scans until that path disappears.
        if opened
            .as_ref()
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            && self.cached.is_some()
        {
            self.cached = None;
            let Some(relocated) = key.locate() else {
                return key.waiting();
            };
            path = relocated;
            opened = std::fs::File::open(&path);
        }
        match opened {
            Ok(file) => self.read_file(key, path, file),
            Err(error) => {
                self.cached = None;
                if error.kind() == std::io::ErrorKind::NotFound {
                    key.waiting()
                } else {
                    key.unavailable("The conversation store is not readable.")
                }
            }
        }
    }

    fn read_file(
        &mut self,
        key: ConversationKey,
        path: PathBuf,
        mut file: std::fs::File,
    ) -> PmConversationSnapshot {
        use std::io::{Read, Seek, SeekFrom};
        let stamp = match file.metadata() {
            Ok(metadata) => SourceStamp::from_file(&file, &metadata),
            Err(_) => {
                self.cached = None;
                return key.unavailable("The conversation store is not readable.");
            }
        };
        if let Some(cached) = &self.cached {
            if cached.key == key && cached.path == path && cached.stamp == stamp {
                return cached.snapshot();
            }
            // A shorter file, replacement, or same-length rewrite is a new
            // source generation. Only growth of the same file is an append.
            if cached.key != key
                || cached.path != path
                || stamp.replaced(&cached.stamp)
                || stamp.len <= cached.stamp.len
            {
                self.cached = None;
            }
        }
        let cached = self.cached.get_or_insert_with(|| CachedConversation {
            snapshot: PmConversationSnapshot::empty(
                Some(&key.id),
                PmConversationAvailability::Ready,
                None,
            ),
            key: key.clone(),
            path,
            stamp: stamp.clone(),
            offset: 0,
            identity_seen: false,
        });
        let parsed = file
            .seek(SeekFrom::Start(cached.offset))
            .map_err(|_| "The conversation store is not readable.")
            .and_then(|_| {
                scan_records(
                    std::io::BufReader::new(file.take(stamp.len - cached.offset)),
                    &cached.key,
                    &mut cached.snapshot,
                    &mut cached.identity_seen,
                )
            });
        match parsed {
            Ok(bytes) => {
                cached.offset += bytes;
                cached.stamp = stamp;
                #[cfg(test)]
                {
                    self.processed_bytes += bytes;
                }
                cached.snapshot()
            }
            Err(detail) => {
                self.cached = None;
                key.unavailable(detail)
            }
        }
    }
}

/// One-shot access for callers that do not retain a project reader.
pub fn read_for_session(session: &Session) -> PmConversationSnapshot {
    PmConversationReader::default().read_for_session(session)
}

#[cfg(test)]
fn read_path(agent: &AgentId, path: &Path, id: &str, cwd: &Path) -> PmConversationSnapshot {
    match std::fs::File::open(path) {
        Ok(file) => parse_reader(agent, std::io::BufReader::new(file), id, cwd),
        Err(error) => PmConversationSnapshot::empty(
            Some(id),
            if error.kind() == std::io::ErrorKind::NotFound {
                PmConversationAvailability::Waiting
            } else {
                PmConversationAvailability::Unavailable
            },
            Some("The conversation store is not readable."),
        ),
    }
}

fn same_path(actual: &Path, expected: &Path) -> bool {
    if actual == expected {
        return true;
    }
    // Resident PM instruction discovery runs in its canonical runtime sibling,
    // while Session.worktree_path remains the project's PM checkout. This is
    // an explicit launch contract, not permission to read arbitrary siblings.
    let runtime = crate::pm_registry::pm_runtime_dir_for_pm_worktree(expected);
    if runtime.as_deref() == Some(actual) {
        return true;
    }
    let Ok(actual) = actual.canonicalize() else {
        return false;
    };
    expected
        .canonicalize()
        .is_ok_and(|expected| actual == expected)
        || runtime.is_some_and(|runtime| {
            runtime
                .canonicalize()
                .is_ok_and(|runtime| actual == runtime)
        })
}

#[cfg(test)]
fn parse_native(agent: &AgentId, bytes: &[u8], id: &str, cwd: &Path) -> PmConversationSnapshot {
    parse_reader(agent, std::io::Cursor::new(bytes), id, cwd)
}

#[cfg(test)]
fn parse_reader(
    agent: &AgentId,
    reader: impl std::io::BufRead,
    id: &str,
    cwd: &Path,
) -> PmConversationSnapshot {
    let key = ConversationKey {
        agent: agent.clone(),
        id: id.to_owned(),
        cwd: cwd.to_owned(),
        home: None,
    };
    let mut snapshot =
        PmConversationSnapshot::empty(Some(id), PmConversationAvailability::Ready, None);
    let mut identity_seen = false;
    match scan_records(reader, &key, &mut snapshot, &mut identity_seen) {
        Ok(_) if identity_seen => snapshot,
        Ok(_) => key.unavailable("Conversation identity has not been verified."),
        Err(detail) => key.unavailable(detail),
    }
}

fn scan_records(
    mut reader: impl std::io::BufRead,
    key: &ConversationKey,
    snapshot: &mut PmConversationSnapshot,
    identity_seen: &mut bool,
) -> Result<u64, &'static str> {
    use std::io::{BufRead, Read};
    let mut complete_bytes = 0;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let count = reader
            .by_ref()
            .take(MAX_RECORD_BYTES + 1)
            .read_until(b'\n', &mut buffer)
            .map_err(|_| "The conversation store is not readable.")?;
        if count == 0 {
            break;
        }
        if count as u64 > MAX_RECORD_BYTES {
            return Err("A conversation record exceeds the supported size.");
        }
        // Keep the byte position before a partial final record. A later append
        // re-reads that record, while all earlier complete records stay cached.
        if buffer.last() != Some(&b'\n') {
            break;
        }
        complete_bytes += count as u64;
        if buffer.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(&buffer)
            .map_err(|_| "The conversation store contains an unsupported record.")?;
        let message = match key.agent {
            AgentId::ClaudeCode => {
                let message = claude::message(&value, &key.id, &key.cwd)?;
                if matches!(value["type"].as_str(), Some("user" | "assistant"))
                    && value["isSidechain"].as_bool() != Some(true)
                    && value["sessionId"].as_str() == Some(&key.id)
                    && value["cwd"]
                        .as_str()
                        .is_some_and(|path| same_path(Path::new(path), &key.cwd))
                {
                    *identity_seen = true;
                }
                message
            }
            AgentId::Codex => {
                *identity_seen |= codex::validate_identity(&value, &key.id, &key.cwd)?;
                codex::message(&value)?
            }
            _ => return Err("This conversation format is unsupported."),
        };
        let Some(message) = message else {
            continue;
        };
        if message.role == PmConversationRole::User && internal_prompt(&message.text) {
            continue;
        }
        if let Some(existing) = snapshot
            .messages
            .iter_mut()
            .find(|existing| existing.id == message.id)
        {
            *existing = message;
        } else {
            snapshot.messages.push(message);
        }
        while snapshot.messages.len() > MAX_MESSAGES
            || (snapshot.messages.len() > 1
                && snapshot
                    .messages
                    .iter()
                    .map(|message| message.text.len())
                    .sum::<usize>()
                    > MAX_TEXT_BYTES)
        {
            snapshot.messages.remove(0);
            snapshot.detail = Some("Showing the most recent conversation messages.".to_owned());
        }
    }
    Ok(complete_bytes)
}

fn internal_prompt(text: &str) -> bool {
    use crate::pm_registry::{
        PM_CYCLE_REPORTING_CLAUSE, PM_GWTD_EXECUTION_WAKE_CLAUSE, PM_STEERING_WAKE_CLAUSE,
    };
    let text = text.trim();
    if text == "$gwt-pm" || crate::pm_registry::parse_protected_pm_delivery_prompt(text).is_some() {
        return true;
    }
    let suffix = format!(
        " {PM_STEERING_WAKE_CLAUSE} {PM_GWTD_EXECUTION_WAKE_CLAUSE} {PM_CYCLE_REPORTING_CLAUSE}"
    );
    if let Some(body) = text.strip_suffix(&suffix) {
        if body == "[gwt] Scheduled supervision tick: reconcile now — read a fresh `issue.monitor.status` snapshot and inventory open PRs with `pr.list` (stale / SUPERSEDED / owner-Issue-closed rows: digest escalations, never auto-close)." { return true; }
        if let Some(reasons) = body.strip_prefix("[gwt] Monitor activity while the PM was idle (")
            .and_then(|body| body.strip_suffix("). Reconcile now: fresh `issue.monitor.status`, triage new items, inventory PRs with `pr.list` (stale/SUPERSEDED/owner-closed rows: digest, never auto-close).")) {
            return !reasons.is_empty() && !reasons.contains(['\r', '\n']);
        }
    }
    false
}

#[cfg(test)]
mod tests;
