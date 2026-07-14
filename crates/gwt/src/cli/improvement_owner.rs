use std::{collections::BTreeSet, fmt, path::Path, sync::OnceLock};

use regex::Regex;

use super::improvement::{
    improvement_fingerprint, typed_evidence_digest, ImprovementCandidate, TypedFailureEvidence,
};

pub(super) const MAX_PUBLIC_BODY_BYTES: usize = 16 * 1024;
const MAX_PUBLIC_TITLE_CHARS: usize = 180;
const MAX_PUBLIC_SUMMARY_CHARS: usize = 150;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicIssuePayload {
    pub(super) summary: String,
    pub(super) title: String,
    pub(super) body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct PublicCommentPayload {
    pub(super) body: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PublicMutationContext {
    denied_values: Vec<String>,
}

impl PublicMutationContext {
    pub(super) fn for_repo(repo_root: &Path) -> Self {
        let mut denied = BTreeSet::new();
        add_path(&mut denied, repo_root);
        if let Ok(canonical) = std::fs::canonicalize(repo_root) {
            add_path(&mut denied, &canonical);
        }
        if let Ok(main_root) = gwt_git::worktree::main_worktree_root(repo_root) {
            add_path(&mut denied, &main_root);
            if let Ok(worktrees) = gwt_git::WorktreeManager::new(main_root).list() {
                for worktree in worktrees {
                    add_path(&mut denied, &worktree.path);
                }
            }
        }
        if let Some(slug) = source_repository_slug(repo_root) {
            add_denied_value(&mut denied, slug);
        }
        for key in ["HOME", "USERPROFILE", "USER", "USERNAME"] {
            if let Ok(value) = std::env::var(key) {
                add_denied_value(&mut denied, value);
            }
        }
        add_secret_environment_values(&mut denied, std::env::vars());
        for agent in gwt_agent::load_custom_agents_from_path(&gwt_core::paths::gwt_config_path())
            .unwrap_or_default()
        {
            add_secret_environment_values(&mut denied, agent.env);
        }
        Self {
            denied_values: denied.into_iter().collect(),
        }
    }

    fn with_candidate(&self, candidate: &ImprovementCandidate, trusted_values: &[&str]) -> Self {
        let mut denied = self.denied_values.iter().cloned().collect::<BTreeSet<_>>();
        add_denied_value(&mut denied, candidate.sanitized_summary.clone());
        if let Some(details) = &candidate.sanitized_details {
            add_denied_value(&mut denied, details.clone());
        }
        if let Some(evidence_digest) = &candidate.evidence_digest {
            if !trusted_values.contains(&evidence_digest.as_str()) {
                add_denied_value(&mut denied, evidence_digest.clone());
            }
        }
        add_denied_value(&mut denied, candidate.dedupe_key.clone());
        for evidence in &candidate.local_evidence {
            if let Some(path) = &evidence.path {
                add_denied_value(&mut denied, path.clone());
            }
        }
        Self {
            denied_values: denied.into_iter().collect(),
        }
    }

    #[cfg(test)]
    fn from_denied_values_for_test<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut denied = BTreeSet::new();
        for value in values {
            add_denied_value(&mut denied, value.into());
        }
        Self {
            denied_values: denied.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrivacyViolationKind {
    DynamicValue,
    UnixPath,
    WindowsPath,
    Secret,
    Authorization,
    UrlCredential,
    UrlQuerySecret,
    PrivateKey,
    Email,
    LogExcerpt,
    CodeExcerpt,
    SizeLimit,
    InvalidTemplateField,
}

impl PrivacyViolationKind {
    const fn code(self) -> &'static str {
        match self {
            Self::DynamicValue => "DYNAMIC_VALUE",
            Self::UnixPath => "UNIX_PATH",
            Self::WindowsPath => "WINDOWS_PATH",
            Self::Secret => "SECRET",
            Self::Authorization => "AUTHORIZATION",
            Self::UrlCredential => "URL_CREDENTIAL",
            Self::UrlQuerySecret => "URL_QUERY_SECRET",
            Self::PrivateKey => "PRIVATE_KEY",
            Self::Email => "EMAIL",
            Self::LogExcerpt => "LOG_EXCERPT",
            Self::CodeExcerpt => "CODE_EXCERPT",
            Self::SizeLimit => "SIZE_LIMIT",
            Self::InvalidTemplateField => "INVALID_TEMPLATE_FIELD",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrivacyViolation {
    kind: PrivacyViolationKind,
}

impl PrivacyViolation {
    const fn new(kind: PrivacyViolationKind) -> Self {
        Self { kind }
    }

    #[cfg(test)]
    pub(super) const fn kind(&self) -> PrivacyViolationKind {
        self.kind
    }
}

impl fmt::Display for PrivacyViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "public mutation rejected by privacy gate: {}",
            self.kind.code()
        )
    }
}

impl std::error::Error for PrivacyViolation {}

pub(super) fn render_public_issue_payload(
    candidate: &ImprovementCandidate,
    context: &PublicMutationContext,
) -> Result<PublicIssuePayload, PrivacyViolation> {
    validate_candidate_template_fields(candidate)?;
    let (summary, problem, expected, observed, public_identity) = match &candidate.typed_evidence {
        Some(evidence) => {
            let (summary, problem, expected, observed) =
                typed_template_fields(candidate, evidence)?;
            (
                summary,
                problem,
                expected,
                observed,
                Some(typed_public_identity(candidate, evidence)?),
            )
        }
        None => (
            "Self-improvement contract evidence required".to_string(),
            "A legacy self-improvement candidate does not contain typed contract evidence."
                .to_string(),
            "Typed evidence is required before unattended owner resolution.".to_string(),
            "No free-form candidate text is rendered into a public mutation.".to_string(),
            None,
        ),
    };
    let title = format!("fix(gwt): {summary}");
    let identity = public_identity
        .as_ref()
        .map(|identity| {
            format!(
                "- Public evidence digest: sha256:{}\n\n{}\n",
                identity.evidence_digest,
                fingerprint_marker(&identity.fingerprint)
            )
        })
        .unwrap_or_else(|| {
            "- Typed public identity: unavailable for legacy evidence\n".to_string()
        });
    let body = format!(
        "## Problem\n\n{problem}\n\n## Expected behavior\n\n{expected}\n\n## Observed evidence\n\n{observed}\n\n{identity}\n## Impact\n\nA gwt-owned contract failure can recur until it has one verified upstream owner.\n\n## Suggested verification\n\n- Reproduce the typed contract outcome.\n- Add a regression test that fails before the fix.\n- Verify the corrected outcome without private source data.\n\n## Source candidate\n\n- Candidate ID: {id}\n- Target artifact: {target}\n- Classification: {classification}\n- Confidence: {confidence}\n- Occurrences: {occurrences}\n\n## Privacy\n\n- This payload is generated from contract-owned typed fields.\n- Free-form evidence, repository identity, paths, credentials, logs, and code remain local-only.\n",
        id = candidate.id,
        target = candidate.target_artifact,
        classification = candidate.classification,
        confidence = candidate.confidence,
        occurrences = candidate.occurrences,
    );
    let payload = PublicIssuePayload {
        summary,
        title,
        body,
    };
    let trusted_values = public_identity
        .as_ref()
        .map(|identity| {
            vec![
                identity.evidence_digest.as_str(),
                identity.fingerprint.as_str(),
            ]
        })
        .unwrap_or_default();
    validate_public_payload(
        &payload,
        &context.with_candidate(candidate, &trusted_values),
    )?;
    Ok(payload)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypedPublicIdentity {
    fingerprint: String,
    evidence_digest: String,
}

fn typed_public_identity(
    candidate: &ImprovementCandidate,
    evidence: &TypedFailureEvidence,
) -> Result<TypedPublicIdentity, PrivacyViolation> {
    let fingerprint = improvement_fingerprint(evidence);
    if candidate.fingerprint.as_deref() != Some(fingerprint.as_str()) {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    Ok(TypedPublicIdentity {
        fingerprint,
        evidence_digest: typed_evidence_digest(evidence),
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn render_occurrence_comment_payload(
    candidate: &ImprovementCandidate,
    occurrence_key: &str,
    context: &PublicMutationContext,
) -> Result<PublicCommentPayload, PrivacyViolation> {
    validate_candidate_template_fields(candidate)?;
    if !occurrence_key_re().is_match(occurrence_key) {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    let evidence = candidate
        .typed_evidence
        .as_ref()
        .ok_or_else(|| PrivacyViolation::new(PrivacyViolationKind::InvalidTemplateField))?;
    let identity = typed_public_identity(candidate, evidence)?;
    let body = format!(
        "gwt recorded one typed self-improvement occurrence.\n\n- Candidate ID: {id}\n- Public evidence digest: sha256:{digest}\n\n{occurrence_marker}\n{fingerprint_marker}\n",
        id = candidate.id,
        digest = identity.evidence_digest,
        occurrence_marker = occurrence_marker(occurrence_key),
        fingerprint_marker = fingerprint_marker(&identity.fingerprint),
    );
    validate_public_comment(
        &body,
        &context.with_candidate(
            candidate,
            &[&identity.evidence_digest, &identity.fingerprint],
        ),
    )?;
    Ok(PublicCommentPayload { body })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn render_reconciliation_comment_payload(
    candidate: &ImprovementCandidate,
    canonical_number: u64,
    duplicate_number: u64,
    context: &PublicMutationContext,
) -> Result<PublicCommentPayload, PrivacyViolation> {
    validate_candidate_template_fields(candidate)?;
    if canonical_number == 0 || duplicate_number == 0 || canonical_number == duplicate_number {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    let evidence = candidate
        .typed_evidence
        .as_ref()
        .ok_or_else(|| PrivacyViolation::new(PrivacyViolationKind::InvalidTemplateField))?;
    let identity = typed_public_identity(candidate, evidence)?;
    let body = format!(
        "This Issue duplicates the same typed self-improvement owner.\n\n- Canonical owner: #{canonical_number}\n- Duplicate owner: #{duplicate_number}\n- Public evidence digest: sha256:{digest}\n\n<!-- gwt:improvement-reconciliation:v1 canonical:{canonical_number} duplicate:{duplicate_number} -->\n{fingerprint_marker}\n",
        digest = identity.evidence_digest,
        fingerprint_marker = fingerprint_marker(&identity.fingerprint),
    );
    validate_public_comment(
        &body,
        &context.with_candidate(
            candidate,
            &[&identity.evidence_digest, &identity.fingerprint],
        ),
    )?;
    Ok(PublicCommentPayload { body })
}

#[cfg_attr(not(test), allow(dead_code))]
fn validate_public_comment(
    body: &str,
    context: &PublicMutationContext,
) -> Result<(), PrivacyViolation> {
    validate_public_payload(
        &PublicIssuePayload {
            summary: "Typed self-improvement occurrence".to_string(),
            title: "Typed self-improvement occurrence".to_string(),
            body: body.to_string(),
        },
        context,
    )
}

fn fingerprint_marker(fingerprint: &str) -> String {
    format!("<!-- gwt:improvement-fingerprint:v1 {fingerprint} -->")
}

#[cfg_attr(not(test), allow(dead_code))]
fn occurrence_marker(occurrence_key: &str) -> String {
    format!("<!-- gwt:improvement-occurrence:v1 {occurrence_key} -->")
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn exact_fingerprint_markers(body: &str) -> Vec<String> {
    let mut in_code_fence = false;
    body.lines()
        .filter_map(|line| {
            if line.trim_start().starts_with("```") {
                in_code_fence = !in_code_fence;
                return None;
            }
            if in_code_fence {
                return None;
            }
            fingerprint_marker_re()
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().to_string())
        })
        .collect()
}

fn typed_template_fields(
    candidate: &ImprovementCandidate,
    evidence: &TypedFailureEvidence,
) -> Result<(String, String, String, String), PrivacyViolation> {
    for value in [
        evidence.subsystem.as_str(),
        evidence.contract_id.as_str(),
        evidence.failure_code.as_str(),
        evidence.target_artifact.as_str(),
        evidence.expected_outcome.as_str(),
        evidence.observed_outcome.as_str(),
    ] {
        if !machine_token_re().is_match(value) {
            return Err(PrivacyViolation::new(
                PrivacyViolationKind::InvalidTemplateField,
            ));
        }
    }
    if evidence.target_artifact != candidate.target_artifact {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    let summary = truncate_chars(
        &format!("{}: {}", candidate.target_artifact, evidence.failure_code),
        MAX_PUBLIC_SUMMARY_CHARS,
    );
    Ok((
        summary,
        format!(
            "The gwt-owned `{}` contract `{}` reported `{}`.",
            evidence.subsystem, evidence.contract_id, evidence.failure_code
        ),
        format!("`{}`", evidence.expected_outcome),
        format!("`{}`", evidence.observed_outcome),
    ))
}

fn validate_candidate_template_fields(
    candidate: &ImprovementCandidate,
) -> Result<(), PrivacyViolation> {
    let target_valid = [
        "skill",
        "AGENTS",
        "hook",
        "launch",
        "index",
        "verification",
        "coordination",
        "issue-spec-workflow",
        "unknown",
    ]
    .contains(&candidate.target_artifact.as_str());
    let classification_valid = ["gwt-caused", "ambiguous", "target-project", "external"]
        .contains(&candidate.classification.as_str());
    let confidence_valid = ["low", "medium", "high"].contains(&candidate.confidence.as_str());
    let id_valid = candidate.id.len() <= 96
        && candidate.id.starts_with("impr-")
        && candidate
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !target_valid || !classification_valid || !confidence_valid || !id_valid {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    Ok(())
}

pub(super) fn validate_public_payload(
    payload: &PublicIssuePayload,
    context: &PublicMutationContext,
) -> Result<(), PrivacyViolation> {
    if payload.summary.chars().count() > MAX_PUBLIC_SUMMARY_CHARS
        || payload.title.chars().count() > MAX_PUBLIC_TITLE_CHARS
        || payload.title.contains(['\r', '\n'])
        || payload.body.len() > MAX_PUBLIC_BODY_BYTES
    {
        return Err(PrivacyViolation::new(PrivacyViolationKind::SizeLimit));
    }
    let combined = format!("{}\n{}\n{}", payload.summary, payload.title, payload.body);
    let lower = combined.to_ascii_lowercase();
    if context.denied_values.iter().any(|value| {
        combined.contains(value) || lower.contains(value.to_ascii_lowercase().as_str())
    }) {
        return Err(PrivacyViolation::new(PrivacyViolationKind::DynamicValue));
    }
    for (regex, kind) in [
        (authorization_re(), PrivacyViolationKind::Authorization),
        (url_credential_re(), PrivacyViolationKind::UrlCredential),
        (url_query_secret_re(), PrivacyViolationKind::UrlQuerySecret),
        (private_key_re(), PrivacyViolationKind::PrivateKey),
        (secret_re(), PrivacyViolationKind::Secret),
        (email_re(), PrivacyViolationKind::Email),
        (windows_path_re(), PrivacyViolationKind::WindowsPath),
        (unix_path_re(), PrivacyViolationKind::UnixPath),
        (log_excerpt_re(), PrivacyViolationKind::LogExcerpt),
        (code_excerpt_re(), PrivacyViolationKind::CodeExcerpt),
    ] {
        if regex.is_match(&combined) {
            return Err(PrivacyViolation::new(kind));
        }
    }
    Ok(())
}

fn add_path(denied: &mut BTreeSet<String>, path: &Path) {
    add_denied_value(denied, path.to_string_lossy().into_owned());
}

fn add_denied_value(denied: &mut BTreeSet<String>, value: String) {
    let value = value.trim();
    if value.chars().count() >= 4 && !value.contains("[redacted-") && value != "***redacted***" {
        denied.insert(value.to_string());
    }
}

fn source_repository_slug(repo_root: &Path) -> Option<String> {
    let mut command = gwt_core::process::hidden_command("git");
    let output = command
        .arg("-C")
        .arg(repo_root)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let remote = String::from_utf8(output.stdout).ok()?;
    let normalized = gwt_core::repo_hash::normalize_origin_url(remote.trim());
    normalized
        .strip_prefix("github.com/")
        .map(str::to_string)
        .filter(|slug| slug.contains('/'))
}

fn is_secret_environment_key(key: &str) -> bool {
    let normalized = key.to_ascii_uppercase();
    gwt_agent::is_secret_env_key(key)
        || normalized.contains("SECRET")
        || normalized.contains("TOKEN")
        || normalized.contains("PASSWORD")
        || normalized.contains("PASSWD")
        || normalized.contains("AUTH")
        || normalized.ends_with("_KEY")
        || normalized.contains("API_KEY")
}

fn add_secret_environment_values<I>(denied: &mut BTreeSet<String>, values: I)
where
    I: IntoIterator<Item = (String, String)>,
{
    for (key, value) in values {
        if is_secret_environment_key(&key) {
            add_denied_value(denied, value);
        }
    }
}

fn truncate_chars(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let mut output = input
        .chars()
        .take(max.saturating_sub(3))
        .collect::<String>();
    output.push_str("...");
    output
}

fn machine_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$").expect("machine token regex")
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn fingerprint_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^<!-- gwt:improvement-fingerprint:v1 (v2:[0-9a-f]{64}) -->$")
            .expect("fingerprint marker regex")
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn occurrence_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^occ:v1:[0-9a-f]{64}$").expect("occurrence key regex"))
}

fn authorization_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)authorization\s*:").expect("authorization regex"))
}

fn url_credential_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)https?://[^\s/:@]+:[^\s/@]+@").expect("URL credential regex")
    })
}

fn url_query_secret_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)[?&][^\s=&]*(?:token|secret|password|passwd|key|auth)[^\s=&]*=")
            .expect("URL query secret regex")
    })
}

fn private_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----").expect("private key regex")
    })
}

