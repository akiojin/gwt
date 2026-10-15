//! Machine-local user instructions, also the source of their paired Q&A reports.

use std::{
    fs,
    fs::OpenOptions,
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{GwtError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserInstructionKind {
    Standing,
    OneOff,
    Question,
}

/// Capture fields supplied with the PM's Q&A post. Summary and answer come
/// from the report's question/body, so the two views cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UserInstructionInput {
    pub original: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub answer: String,
    pub actions: Vec<String>,
    pub issue_numbers: Vec<u64>,
    pub kind: UserInstructionKind,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub authorizations: Vec<crate::pm_authorization::OperationIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInstruction {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub original: String,
    pub summary: String,
    pub answer: String,
    pub actions: Vec<String>,
    pub issue_numbers: Vec<u64>,
    pub kind: UserInstructionKind,
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorizations: Vec<crate::pm_authorization::OperationScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<InstructionAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionAuthority {
    pub session_id: String,
    pub turn_id: String,
    pub prompt_sha256: String,
}

pub fn instructions_path(repo_path: &Path) -> PathBuf {
    crate::paths::gwt_project_dir_for_repo_path(repo_path)
        .join("project-state/user-instructions.json")
}

/// Readers see either complete snapshot and do not block the prompt hook.
pub fn load(repo_path: &Path) -> Result<Vec<UserInstruction>> {
    match fs::read(instructions_path(repo_path)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid user instruction store: {error}"),
            )
            .into()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

/// A single publication saves the instruction, its Q&A report source and all
/// explicit supersession links. The separate lock survives snapshot replacement.
pub fn record(repo_path: &Path, input: UserInstructionInput) -> Result<UserInstruction> {
    record_inner(repo_path, input, None)
}

/// The caller must first match the original against the captured human prompt.
/// Ordinary PM-authored records never become mutation authority implicitly.
pub fn record_from_prompt(
    repo_path: &Path,
    input: UserInstructionInput,
    session_id: &str,
    turn_id: &str,
) -> Result<UserInstruction> {
    if session_id.trim().is_empty() || turn_id.trim().is_empty() {
        return Err(GwtError::Config(
            "human prompt provenance is required".into(),
        ));
    }
    let authority = InstructionAuthority {
        session_id: session_id.into(),
        turn_id: turn_id.into(),
        prompt_sha256: crate::pm_authorization::digest(input.original.as_bytes()),
    };
    record_inner(repo_path, input, Some(authority))
}

fn record_inner(
    repo_path: &Path,
    input: UserInstructionInput,
    authority: Option<InstructionAuthority>,
) -> Result<UserInstruction> {
    if !input.authorizations.is_empty()
        && (authority.is_none() || input.kind == UserInstructionKind::Question)
    {
        return Err(GwtError::Config(
            "scoped authorization requires a recorded human instruction".into(),
        ));
    }
    let authorizations = input
        .authorizations
        .iter()
        .map(crate::pm_authorization::OperationIntent::scope)
        .collect::<std::io::Result<Vec<_>>>()?;
    if [&input.original, &input.summary, &input.answer]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(GwtError::Config(
            "user instruction original, summary and answer must be nonempty".into(),
        ));
    }
    let path = instructions_path(repo_path);
    fs::create_dir_all(path.parent().expect("instruction store parent"))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))?;
    lock.lock_exclusive()?;
    let mut records = load(repo_path)?;
    if let Some(authority) = &authority {
        if let Some(previous) = records.iter().find(|entry| {
            entry.authority.as_ref().is_some_and(|recorded| {
                recorded.session_id == authority.session_id && recorded.turn_id == authority.turn_id
            })
        }) {
            if previous.authority.as_ref() != Some(authority)
                || previous.kind != input.kind
                || previous.authorizations.len() != authorizations.len()
                || !previous
                    .authorizations
                    .iter()
                    .all(|scope| authorizations.contains(scope))
            {
                return Err(GwtError::Config(
                    "a human turn cannot change its captured instruction or authorization scope"
                        .into(),
                ));
            }
            return Ok(previous.clone());
        }
    }
    let saved = UserInstruction {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: Utc::now(),
        original: redact(&input.original),
        summary: redact(&input.summary),
        answer: redact(&input.answer),
        actions: input.actions.iter().map(|value| redact(value)).collect(),
        issue_numbers: input.issue_numbers,
        kind: input.kind,
        superseded_by: None,
        authorizations,
        authority,
    };
    for id in input.supersedes {
        let previous = records
            .iter_mut()
            .find(|entry| entry.id == id)
            .filter(|entry| {
                entry.kind == UserInstructionKind::Standing && entry.superseded_by.is_none()
            })
            .ok_or_else(|| {
                GwtError::Config(format!(
                    "supersedes must name an active standing instruction: {id}"
                ))
            })?;
        previous.superseded_by = Some(saved.id.clone());
    }
    records.push(saved.clone());
    // The existing Work publisher syncs both the payload and its parent.
    crate::workspace_projection::write_atomic(
        &path,
        &serde_json::to_vec(&records)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
    )?;
    Ok(saved)
}

/// Compact durable instructions reloaded at startup and on every PM turn.
pub fn standing_context(repo_path: &Path) -> Result<String> {
    let mut lines = Vec::new();
    for record in load(repo_path)? {
        if record.kind == UserInstructionKind::Standing && record.superseded_by.is_none() {
            let summary: String = record
                .summary
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(200)
                .collect();
            lines.push(format!("- {}: {}", record.id, summary));
        }
    }
    if lines.is_empty() {
        return Ok(String::new());
    }
    Ok(format!("# Active user instructions\n\nFollow these standing instructions. Search memory for the full original and cite applicable instruction IDs in Reports.\n\n{}", lines.join("\n")))
}

fn secret_assignment_regex() -> &'static Regex {
    static ASSIGNMENT: OnceLock<Regex> = OnceLock::new();
    ASSIGNMENT.get_or_init(|| Regex::new(
        r#"(?i)\b([a-z_]*(?:token|password|secret|api[_-]?key|authorization)[a-z_]*["']?\s*[:=]\s*)(?:"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|[^\s"',;]+)"#
    ).expect("secret assignment regex"))
}

