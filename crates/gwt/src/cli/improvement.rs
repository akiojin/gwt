use std::{
    fs, io,
    path::{Path, PathBuf},
};

use chrono::Utc;
use gwt_github::{cache::write_atomic, client::ApiError, SpecOpsError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::CliEnv;

const UPSTREAM_REPOSITORY: &str = "akiojin/gwt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImprovementCommand {
    Capture(ImprovementCaptureCommand),
    List(ImprovementListCommand),
    Dismiss(ImprovementDismissCommand),
    LinkIssue(ImprovementLinkIssueCommand),
    PromoteIssue(ImprovementPromoteIssueCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementCaptureCommand {
    pub source: String,
    pub target_artifact: String,
    pub classification: String,
    pub confidence: String,
    pub summary: String,
    pub details: Option<String>,
    pub evidence_digest: Option<String>,
    pub dedupe_key: Option<String>,
    pub local_evidence: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementListCommand {
    pub state: Option<String>,
    pub classification: Option<String>,
    pub confidence: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementDismissCommand {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementLinkIssueCommand {
    pub id: String,
    pub number: u64,
    pub url: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementPromoteIssueCommand {
    pub id: String,
    pub force: bool,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CandidateStore {
    #[serde(default)]
    candidates: Vec<ImprovementCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ImprovementCandidate {
    id: String,
    created_at: String,
    updated_at: String,
    source: String,
    target_artifact: String,
    classification: String,
    confidence: String,
    state: String,
    dedupe_key: String,
    occurrences: u64,
    sanitized_summary: String,
    sanitized_details: Option<String>,
    evidence_digest: Option<String>,
    local_evidence: Vec<LocalEvidenceReference>,
    linked_issue: Option<LinkedIssue>,
    dismissed_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingImprovementStopCandidate {
    pub id: String,
    pub summary: String,
    pub target_artifact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalEvidenceReference {
    kind: String,
    path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LinkedIssue {
    number: u64,
    url: String,
    repository: String,
}

pub fn run<E: CliEnv>(
    env: &mut E,
    command: ImprovementCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let code = match command {
        ImprovementCommand::Capture(command) => capture(env, command, out)?,
        ImprovementCommand::List(command) => list(env.repo_path(), command, out)?,
        ImprovementCommand::Dismiss(command) => dismiss(env.repo_path(), command, out)?,
        ImprovementCommand::LinkIssue(command) => link_issue(env.repo_path(), command, out)?,
        ImprovementCommand::PromoteIssue(command) => promote_issue(env, command, out)?,
    };
    Ok(code)
}

pub(crate) fn candidate_store_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".gwt")
        .join("improvements")
        .join("candidates.json")
}

pub(crate) fn pending_high_confidence_contract_violations(
    repo_root: &Path,
) -> Vec<PendingImprovementStopCandidate> {
    load_store(repo_root)
        .map(|store| {
            store
                .candidates
                .into_iter()
                .filter(|candidate| {
                    candidate.state == "pending"
                        && candidate.classification == "gwt-caused"
                        && candidate.confidence == "high"
                        && is_contract_artifact(&candidate.target_artifact)
                })
                .map(|candidate| PendingImprovementStopCandidate {
                    id: candidate.id,
                    summary: candidate.sanitized_summary,
                    target_artifact: candidate.target_artifact,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn candidate_public_values(repo_root: &Path) -> Vec<Value> {
    let mut candidates = load_store(repo_root)
        .map(|store| store.candidates)
        .unwrap_or_default();
    candidates.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    candidates
        .into_iter()
        .map(|candidate| candidate_public_json(&candidate))
        .collect()
}

fn is_contract_artifact(target_artifact: &str) -> bool {
    matches!(
        target_artifact,
        "skill"
            | "AGENTS"
            | "hook"
            | "launch"
            | "index"
            | "verification"
            | "coordination"
            | "issue-spec-workflow"
    )
}

fn capture<E: CliEnv>(
    env: &mut E,
    command: ImprovementCaptureCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    validate_enum(
        "source",
        &command.source,
        &[
            "agent-failure",
            "user-correction",
            "review-feedback",
            "hook-runtime",
            "verification",
            "manual",
        ],
    )?;
    validate_enum(
        "target_artifact",
        &command.target_artifact,
        &[
            "skill",
            "AGENTS",
            "hook",
            "launch",
            "index",
            "verification",
            "coordination",
            "issue-spec-workflow",
            "unknown",
        ],
    )?;
    validate_enum(
        "classification",
        &command.classification,
        &["gwt-caused", "ambiguous", "target-project", "external"],
    )?;
    validate_enum(
        "confidence",
        &command.confidence,
        &["low", "medium", "high"],
    )?;
    if command.summary.trim().is_empty() {
        return Err(invalid("summary must not be empty"));
    }

    let now = Utc::now().to_rfc3339();
    let sanitized_summary = sanitize_text(&command.summary);
    let repo_root = env.repo_path().to_path_buf();
    let dedupe_key = command
        .dedupe_key
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| default_dedupe_key(&command, &sanitized_summary));
    let mut store = load_store(&repo_root)?;
    let mut updated_existing = false;
    let candidate = if let Some(candidate) = store
        .candidates
        .iter_mut()
        .find(|candidate| candidate.dedupe_key == dedupe_key)
    {
        candidate.updated_at = now;
        candidate.occurrences += 1;
        candidate.sanitized_summary = sanitized_summary;
        candidate.sanitized_details = command.details.as_deref().map(sanitize_text);
        candidate.evidence_digest = command.evidence_digest.as_deref().map(sanitize_text);
        candidate.local_evidence = sanitize_local_evidence(&command.local_evidence);
        updated_existing = true;
        candidate.clone()
    } else {
        let candidate = ImprovementCandidate {
            id: format!("impr-{}", Uuid::new_v4().simple()),
            created_at: now.clone(),
            updated_at: now,
            source: command.source,
            target_artifact: command.target_artifact,
            classification: command.classification,
            confidence: command.confidence,
            state: "pending".to_string(),
            dedupe_key,
            occurrences: 1,
            sanitized_summary,
            sanitized_details: command.details.as_deref().map(sanitize_text),
            evidence_digest: command.evidence_digest.as_deref().map(sanitize_text),
            local_evidence: sanitize_local_evidence(&command.local_evidence),
            linked_issue: None,
            dismissed_reason: None,
        };
        store.candidates.push(candidate.clone());
        candidate
    };
    save_store(&repo_root, &store)?;
    if should_publish_capture_status(&candidate) {
        post_candidate_captured_status(env, &candidate, updated_existing)?;
    }
    write_json(
        out,
        json!({
            "id": candidate.id,
            "state": candidate.state,
            "dedupe_key": candidate.dedupe_key,
            "occurrences": candidate.occurrences,
            "updated": updated_existing,
        }),
    )?;
    Ok(0)
}

fn list(
    repo_root: &Path,
    command: ImprovementListCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let mut candidates = load_store(repo_root)?.candidates;
    candidates.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let mut values = Vec::new();
    for candidate in candidates {
        if let Some(state) = &command.state {
            if &candidate.state != state {
                continue;
            }
        }
        if let Some(classification) = &command.classification {
            if &candidate.classification != classification {
                continue;
            }
        }
        if let Some(confidence) = &command.confidence {
            if &candidate.confidence != confidence {
                continue;
            }
        }
        values.push(candidate_public_json(&candidate));
        if let Some(limit) = command.limit {
            if values.len() >= limit {
                break;
            }
        }
    }
    write_json(out, json!({ "candidates": values }))?;
    Ok(0)
}

fn dismiss(
    repo_root: &Path,
    command: ImprovementDismissCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if command.reason.trim().is_empty() {
        return Err(invalid("dismiss reason must not be empty"));
    }
    let mut store = load_store(repo_root)?;
    let now = Utc::now().to_rfc3339();
    let candidate = find_candidate_mut(&mut store, &command.id)?;
    candidate.state = "dismissed".to_string();
    candidate.updated_at = now;
    candidate.dismissed_reason = Some(sanitize_text(&command.reason));
    let response = candidate_public_json(candidate);
    save_store(repo_root, &store)?;
    write_json(out, response)?;
    Ok(0)
}

fn link_issue(
    repo_root: &Path,
    command: ImprovementLinkIssueCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let repository = command
        .repository
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| UPSTREAM_REPOSITORY.to_string());
    let url = command
        .url
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "https://github.com/{repository}/issues/{number}",
                number = command.number
            )
        });
    let mut store = load_store(repo_root)?;
    let now = Utc::now().to_rfc3339();
    let candidate = find_candidate_mut(&mut store, &command.id)?;
    candidate.state = "linked".to_string();
    candidate.updated_at = now;
    candidate.linked_issue = Some(LinkedIssue {
        number: command.number,
        url: sanitize_text(&url),
        repository: sanitize_text(&repository),
    });
    let response = candidate_public_json(candidate);
    save_store(repo_root, &store)?;
    write_json(out, response)?;
    Ok(0)
}

fn promote_issue<E: CliEnv>(
    env: &mut E,
    command: ImprovementPromoteIssueCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let repo_root = env.repo_path().to_path_buf();
    let mut store = load_store(&repo_root)?;
    let Some(index) = store
        .candidates
        .iter()
        .position(|candidate| candidate.id == command.id)
    else {
        return Err(invalid("candidate not found"));
    };
    let candidate = store.candidates[index].clone();
    if candidate.state == "dismissed" {
        return Err(invalid("dismissed candidates cannot be promoted"));
    }
    if let Some(linked) = &candidate.linked_issue {
        if candidate.state == "promoted" || candidate.state == "linked" {
            write_json(
                out,
                json!({
                    "id": candidate.id,
                    "state": candidate.state,
                    "issue_number": linked.number,
                    "issue_url": linked.url,
                    "repository": linked.repository,
                    "already_linked": true,
                }),
            )?;
            return Ok(0);
        }
    }
    if !command.force && candidate.classification != "gwt-caused" {
        return Err(invalid(
            "candidate is not classified as gwt-caused; pass force:true to promote",
        ));
    }
    if !command.force && candidate.confidence == "low" {
        return Err(invalid(
            "low-confidence candidates require force:true to promote",
        ));
    }
    let title = issue_title(&candidate);
    let body = render_public_issue_body(&candidate);
    let snapshot = env
        .create_issue_in_repo("akiojin", "gwt", &title, &body, &command.labels)
        .map_err(io_as_spec_error)?;
    let issue_url = format!(
        "https://github.com/{UPSTREAM_REPOSITORY}/issues/{number}",
        number = snapshot.number.0
    );
    let now = Utc::now().to_rfc3339();
    let linked = LinkedIssue {
        number: snapshot.number.0,
        url: issue_url.clone(),
        repository: UPSTREAM_REPOSITORY.to_string(),
    };
    store.candidates[index].state = "promoted".to_string();
    store.candidates[index].updated_at = now;
    store.candidates[index].linked_issue = Some(linked);
    save_store(&repo_root, &store)?;
    post_candidate_promoted_status(&mut *env, &candidate, snapshot.number.0, &issue_url)?;
    write_json(
        out,
        json!({
            "id": candidate.id,
            "state": "promoted",
            "repository": UPSTREAM_REPOSITORY,
            "issue_number": snapshot.number.0,
            "issue_url": issue_url,
        }),
    )?;
    Ok(0)
}

fn should_publish_capture_status(candidate: &ImprovementCandidate) -> bool {
    candidate.confidence == "high" && candidate.classification == "gwt-caused"
}

fn post_candidate_captured_status<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    updated_existing: bool,
) -> Result<(), SpecOpsError> {
    let status = if updated_existing {
        "updated"
    } else {
        "captured"
    };
    let body = format!(
        "Current state: Improvement Candidate {id} was {status} with high confidence for `{target}`.\n\nReason: {summary}\n\nNext: Review it in Improvement Inbox, promote it to an upstream gwt Issue, or dismiss it with a reason.",
        id = candidate.id,
        target = candidate.target_artifact,
        summary = candidate.sanitized_summary,
    );
    post_improvement_board_status(env, body)
}

fn post_candidate_promoted_status<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    issue_number: u64,
    issue_url: &str,
) -> Result<(), SpecOpsError> {
    let body = format!(
        "Current state: Improvement Candidate {id} was promoted to akiojin/gwt Issue #{issue_number}.\n\nReason: {summary}\n\nNext: Track the follow-up in {issue_url}.",
        id = candidate.id,
        summary = candidate.sanitized_summary,
    );
    post_improvement_board_status(env, body)
}

fn post_improvement_board_status<E: CliEnv>(env: &mut E, body: String) -> Result<(), SpecOpsError> {
    let mut board_out = String::new();
    super::board::run(
        env,
        super::BoardCommand::Post(Box::new(super::BoardPostCommand {
            kind: "status".to_string(),
            body: Some(body),
            file: None,
            title: Some("Improvement Candidate".to_string()),
            title_summary: Some("Improvement Candidate".to_string()),
            parent: None,
            topics: vec!["improvement".to_string()],
            owners: vec!["SPEC-3164".to_string()],
            targets: Vec::new(),
            mentions: Vec::new(),
            broadcast: true,
        })),
        &mut board_out,
    )?;
    Ok(())
}

fn issue_title(candidate: &ImprovementCandidate) -> String {
    let summary = truncate_chars(&candidate.sanitized_summary, 90);
    format!("fix(gwt): {summary}")
}

fn render_public_issue_body(candidate: &ImprovementCandidate) -> String {
    let mut body = String::new();
    body.push_str("## Problem\n\n");
    body.push_str(&candidate.sanitized_summary);
    body.push_str("\n\n");
    match &candidate.sanitized_details {
        Some(details) if !details.trim().is_empty() => {
            body.push_str("Context:\n\n");
            body.push_str(details);
            body.push_str("\n\n");
        }
        _ => {
            body.push_str("Context:\n\nNo public-safe details were provided.\n\n");
        }
    }
    body.push_str("## Expected behavior\n\n");
    body.push_str(&format!(
        "gwt should handle `{target}` self-improvement failures with enough public-safe context for maintainers to reproduce, prioritize, and implement the fix without relying on private local logs.\n",
        target = candidate.target_artifact
    ));
    body.push_str("\n## Observed evidence\n\n");
    match &candidate.evidence_digest {
        Some(digest) if !digest.trim().is_empty() => body.push_str(digest),
        _ => body.push_str("No public-safe evidence digest was provided."),
    }
    body.push_str(&format!(
        "\n\nPublic metadata reports {occurrences} occurrence(s), classification `{classification}`, confidence `{confidence}`, and source `{source}`.",
        occurrences = candidate.occurrences,
        classification = candidate.classification,
        confidence = candidate.confidence,
        source = candidate.source
    ));
    body.push_str("\n\n## Impact\n\n");
    body.push_str(&format!(
        "If this remains unresolved, gwt-caused `{target}` regressions can stay local to an agent session instead of becoming trackable upstream work. This increases the chance that the same failure is repeated by future agents or users.",
        target = candidate.target_artifact
    ));
    body.push_str("\n\n## Suggested verification\n\n");
    body.push_str(&format!(
        "- Confirm whether the gwt-owned {target} behavior still violates the expected contract.\n",
        target = candidate.target_artifact
    ));
    body.push_str("- Reproduce the candidate trigger or inspect the sanitized evidence digest.\n");
    body.push_str("- Add or update a regression test that fails before the fix.\n");
    body.push_str(
        "- Verify the fix without requiring private target-project logs, paths, or secrets.\n",
    );
    body.push_str("\n## Source candidate\n\n");
    body.push_str(&format!("- Candidate ID: {}\n", candidate.id));
    body.push_str(&format!("- Source: {}\n", candidate.source));
    body.push_str(&format!(
        "- Target artifact: {}\n",
        candidate.target_artifact
    ));
    body.push_str(&format!("- Classification: {}\n", candidate.classification));
    body.push_str(&format!("- Confidence: {}\n", candidate.confidence));
    body.push_str(&format!("- Dedupe key: {}\n", candidate.dedupe_key));
    body.push_str(&format!("- Occurrences: {}\n", candidate.occurrences));
    body.push_str("\n\n## Privacy\n\n");
    body.push_str("- Public body generated from sanitized candidate fields only.\n");
    body.push_str("- Raw target project paths, repository names, secrets, logs, and code excerpts are local-only.\n");
    body.push_str(&format!(
        "- Local evidence references: {}\n",
        candidate.local_evidence.len()
    ));
    body
}

fn find_candidate_mut<'a>(
    store: &'a mut CandidateStore,
    id: &str,
) -> Result<&'a mut ImprovementCandidate, SpecOpsError> {
    store
        .candidates
        .iter_mut()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| invalid("candidate not found"))
}

fn candidate_public_json(candidate: &ImprovementCandidate) -> Value {
    json!({
        "id": candidate.id,
        "state": candidate.state,
        "source": candidate.source,
        "target_artifact": candidate.target_artifact,
        "classification": candidate.classification,
        "confidence": candidate.confidence,
        "dedupe_key": candidate.dedupe_key,
        "occurrences": candidate.occurrences,
        "summary": candidate.sanitized_summary,
        "details": candidate.sanitized_details,
        "evidence_digest": candidate.evidence_digest,
        "linked_issue": candidate.linked_issue,
        "dismissed_reason": candidate.dismissed_reason,
        "updated_at": candidate.updated_at,
        "issue_preview": {
            "repository": UPSTREAM_REPOSITORY,
            "title": issue_title(candidate),
            "body": render_public_issue_body(candidate),
        },
    })
}

fn load_store(repo_root: &Path) -> Result<CandidateStore, SpecOpsError> {
    let path = candidate_store_path(repo_root);
    if !path.exists() {
        return Ok(CandidateStore {
            candidates: Vec::new(),
        });
    }
    let raw = fs::read_to_string(&path).map_err(io_as_spec_error)?;
    serde_json::from_str(&raw).map_err(serde_as_spec_error)
}

fn save_store(repo_root: &Path, store: &CandidateStore) -> Result<(), SpecOpsError> {
    let path = candidate_store_path(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_as_spec_error)?;
    }
    let bytes = serde_json::to_vec_pretty(store).map_err(serde_as_spec_error)?;
    write_atomic(&path, &bytes).map_err(io_as_spec_error)
}

fn sanitize_local_evidence(items: &[Value]) -> Vec<LocalEvidenceReference> {
    items
        .iter()
        .filter_map(|value| {
            let object = value.as_object()?;
            let kind = object
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("evidence");
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .map(sanitize_text);
            Some(LocalEvidenceReference {
                kind: sanitize_text(kind),
                path,
            })
        })
        .collect()
}

fn sanitize_text(input: &str) -> String {
    let token_re =
        Regex::new(r"\b(?:gh[pousr]|github_pat)_[A-Za-z0-9_]+\b").expect("valid token regex");
    let path_re = Regex::new(r"(?:/[A-Za-z0-9._\-\s]+){2,}").expect("valid path regex");
    let mut out = token_re.replace_all(input, "[redacted-secret]").to_string();
    out = path_re.replace_all(&out, "[redacted-path]").to_string();
    if out.chars().count() > 2_000 {
        out = truncate_chars(&out, 2_000);
        out.push_str("...[truncated]");
    }
    out
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut out = input.chars().take(max_chars).collect::<String>();
    if input.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn default_dedupe_key(command: &ImprovementCaptureCommand, summary: &str) -> String {
    let slug = summary
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "candidate".to_string()
    } else {
        slug
    };
    format!(
        "{}:{}:{}",
        command.classification, command.target_artifact, slug
    )
}

fn validate_enum(flag: &'static str, value: &str, allowed: &[&str]) -> Result<(), SpecOpsError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(SpecOpsError::from(ApiError::Unexpected(format!(
            "invalid value for {flag}: {value}"
        ))))
    }
}

