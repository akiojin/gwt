//! Machine-local direct PM messages. Reports and prompt context share one source.
//! Caller/recipient authority is established by the CLI, never by this store.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    operation_deadline::NamedFileLock,
    pm_report::{PmReport, PmReportKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerKind {
    Request,
    Reply,
    Notice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerProject {
    pub project: String,
    pub project_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PeerSend {
    pub from: PeerProject,
    pub to: PeerProject,
    pub kind: PeerKind,
    pub text: String,
    pub reply_to: Option<String>,
    pub timeout_seconds: u64,
    pub unavailable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerMessage {
    pub id: String,
    pub from: PeerProject,
    pub to: PeerProject,
    pub kind: PeerKind,
    pub text: String,
    pub reply_to: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub answered_by: Option<String>,
    pub unavailable: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Messages {
    schema_version: u32,
    messages: Vec<PeerMessage>,
}

fn store_path() -> PathBuf {
    crate::paths::gwt_home().join("pm-peer-messages.json")
}

/// A PM's isolated checkout/runtime has no origin; its owning store is the
/// identity. Recognize only the managed shape, including canonical HOME paths.
pub fn project_dir(repo: &Path) -> PathBuf {
    let projects = crate::paths::gwt_projects_dir();
    repo.ancestors()
        .find_map(|path| {
            if !matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("worktree" | "runtime")
            ) {
                return None;
            }
            let pm = path.parent()?;
            if pm.file_name()? != "pm" {
                return None;
            }
            let project = pm.parent()?;
            let parent = project.parent()?;
            let same = parent == projects
                || dunce::canonicalize(parent)
                    .ok()
                    .zip(dunce::canonicalize(&projects).ok())
                    .is_some_and(|(a, b)| a == b);
            same.then(|| projects.join(project.file_name().expect("project component")))
        })
        .unwrap_or_else(|| crate::paths::gwt_project_dir_for_repo_path(repo))
}

fn load() -> io::Result<Messages> {
    match fs::read(store_path()) {
        Ok(bytes) => {
            let store: Messages = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if store.schema_version != 1 {
                return Err(io::Error::other("unsupported PM peer schema"));
            }
            Ok(store)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Messages {
            schema_version: 1,
            messages: vec![],
        }),
        Err(error) => Err(error),
    }
}

pub fn validate(
    kind: PeerKind,
    text: &str,
    reply_to: Option<&str>,
    timeout_seconds: u64,
) -> io::Result<()> {
    if text.trim().is_empty() || text.len() > 1000 {
        return Err(io::Error::other(
            "PM peer text must be nonempty and at most 1000 bytes",
        ));
    }
    // The existing redactor appends an ellipsis past its 1000-character
    // bound. Reject expansion before publication rather than losing conditions.
    if crate::error_ledger::sanitize_error_message(text)
        .chars()
        .count()
        > 1000
    {
        return Err(io::Error::other(
            "PM peer text exceeds the limit after credential redaction; shorten the message",
        ));
    }
    if !(1..=86400).contains(&timeout_seconds) {
        return Err(io::Error::other("PM peer timeout_seconds must be 1..86400"));
    }
    if (kind == PeerKind::Reply) != reply_to.is_some()
        || reply_to.is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err(io::Error::other(
            "only a reply requires reply_to containing a message UUID",
        ));
    }
    Ok(())
}

/// Publish a message and settle its exact original in one locked replacement.
/// An unavailable attempt is retained only for the sender, never as delivery.
pub fn send(request: PeerSend, now: DateTime<Utc>) -> io::Result<PeerMessage> {
    validate(
        request.kind,
        &request.text,
        request.reply_to.as_deref(),
        request.timeout_seconds,
    )?;
    if request.from.project_dir == request.to.project_dir {
        return Err(io::Error::other("PM peer messages require another project"));
    }
    let path = store_path();
    fs::create_dir_all(path.parent().expect("peer store parent"))?;
    let _lock = NamedFileLock::acquire(&path.with_extension("lock"), "pm_peer")?;
    let mut store = load()?;
    let original = if let Some(id) = &request.reply_to {
        Some(
            store
                .messages
                .iter()
                .position(|message| {
                    message.id == *id
                        && message.to.project_dir == request.from.project_dir
                        && message.from.project_dir == request.to.project_dir
                        && message.answered_by.is_none()
                        && message.unavailable.is_none()
                })
                .ok_or_else(|| {
                    io::Error::other(
                        "reply_to is not an unanswered message from the addressed peer",
                    )
                })?,
        )
    } else {
        None
    };
    let message = PeerMessage {
        id: uuid::Uuid::new_v4().to_string(),
        from: request.from,
        to: request.to,
        kind: request.kind,
        text: crate::error_ledger::sanitize_error_message(&request.text),
        reply_to: request.reply_to,
        created_at: now,
        expires_at: now + chrono::Duration::seconds(request.timeout_seconds as i64),
        answered_by: None,
        unavailable: request.unavailable,
    };
    if message.unavailable.is_none() {
        if let Some(index) = original {
            store.messages[index].answered_by = Some(message.id.clone());
        }
    }
    store.messages.push(message.clone());
    let temporary = path.with_file_name(format!(".pm-peer-{}.publishing", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let publication = (|| {
        file.write_all(&serde_json::to_vec(&store).map_err(io::Error::other)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)?;
        #[cfg(unix)]
        fs::File::open(path.parent().expect("peer store parent"))?.sync_all()?;
        Ok::<(), io::Error>(())
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    publication?;
    Ok(message)
}

/// Atomic snapshots need no writer lock. Reading never consumes a message.
pub fn pending(project_dir: &Path) -> io::Result<Vec<PeerMessage>> {
    Ok(load()?
        .messages
        .into_iter()
        .filter(|message| {
            message.to.project_dir == project_dir
                && message.answered_by.is_none()
                && message.unavailable.is_none()
        })
        .collect())
}

/// Deterministic one-line Reports, without a second write or duplicate append.
pub fn reports(project_dir: &Path, now: DateTime<Utc>) -> io::Result<Vec<PmReport>> {
    Ok(load()?
        .messages
        .into_iter()
        .filter_map(|message| {
            let sent = message.from.project_dir == project_dir;
            let received = message.to.project_dir == project_dir && message.unavailable.is_none();
            if !sent && !received {
                return None;
            }
            let waiting = if message.unavailable.is_some() {
                " — 相手プロジェクトの対応待ち（PM 不在・停止または生存未確認）"
            } else if sent
                && message.kind == PeerKind::Request
                && message.answered_by.is_none()
                && now >= message.expires_at
            {
                " — 相手プロジェクトの PM の応答待ち"
            } else {
                ""
            };
            let peer = if sent {
                &message.to.project
            } else {
                &message.from.project
            };
            let body = format!(
                "PM peer {} {} {:?} {}: {}{}",
                message.id,
                if sent { "→" } else { "←" },
                message.kind,
                peer,
                message.text.replace(['\r', '\n'], " "),
                waiting
            );
            Some(PmReport {
                id: format!("pm-peer-{}", message.id),
                kind: PmReportKind::Progress,
                question: None,
                body,
                created_at: message.created_at,
            })
        })
        .collect())
}