pub(crate) fn redact(value: &str) -> String {
    static TOKEN: OnceLock<Regex> = OnceLock::new();
    let token = TOKEN.get_or_init(|| {
        Regex::new(r"\b(?:sk-(?:proj-|ant-)?|github_pat_)[A-Za-z0-9_-]{16,}")
            .expect("API token regex")
    });
    // Redact complete quoted assignments before the line filter can consume
    // their key and leave a whitespace-separated suffix behind.
    let redacted = secret_assignment_regex().replace_all(value, "${1}***redacted***");
    let redacted = crate::process_console::redact_line(&redacted);
    token.replace_all(&redacted, "***redacted***").into_owned()
}

pub(crate) fn redact_json(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(text) => Value::String(redact(&text)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_json).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| {
                    // Treat a JSON field as an assignment using the existing secret-key policy.
                    let public_security_status = matches!(
                        key.as_str(),
                        "secret_scanning" | "secret_scanning_push_protection"
                    ) && value.as_object().is_some_and(|setting| {
                        setting.len() == 1
                            && matches!(
                                setting.get("status").and_then(Value::as_str),
                                Some("enabled" | "disabled")
                            )
                    });
                    let value = if !public_security_status
                        && secret_assignment_regex().is_match(&format!("{key}=value"))
                    {
                        Value::String("***redacted***".into())
                    } else {
                        redact_json(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedGwtHome;

    #[test]
    fn authorization_audit_preserves_public_security_status_and_redacts_secret_values() {
        use crate::pm_authorization::{self as authority, Authorization, OperationIntent};
        use serde_json::{json, Value};
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let status = |state: &str| {
            json!({"security_and_analysis":{
            "secret_scanning":{"status":state},
            "secret_scanning_push_protection":{"status":state}
        },"token":"fixture-credential"})
        };
        let intent = OperationIntent {
            operation: "repository.settings.set".into(),
            params: json!({"security_and_analysis":status("enabled")["security_and_analysis"]}),
        };
        let approval = authority::record_approval(
            &repo,
            &intent,
            "Enable scanning",
            "Disable scanning",
            false,
        )
        .unwrap();
        let attempt = authority::authorize(
            &repo,
            &intent,
            &Authorization {
                approval_id: Some(approval.id),
                instruction_id: None,
            },
            None,
            status("disabled"),
        )
        .unwrap();
        attempt.finish(&repo, true, status("enabled")).unwrap();
        let raw = std::fs::read_to_string(authority::audit_path(&repo)).unwrap();
        assert!(!raw.contains("fixture-credential"));
        let completed: Value = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
        for feature in ["secret_scanning", "secret_scanning_push_protection"] {
            assert_eq!(
                completed["before"]["security_and_analysis"][feature]["status"],
                "disabled"
            );
            assert_eq!(
                completed["after"]["security_and_analysis"][feature]["status"],
                "enabled"
            );
        }
    }

    fn request(original: &str) -> UserInstructionInput {
        UserInstructionInput {
            original: original.into(),
            summary: original.into(),
            answer: "日本語で報告します。".into(),
            actions: vec!["Issue #5342 の報告に適用".into()],
            issue_numbers: vec![5342],
            kind: UserInstructionKind::Standing,
            supersedes: vec![],
            authorizations: vec![],
        }
    }

    #[test]
    fn operation_instructions_require_human_provenance_and_exact_scope() {
        use crate::pm_authorization::{self, Authorization, OperationIntent};
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "update.settings.set".into(),
            params: serde_json::json!({"auto_apply_updates": true}),
        };
        let mut input = request("Enable automatic updates.");
        input.kind = UserInstructionKind::OneOff;
        input.authorizations = vec![intent.clone()];
        assert!(
            record(&repo, input.clone()).is_err(),
            "PM capture alone is not authority"
        );
        let saved = record_from_prompt(&repo, input, "pm-session", "human-turn").unwrap();
        let authorization = Authorization {
            instruction_id: Some(saved.id.clone()),
            approval_id: None,
        };
        let mut different = intent.clone();
        different.params["auto_apply_updates"] = false.into();
        assert!(pm_authorization::authorize(
            &repo,
            &different,
            &authorization,
            None,
            serde_json::Value::Null
        )
        .is_err());
        let attempt = pm_authorization::authorize(
            &repo,
            &intent,
            &authorization,
            None,
            serde_json::Value::Null,
        )
        .unwrap();
        attempt
            .finish(&repo, true, serde_json::json!({"auto_apply_updates":true}))
            .unwrap();
        assert!(
            pm_authorization::authorize(
                &repo,
                &intent,
                &authorization,
                None,
                serde_json::Value::Null
            )
            .is_err(),
            "one-off cannot be replayed"
        );
        let audit = std::fs::read_to_string(pm_authorization::audit_path(&repo)).unwrap();
        assert!(audit.contains(&saved.id));
        assert!(audit.contains("authorized") && audit.contains("succeeded"));
    }

    #[test]
    fn authorization_capture_is_idempotent_for_one_human_turn() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let mut input = request("Enable automatic updates.");
        input.kind = UserInstructionKind::OneOff;
        input.authorizations = vec![crate::pm_authorization::OperationIntent {
            operation: "update.settings.set".into(),
            params: serde_json::json!({"auto_apply_updates":true}),
        }];
        let first = record_from_prompt(&repo, input.clone(), "pm-session", "human-turn").unwrap();
        let retried = record_from_prompt(&repo, input.clone(), "pm-session", "human-turn").unwrap();
        assert_eq!(
            first.id, retried.id,
            "one human turn cannot mint another one-off instruction"
        );
        assert_eq!(load(&repo).unwrap().len(), 1);

        let mut changed = input.clone();
        changed.authorizations[0].params["auto_apply_updates"] = false.into();
        assert!(record_from_prompt(&repo, changed, "pm-session", "human-turn").is_err());
        let mut changed = input.clone();
        changed.kind = UserInstructionKind::Standing;
        assert!(record_from_prompt(&repo, changed, "pm-session", "human-turn").is_err());
        input.original = "A different human prompt".into();
        assert!(record_from_prompt(&repo, input, "pm-session", "human-turn").is_err());
        assert_eq!(load(&repo).unwrap().len(), 1);
    }

    #[test]
    fn native_approval_is_scoped_and_remembered_permission_can_be_revoked() {
        use crate::pm_authorization::{self, Authorization, OperationIntent};
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let intent = OperationIntent {
            operation: "discussion.create".into(),
            params: serde_json::json!({"title":"Board", "body":"Content", "category":"General", "sync_board":true}),
        };
        assert!(pm_authorization::authorize(
            &repo,
            &intent,
            &Authorization::default(),
            None,
            serde_json::Value::Null
        )
        .is_err());
        let one = pm_authorization::record_approval(
            &repo,
            &intent,
            "Create Board",
            "Close the discussion",
            false,
        )
        .unwrap();
        let authorization = Authorization {
            instruction_id: None,
            approval_id: Some(one.id),
        };
        let mut other = intent.clone();
        other.params["title"] = "Different".into();
        assert!(pm_authorization::authorize(
            &repo,
            &other,
            &authorization,
            None,
            serde_json::Value::Null
        )
        .is_err());
        pm_authorization::authorize(
            &repo,
            &intent,
            &authorization,
            None,
            serde_json::Value::Null,
        )
        .unwrap();
        assert!(pm_authorization::authorize(
            &repo,
            &intent,
            &authorization,
            None,
            serde_json::Value::Null
        )
        .is_err());
        let standing = pm_authorization::record_approval(
            &repo,
            &intent,
            "Manage Board",
            "Revoke permission",
            true,
        )
        .unwrap();
        assert!(pm_authorization::authorize(
            &repo,
            &other,
            &Authorization::default(),
            None,
            serde_json::Value::Null
        )
        .is_ok());
        pm_authorization::revoke(&repo, &standing.id).unwrap();
        assert!(pm_authorization::authorize(
            &repo,
            &other,
            &Authorization::default(),
            None,
            serde_json::Value::Null
        )
        .is_err());
        assert!(pm_authorization::authorize(
            &home.path().join("other-repo"),
            &intent,
            &authorization,
            None,
            serde_json::Value::Null
        )
        .is_err());
    }

    #[test]
    fn records_reload_original_and_supersession_with_active_standing_context() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        assert!(load(&repo).unwrap().is_empty());
        let original = "報告は日本語で。\n\n判断の根拠も書いてください。";
        let first = record(&repo, request(original)).unwrap();
        assert_eq!(
            load(&repo).unwrap().as_slice(),
            std::slice::from_ref(&first)
        );
        assert_eq!(first.original, original);
        assert!(standing_context(&repo).unwrap().contains(&first.id));
        let mut next = request("以後は英語で報告してください。");
        next.supersedes.push(first.id.clone());
        let second = record(&repo, next).unwrap();
        let restored = load(&repo).unwrap();
        assert_eq!(restored[0].original, original);
        assert_eq!(
            restored[0].superseded_by.as_deref(),
            Some(second.id.as_str())
        );
        assert_eq!(restored[1], second);
        let context = standing_context(&repo).unwrap();
        assert!(context.contains(&second.id));
        assert!(!context.contains(&first.id));
        assert!(!repo.exists(), "records must remain machine-local");
    }

    #[test]
    fn secrets_are_redacted_in_every_persisted_text_field() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let secret = "ghp_0123456789abcdefghijklmnop";
        let fine_grained = "github_pat_0123456789abcdefghijklmnop_0123456789";
        let mut input = request(&format!(
            "{secret} {fine_grained} password=\"correct horse battery staple\""
        ));
        input.answer = "OPENAI_API_KEY=sk-proj-0123456789abcdefghijklmnop".into();
        input.actions = vec![
            "Authorization: Bearer private-value".into(),
            "token=\"correct horse battery staple\"".into(),
            r#"{"Authorization": "Bearer fixtureCredentialValue0123456789"}"#.into(),
        ];
        let saved = record(&repo, input).unwrap();
        let raw = std::fs::read_to_string(instructions_path(&repo)).unwrap();
        for value in [
            secret,
            fine_grained,
            "sk-proj-0123456789abcdefghijklmnop",
            "private-value",
            "fixtureCredentialValue0123456789",
            "horse",
            "battery",
            "staple",
        ] {
            assert!(!raw.contains(value));
        }
        assert_eq!(load(&repo).unwrap(), [saved]);
    }

    #[test]
    fn instruction_and_qa_report_are_read_from_the_same_atomic_source() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let saved = record(&repo, request("報告は日本語で。")).unwrap();
        let reports = crate::pm_report::load_reports(&repo).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, saved.id);
        assert_eq!(reports[0].body, saved.answer);
        assert_eq!(
            serde_json::to_value(&reports[0]).unwrap()["question"],
            saved.summary
        );
        assert_eq!(serde_json::to_value(&reports[0]).unwrap()["kind"], "qa");
        assert!(
            !crate::pm_report::reports_path(&repo).exists(),
            "Q&A must not require a second write"
        );
    }

    #[test]
    fn invalid_supersession_and_damaged_store_preserve_previous_bytes() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        record(&repo, request("日本語で報告")).unwrap();
        let path = instructions_path(&repo);
        let before = std::fs::read(&path).unwrap();
        let mut invalid = request("英語で報告");
        invalid.supersedes.push("unknown-id".into());
        assert!(record(&repo, invalid).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"broken").unwrap();
        assert!(load(&repo).is_err());
        assert!(record(&repo, request("preserve corruption")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    }
}