fn write_json(out: &mut String, value: Value) -> Result<(), SpecOpsError> {
    out.push_str(&serde_json::to_string(&value).map_err(serde_as_spec_error)?);
    out.push('\n');
    Ok(())
}

fn invalid(message: &str) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message.to_string()))
}

fn io_as_spec_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

fn serde_as_spec_error(err: serde_json::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(err.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::cli::{run_collect, BoardCommand, CliCommand, TestEnv};

    fn parse_output(output: &str) -> Value {
        serde_json::from_str(output.trim()).expect("JSON output")
    }

    fn board_entries_json(env: &mut TestEnv) -> Value {
        let (_, board_out) = run_collect(
            env,
            CliCommand::Board(BoardCommand::Show {
                json: true,
                workspace: None,
                all: true,
            }),
        )
        .expect("board show");
        serde_json::from_str(&board_out).expect("board JSON")
    }

    fn board_bodies(env: &mut TestEnv) -> Vec<String> {
        board_entries_json(env)["board"]["entries"]
            .as_array()
            .expect("board entries")
            .iter()
            .map(|entry| {
                entry["body"]
                    .as_str()
                    .expect("board entry body")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn high_confidence_capture_posts_board_status() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let (capture_code, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(ImprovementCaptureCommand {
                source: "agent-failure".to_string(),
                target_artifact: "skill".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "high".to_string(),
                summary: "Skill loop missed a required update".to_string(),
                details: None,
                evidence_digest: Some("Stop hook caught the missing skill update.".to_string()),
                dedupe_key: Some("skill:missed-required-update".to_string()),
                local_evidence: Vec::new(),
            })),
        )
        .expect("capture");

        assert_eq!(capture_code, 0);
        let id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("candidate id")
            .to_string();
        let bodies = board_bodies(&mut env);
        assert_eq!(
            bodies.len(),
            1,
            "capture should post exactly one board entry"
        );
        assert!(
            bodies[0].contains(&id),
            "board entry should mention candidate id"
        );
        assert!(
            bodies[0].contains("Skill loop missed a required update"),
            "board entry should include sanitized summary: {}",
            bodies[0]
        );
        assert!(
            bodies[0].contains("Improvement Candidate"),
            "board entry should identify the improvement candidate: {}",
            bodies[0]
        );
    }

    #[test]
    fn low_confidence_capture_does_not_post_board_status() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(ImprovementCaptureCommand {
                source: "verification".to_string(),
                target_artifact: "verification".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "low".to_string(),
                summary: "Weak verification signal".to_string(),
                details: None,
                evidence_digest: None,
                dedupe_key: Some("verification:weak-signal-board".to_string()),
                local_evidence: Vec::new(),
            })),
        )
        .expect("capture");

        let bodies = board_bodies(&mut env);
        assert!(
            bodies.is_empty(),
            "low-confidence capture should stay local without a board post: {bodies:?}"
        );
    }

    #[test]
    fn promote_issue_posts_board_status_with_upstream_issue() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let (_, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(ImprovementCaptureCommand {
                source: "agent-failure".to_string(),
                target_artifact: "coordination".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "high".to_string(),
                summary: "Board guidance drift".to_string(),
                details: None,
                evidence_digest: Some("Agent missed required Board update.".to_string()),
                dedupe_key: Some("coordination:board-guidance-drift-post".to_string()),
                local_evidence: Vec::new(),
            })),
        )
        .expect("capture");
        let id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("candidate id")
            .to_string();

        run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: id.clone(),
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect("promote");

        let bodies = board_bodies(&mut env);
        assert_eq!(
            bodies.len(),
            2,
            "capture and promote should each post a board status"
        );
        let promoted = bodies.last().expect("promoted board body");
        assert!(promoted.contains(&id), "promoted body should include id");
        assert!(
            promoted.contains("akiojin/gwt Issue #1"),
            "promoted body should include upstream Issue: {promoted}"
        );
        assert!(
            promoted.contains("Board guidance drift"),
            "promoted body should include sanitized summary: {promoted}"
        );
    }

    #[test]
    fn promote_issue_creates_sanitized_issue_in_upstream_gwt_repo() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let skill_path = env.repo_path.join(".codex/skills/gwt-discussion/SKILL.md");
        std::fs::create_dir_all(skill_path.parent().expect("skill parent")).expect("skill dir");
        std::fs::write(&skill_path, "original skill").expect("skill file");

        let (capture_code, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(ImprovementCaptureCommand {
                source: "agent-failure".to_string(),
                target_artifact: "skill".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "high".to_string(),
                summary: "Skill update missing for /Users/alice/private-app".to_string(),
                details: Some(
                    "Failure came from /Users/alice/private-app/AGENTS.md and token ghp_1234567890abcdef."
                        .to_string(),
                ),
                evidence_digest: Some("Stop hook allowed completion without capture.".to_string()),
                dedupe_key: Some("skill:gwt-discussion:self-improvement".to_string()),
                local_evidence: Vec::new(),
            })),
        )
        .expect("capture");
        assert_eq!(capture_code, 0);
        let id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("id")
            .to_string();

        let (promote_code, promote_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: id.clone(),
                    force: false,
                    labels: vec!["bug".to_string()],
                },
            )),
        )
        .expect("promote");

        assert_eq!(promote_code, 0, "promotion output: {promote_out}");
        let output = parse_output(&promote_out);
        assert_eq!(output["state"], "promoted");
        assert_eq!(output["repository"], "akiojin/gwt");
        assert_eq!(env.target_issue_create_call_log.len(), 1);
        let call = &env.target_issue_create_call_log[0];
        assert_eq!(call.owner, "akiojin");
        assert_eq!(call.repo, "gwt");
        assert!(
            !call.body.contains("/Users/alice"),
            "public Issue body must not contain private paths: {}",
            call.body
        );
        assert!(
            !call.body.contains("ghp_1234567890abcdef"),
            "public Issue body must not contain token-like secrets: {}",
            call.body
        );
        assert!(call.body.contains("## Problem"));
        assert!(call.body.contains("## Expected behavior"));
        assert!(call.body.contains("## Observed evidence"));
        assert!(call.body.contains("## Impact"));
        assert!(call.body.contains("## Suggested verification"));
        assert!(call.body.contains("## Source candidate"));
        assert!(call.body.contains("- Target artifact: skill"));
        assert!(call.body.contains("Candidate ID"));
        assert!(call.body.contains(&id));
        assert_eq!(
            std::fs::read_to_string(&skill_path).expect("skill file"),
            "original skill",
            "promotion must not auto-mutate skill files"
        );
    }

    #[test]
    fn list_candidates_includes_sanitized_upstream_issue_preview() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let (capture_code, _capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(ImprovementCaptureCommand {
                source: "agent-failure".to_string(),
                target_artifact: "skill".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "high".to_string(),
                summary: "Skill update missing for /Users/alice/private-app".to_string(),
                details: Some(
                    "Failure came from /Users/alice/private-app/AGENTS.md and token ghp_1234567890abcdef."
                        .to_string(),
                ),
                evidence_digest: Some("Stop hook allowed completion without capture.".to_string()),
                dedupe_key: Some("skill:gwt-discussion:self-improvement".to_string()),
                local_evidence: Vec::new(),
            })),
        )
        .expect("capture");
        assert_eq!(capture_code, 0);

        let (list_code, list_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::List(ImprovementListCommand {
                state: Some("pending".to_string()),
                classification: None,
                confidence: None,
                limit: None,
            })),
        )
        .expect("list");

        assert_eq!(list_code, 0, "list output: {list_out}");
        let output = parse_output(&list_out);
        let preview = &output["candidates"][0]["issue_preview"];
        assert_eq!(preview["repository"], "akiojin/gwt");
        assert!(preview["title"]
            .as_str()
            .expect("preview title")
            .starts_with("fix(gwt): Skill update missing"));
        let body = preview["body"].as_str().expect("preview body");
        assert!(body.contains("## Problem"));
        assert!(body.contains("## Expected behavior"));
        assert!(body.contains("## Observed evidence"));
        assert!(body.contains("## Impact"));
        assert!(body.contains("## Suggested verification"));
        assert!(body.contains("## Source candidate"));
        assert!(body.contains("## Privacy"));
        assert!(body.contains("Stop hook allowed completion without capture."));
        assert!(body.contains(
            "Confirm whether the gwt-owned skill behavior still violates the expected contract."
        ));
        assert!(body.contains("- Target artifact: skill"));
        assert!(
            !body.contains("/Users/alice"),
            "preview body must not contain private paths: {body}"
        );
        assert!(
            !body.contains("ghp_1234567890abcdef"),
            "preview body must not contain token-like secrets: {body}"
        );
    }

    #[test]
    fn promote_issue_requires_gwt_cause_and_enough_confidence_without_force() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let (_, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(ImprovementCaptureCommand {
                source: "verification".to_string(),
                target_artifact: "verification".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "low".to_string(),
                summary: "Weak signal should stay local".to_string(),
                details: None,
                evidence_digest: None,
                dedupe_key: Some("verification:weak-signal".to_string()),
                local_evidence: Vec::new(),
            })),
        )
        .expect("capture");
        let low_id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("low id")
            .to_string();

        let err = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: low_id,
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect_err("low-confidence promotion should be rejected");
        assert!(
            err.to_string().contains("low-confidence"),
            "unexpected error: {err}"
        );
        assert!(env.target_issue_create_call_log.is_empty());

        let (_, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(ImprovementCaptureCommand {
                source: "manual".to_string(),
                target_artifact: "unknown".to_string(),
                classification: "target-project".to_string(),
                confidence: "high".to_string(),
                summary: "Target project failure should not auto-promote".to_string(),
                details: None,
                evidence_digest: None,
                dedupe_key: Some("target-project:not-gwt".to_string()),
                local_evidence: Vec::new(),
            })),
        )
        .expect("capture");
        let target_id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("target id")
            .to_string();
        let err = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: target_id,
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect_err("non-gwt-caused promotion should be rejected");
        assert!(
            err.to_string().contains("not classified as gwt-caused"),
            "unexpected error: {err}"
        );
        assert!(env.target_issue_create_call_log.is_empty());
    }

    #[test]
    fn repeated_capture_preserves_linked_issue_and_promote_is_idempotent() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let command = ImprovementCaptureCommand {
            source: "agent-failure".to_string(),
            target_artifact: "coordination".to_string(),
            classification: "gwt-caused".to_string(),
            confidence: "high".to_string(),
            summary: "Board guidance drift".to_string(),
            details: None,
            evidence_digest: None,
            dedupe_key: Some("coordination:board-guidance-drift".to_string()),
            local_evidence: Vec::new(),
        };
        let (_, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(command.clone())),
        )
        .expect("capture");
        let id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("id")
            .to_string();
        run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: id.clone(),
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect("promote");
        assert_eq!(env.target_issue_create_call_log.len(), 1);

        let (_, recapture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::Capture(command)),
        )
        .expect("recapture");
        let recapture = parse_output(&recapture_out);
        assert_eq!(recapture["id"], id);
        assert_eq!(recapture["occurrences"], 2);

        let (_, promote_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id,
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect("idempotent promote");
        let promoted = parse_output(&promote_out);
        assert_eq!(promoted["already_linked"], true);
        assert_eq!(
            env.target_issue_create_call_log.len(),
            1,
            "already linked candidate must not create duplicate upstream Issues"
        );
    }
}