fn secret_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\b(?:gh[pousr]|github_pat)_[A-Za-z0-9_]{16,}\b|\b(?:api[_-]?key|token|secret|password|passwd|auth)\s*[=:]\s*[^\s]+)",
        )
        .expect("secret regex")
    })
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
            .expect("email regex")
    })
}

fn windows_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:\b[A-Z]:\\|\\\\[A-Za-z0-9._-]+\\)[^\r\n]*").expect("Windows path regex")
    })
}

fn unix_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?:^|[\s(\"'])/(?:[^/\s]+/)+[^/\s]+"#).expect("Unix path regex")
    })
}

fn log_excerpt_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:^|\s)(?:TRACE|DEBUG|INFO|WARN|ERROR)(?:\s|:)|stack backtrace:|thread '[^']+' panicked")
            .expect("log excerpt regex")
    })
}

fn code_excerpt_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"```").expect("code excerpt regex"))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use serde_json::json;

    use super::*;
    use crate::cli::improvement::ImprovementCandidate;

    fn candidate(typed: bool) -> ImprovementCandidate {
        serde_json::from_value(json!({
            "schema_version": 3,
            "id": "impr-public-template",
            "created_at": "2026-07-14T00:00:00Z",
            "updated_at": "2026-07-14T00:00:00Z",
            "source": "agent-failure",
            "target_artifact": "coordination",
            "classification": "gwt-caused",
            "confidence": "high",
            "state": "owner-resolving",
            "dedupe_key": "private-repo:customer-8675309",
            "occurrences": 2,
            "fingerprint": "v2:4bea839977a5aeedbf562acaeeb547012b0447f3335279830405fafb37726532",
            "eligibility": "deterministic",
            "typed_evidence": typed.then(|| json!({
                "subsystem": "coordination",
                "contract_id": "coordination.board-status",
                "contract_schema_revision": 1,
                "failure_code": "STATUS_NOT_POSTED",
                "target_artifact": "coordination",
                "expected_outcome": "BOARD_STATUS_POSTED",
                "observed_outcome": "BOARD_STATUS_MISSING"
            })),
            "distinct_occurrences": [],
            "sanitized_summary": "Customer customer-8675309 failed at /Users/alice/private-repo",
            "sanitized_details": "Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz",
            "evidence_digest": "alice@example.com",
            "local_evidence": [{
                "kind": "transcript",
                "path": "C:\\Users\\alice\\private-repo\\trace.log"
            }],
            "linked_issue": null,
            "dismissed_reason": null
        }))
        .expect("candidate")
    }

    fn payload_with_body(body: &str) -> PublicIssuePayload {
        PublicIssuePayload {
            summary: "Safe contract failure".to_string(),
            title: "fix(gwt): Safe contract failure".to_string(),
            body: body.to_string(),
        }
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn repository_context_collects_slug_roots_worktrees_and_secret_key_shapes() {
        let root = tempfile::tempdir().expect("root");
        let repository = root.path().join("private-repo");
        let worktree = root.path().join("private-repo-worktree");
        std::fs::create_dir_all(&repository).expect("repository");
        run_git(&repository, &["init"]);
        run_git(
            &repository,
            &["config", "user.email", "test@example.invalid"],
        );
        run_git(&repository, &["config", "user.name", "Test"]);
        run_git(
            &repository,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/acme/private-repo.git",
            ],
        );
        std::fs::write(repository.join("README.md"), "fixture\n").expect("README");
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-m", "test: fixture"]);
        run_git(
            &repository,
            &[
                "worktree",
                "add",
                "--detach",
                worktree.to_str().expect("worktree path"),
                "HEAD",
            ],
        );

        let context = PublicMutationContext::for_repo(&worktree);
        assert!(context
            .denied_values
            .contains(&"acme/private-repo".to_string()));
        assert!(context
            .denied_values
            .iter()
            .any(|value| value.contains("private-repo-worktree")));
        assert!(context
            .denied_values
            .iter()
            .any(|value| value.contains("private-repo")));
        for key in [
            "GITHUB_TOKEN",
            "service_password",
            "CUSTOM_AUTH_VALUE",
            "AWS_SECRET_ACCESS_KEY",
        ] {
            assert!(is_secret_environment_key(key), "{key}");
        }
        assert!(!is_secret_environment_key("PATH"));
        assert!(!is_secret_environment_key("GWT_PROJECT_ROOT"));
        let mut configured = BTreeSet::new();
        add_secret_environment_values(
            &mut configured,
            [
                (
                    "CUSTOM_PASSWORD".to_string(),
                    "configured-value".to_string(),
                ),
                ("PATH".to_string(), "/public/bin".to_string()),
            ],
        );
        assert!(configured.contains("configured-value"));
        assert!(!configured.contains("/public/bin"));
    }

    #[test]
    fn privacy_validator_rejects_dynamic_deny_values_without_echoing_them() {
        for denied in [
            "acme/private-repo",
            "/Users/alice/private-repo",
            "/Users/alice/private-repo-worktree",
            "/Users/alice",
            "alice-machine-user",
            "customer-8675309",
            "configured-secret-value",
        ] {
            let context = PublicMutationContext::from_denied_values_for_test([denied]);
            let error = validate_public_payload(
                &payload_with_body(&format!("Public body accidentally contains {denied}")),
                &context,
            )
            .expect_err("dynamic taint must fail closed");
            assert_eq!(error.kind(), PrivacyViolationKind::DynamicValue);
            assert!(!error.to_string().contains(denied));
        }
    }

    #[test]
    fn privacy_validator_rejects_structural_taint_and_size_matrix() {
        let context = PublicMutationContext::from_denied_values_for_test([] as [&str; 0]);
        let cases = [
            ("read /var/tmp/private.log", PrivacyViolationKind::UnixPath),
            (
                "read C:\\Users\\alice\\private.log",
                PrivacyViolationKind::WindowsPath,
            ),
            (
                "token ghp_abcdefghijklmnopqrstuvwxyz",
                PrivacyViolationKind::Secret,
            ),
            (
                "Authorization: Bearer opaque-value",
                PrivacyViolationKind::Authorization,
            ),
            (
                "https://alice:password@example.com/path",
                PrivacyViolationKind::UrlCredential,
            ),
            (
                "https://example.com/path?access_token=opaque-value",
                PrivacyViolationKind::UrlQuerySecret,
            ),
            (
                "-----BEGIN OPENSSH PRIVATE KEY-----",
                PrivacyViolationKind::PrivateKey,
            ),
            ("contact alice@example.com", PrivacyViolationKind::Email),
            (
                "2026-07-14T00:00:00Z ERROR request failed\nstack backtrace:",
                PrivacyViolationKind::LogExcerpt,
            ),
            (
                "```rust\nfn private() {}\n```",
                PrivacyViolationKind::CodeExcerpt,
            ),
        ];
        for (body, expected) in cases {
            let error = validate_public_payload(&payload_with_body(body), &context)
                .expect_err("structural taint must fail closed");
            assert_eq!(error.kind(), expected, "body: {body}");
        }

        let oversized = "x".repeat(MAX_PUBLIC_BODY_BYTES + 1);
        let error = validate_public_payload(&payload_with_body(&oversized), &context)
            .expect_err("oversized payload must fail closed");
        assert_eq!(error.kind(), PrivacyViolationKind::SizeLimit);
    }

    #[test]
    fn typed_and_legacy_templates_never_render_free_form_candidate_fields() {
        let context = PublicMutationContext::from_denied_values_for_test([
            "customer-8675309",
            "/Users/alice/private-repo",
            "ghp_abcdefghijklmnopqrstuvwxyz",
            "alice@example.com",
        ]);
        let typed = render_public_issue_payload(&candidate(true), &context)
            .expect("typed contract template");
        assert_eq!(typed.summary, "coordination: STATUS_NOT_POSTED");
        assert!(typed.body.contains("coordination.board-status"));
        assert!(typed.body.contains("BOARD_STATUS_POSTED"));
        assert!(typed.body.contains("BOARD_STATUS_MISSING"));
        assert!(!typed.body.contains("customer-8675309"));
        assert!(!typed.body.contains("private-repo"));
        assert!(!typed.body.contains("ghp_"));
        assert!(!typed.body.contains("alice@example.com"));

        let legacy = render_public_issue_payload(&candidate(false), &context)
            .expect("fixed legacy template");
        assert_eq!(
            legacy.summary,
            "Self-improvement contract evidence required"
        );
        assert!(!legacy.body.contains("customer-8675309"));
        assert!(!legacy.body.contains("private-repo"));
    }

    #[test]
    fn typed_issue_template_contains_computed_digest_and_exact_fingerprint_marker() {
        let context = PublicMutationContext::default();
        let payload = render_public_issue_payload(&candidate(true), &context)
            .expect("typed contract template");
        assert!(payload.body.contains(
            "Public evidence digest: sha256:3f649bd386b953b42442e8cefcbd1449d657f49a972f11d72f810bcda167756a"
        ));
        let marker = "<!-- gwt:improvement-fingerprint:v1 v2:4bea839977a5aeedbf562acaeeb547012b0447f3335279830405fafb37726532 -->";
        assert!(payload.body.lines().any(|line| line == marker));

        let lookalikes = format!(
            "{marker}\n{marker} suffix\nprefix {marker}\n```\n{marker}\n```\n<!-- gwt:improvement-fingerprint:v1 v2:{} -->",
            "0".repeat(64)
        );
        assert_eq!(
            exact_fingerprint_markers(&lookalikes),
            vec![
                "v2:4bea839977a5aeedbf562acaeeb547012b0447f3335279830405fafb37726532".to_string(),
                format!("v2:{}", "0".repeat(64)),
            ]
        );
    }

    #[test]
    fn occurrence_and_reconciliation_templates_are_opaque_and_taint_free() {
        let candidate = candidate(true);
        let context = PublicMutationContext::from_denied_values_for_test([
            "customer-8675309",
            "/Users/alice/private-repo",
            "ghp_abcdefghijklmnopqrstuvwxyz",
            "alice@example.com",
        ]);
        let occurrence_key =
            "occ:v1:760fc151831a9d5bf11893e402fdf5d63727e188dbc17015c67b2054f4a97148";
        let occurrence = render_occurrence_comment_payload(&candidate, occurrence_key, &context)
            .expect("occurrence comment");
        assert!(occurrence.body.lines().any(|line| {
            line == "<!-- gwt:improvement-occurrence:v1 occ:v1:760fc151831a9d5bf11893e402fdf5d63727e188dbc17015c67b2054f4a97148 -->"
        }));
        assert!(occurrence.body.contains("Public evidence digest: sha256:"));

        let reconciliation = render_reconciliation_comment_payload(&candidate, 42, 84, &context)
            .expect("reconciliation comment");
        assert!(reconciliation.body.contains("Canonical owner: #42"));
        assert!(reconciliation.body.contains("Duplicate owner: #84"));
        assert!(reconciliation.body.lines().any(|line| {
            line == "<!-- gwt:improvement-reconciliation:v1 canonical:42 duplicate:84 -->"
        }));

        for body in [&occurrence.body, &reconciliation.body] {
            assert!(!body.contains("customer-8675309"));
            assert!(!body.contains("private-repo"));
            assert!(!body.contains("ghp_"));
            assert!(!body.contains("alice@example.com"));
        }
    }
}
