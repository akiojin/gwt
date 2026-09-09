//! Observe-only semantic effect classification for hook events.
//!
//! The classifier is deliberately a pre-filter and diagnostic surface. It
//! performs no authorization and never changes a [`super::HookOutput`]; exact
//! operation and transport sinks remain responsible for enforcement.

use std::{fmt, path::Path};

use crate::cli::governance::GovernanceEffect;
use serde::{Deserialize, Serialize};

use super::{block_bash_policy, block_git_branch_ops, workflow_policy, HookEvent};

pub const EFFECT_OBSERVATION_REVISION: u32 = 1;

const REASON_KNOWN_READ_ONLY: &str = "known_read_only_operation";
const REASON_KNOWN_REVERSIBLE: &str = "known_reversible_operation";
const REASON_KNOWN_PROTECTED: &str = "known_protected_operation";
const REASON_NOT_EXPLICITLY_REVERSIBLE: &str = "operation_not_explicitly_reversible";
const REASON_SHELL_READ_ONLY: &str = "shell_read_only_heuristic";
const REASON_SHELL_REVERSIBLE: &str = "shell_reversible_heuristic";
const REASON_SHELL_PROTECTED: &str = "shell_protected_heuristic";
const REASON_LEXICAL_MANAGED_PATH: &str = "lexical_managed_path_unverified";
const REASON_LEXICAL_EXTERNAL: &str = "lexical_external_repository_unverified";
const REASON_EXPLICIT_REMOTE: &str = "explicit_remote_repository_heuristic";
const REASON_UNKNOWN_REMOTE: &str = "unknown_remote_repository";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryTarget {
    ManagedCurrent,
    ExternalPath,
    ExplicitRemote(String),
    UnknownRemote,
}

impl fmt::Display for RepositoryTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManagedCurrent => formatter.write_str("managed_current"),
            Self::ExternalPath => formatter.write_str("external_path"),
            Self::ExplicitRemote(repository) => write!(formatter, "remote:{repository}"),
            Self::UnknownRemote => formatter.write_str("unknown_remote"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationConfidence {
    Exact,
    Heuristic,
}

impl fmt::Display for ObservationConfidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact => formatter.write_str("exact"),
            Self::Heuristic => formatter.write_str("heuristic"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectObservation {
    pub observation_revision: u32,
    pub target: RepositoryTarget,
    pub operation: String,
    pub effect: GovernanceEffect,
    pub confidence: ObservationConfidence,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetClassification {
    target: RepositoryTarget,
    confidence: ObservationConfidence,
    reason: Option<&'static str>,
}

/// Classify a hook event without performing I/O or making a policy decision.
pub fn classify_event(event: &HookEvent, managed_root: &Path) -> Option<EffectObservation> {
    let cwd = event.cwd.as_deref().map(Path::new).unwrap_or(managed_root);

    match event.tool_name.as_deref() {
        Some("Bash") => classify_bash_command(event.command()?, cwd, managed_root),
        Some("Read" | "Glob" | "Grep") => Some(observation(
            event_path_target(event, cwd, managed_root),
            format!("tool.{}", event.tool_name.as_deref()?.to_ascii_lowercase()),
            GovernanceEffect::Observe,
            ObservationConfidence::Exact,
            REASON_KNOWN_READ_ONLY,
        )),
        Some("Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "apply_patch") => {
            let protected = event_targets_authority_surface(event, cwd);
            Some(observation(
                event_path_target(event, cwd, managed_root),
                format!(
                    "tool.{}",
                    event
                        .tool_name
                        .as_deref()?
                        .to_ascii_lowercase()
                        .replace('_', "-")
                ),
                if protected {
                    GovernanceEffect::Protected
                } else {
                    GovernanceEffect::Reversible
                },
                ObservationConfidence::Exact,
                if protected {
                    REASON_KNOWN_PROTECTED
                } else {
                    REASON_KNOWN_REVERSIBLE
                },
            ))
        }
        _ => None,
    }
}

/// Classify a Bash command using the hook's canonical managed repository
/// context. Compound commands report their strongest observed effect.
pub fn classify_bash_command(
    command: &str,
    cwd: &Path,
    managed_root: &Path,
) -> Option<EffectObservation> {
    if let Some(envelope) = workflow_policy::json_envelope(command) {
        let operation = envelope.get("operation")?.as_str()?.to_string();
        return Some(classify_json_operation(
            operation,
            envelope.get("params"),
            target_for_path(cwd, cwd, managed_root),
        ));
    }

    let segments = super::segments::split_command_segments(command);
    let mut observations = segments
        .iter()
        .filter_map(|segment| classify_segment(segment, cwd, managed_root));
    let first = observations.next()?;
    let mut strongest = first;
    let mut compound = false;
    for observation in observations {
        compound = true;
        if effect_rank(observation.effect) > effect_rank(strongest.effect) {
            strongest = observation;
        }
    }
    if compound {
        strongest.operation = "shell.compound".to_string();
    }
    Some(strongest)
}

pub(crate) fn observe_event(event: &HookEvent, managed_root: &Path) {
    if let Some(observation) = classify_event(event, managed_root) {
        trace_observation(&observation);
    }
}

pub(crate) fn observe_bash_command(command: &str, cwd: &Path, managed_root: &Path) {
    if let Some(observation) = classify_bash_command(command, cwd, managed_root) {
        trace_observation(&observation);
    }
}

fn trace_observation(observation: &EffectObservation) {
    tracing::debug!(
        target: "gwt::hook::effect_classifier",
        observation_revision = observation.observation_revision,
        repository_target = %observation.target,
        operation = %observation.operation,
        effect = effect_name(observation.effect),
        confidence = %observation.confidence,
        reason = %observation.reason,
        "observed hook operation effect"
    );
}

fn observation(
    target: TargetClassification,
    operation: String,
    effect: GovernanceEffect,
    operation_confidence: ObservationConfidence,
    operation_reason: &'static str,
) -> EffectObservation {
    let confidence = if target.confidence == ObservationConfidence::Heuristic
        || operation_confidence == ObservationConfidence::Heuristic
    {
        ObservationConfidence::Heuristic
    } else {
        ObservationConfidence::Exact
    };
    EffectObservation {
        observation_revision: EFFECT_OBSERVATION_REVISION,
        target: target.target,
        operation,
        effect,
        confidence,
        reason: target.reason.unwrap_or(operation_reason).to_string(),
    }
}

fn classify_json_operation(
    operation: String,
    params: Option<&serde_json::Value>,
    target: TargetClassification,
) -> EffectObservation {
    let (effect, reason) = if operation == "pr.create" {
        if params
            .and_then(|value| value.get("draft"))
            .and_then(serde_json::Value::as_bool)
            .is_some_and(|draft| draft)
        {
            (GovernanceEffect::Reversible, REASON_KNOWN_REVERSIBLE)
        } else {
            (GovernanceEffect::Protected, REASON_KNOWN_PROTECTED)
        }
    } else if operation == "execution.continue" {
        (GovernanceEffect::Protected, REASON_KNOWN_PROTECTED)
    } else if workflow_policy::is_read_only_json_envelope_operation(&operation) {
        (GovernanceEffect::Observe, REASON_KNOWN_READ_ONLY)
    } else if is_explicitly_reversible_json_operation(&operation) {
        (GovernanceEffect::Reversible, REASON_KNOWN_REVERSIBLE)
    } else {
        (
            GovernanceEffect::Protected,
            REASON_NOT_EXPLICITLY_REVERSIBLE,
        )
    };
    observation(
        target,
        operation,
        effect,
        ObservationConfidence::Exact,
        reason,
    )
}

fn is_explicitly_reversible_json_operation(operation: &str) -> bool {
    matches!(operation, "pr.edit" | "pr.draft" | "workspace.update")
}

fn classify_segment(segment: &str, cwd: &Path, managed_root: &Path) -> Option<EffectObservation> {
    let tokens = workflow_policy::segment_tokens(segment);
    let command_name = workflow_policy::normalize_command_name(tokens.first().copied()?);
    let mut target = target_for_path(cwd, cwd, managed_root);

    if command_name == "git" {
        let parsed = parse_git_command(&tokens[1..]);
        if let Some(directory) = parsed.directory {
            target = target_for_path(Path::new(directory), cwd, managed_root);
        }
        let operation = parsed
            .subcommand
            .map_or_else(|| "git".to_string(), |name| format!("git.{name}"));
        let targets_external = target.target == RepositoryTarget::ExternalPath;
        let (effect, reason) = match (parsed.subcommand, parsed.args.first().copied()) {
            (Some("worktree"), Some("list")) => (GovernanceEffect::Observe, REASON_SHELL_READ_ONLY),
            _ if is_destructive_git_command(parsed.subcommand, parsed.args) => {
                (GovernanceEffect::Protected, REASON_SHELL_PROTECTED)
            }
            (Some("checkout" | "switch"), _) if targets_external => {
                (GovernanceEffect::Reversible, REASON_SHELL_REVERSIBLE)
            }
            (Some("worktree"), _) => (GovernanceEffect::Protected, REASON_SHELL_PROTECTED),
            _ if block_git_branch_ops::evaluate_bash_command(segment).is_some() => {
                (GovernanceEffect::Protected, REASON_SHELL_PROTECTED)
            }
            _ if workflow_policy::is_read_only_segment(segment) => {
                (GovernanceEffect::Observe, REASON_SHELL_READ_ONLY)
            }
            _ => (GovernanceEffect::Reversible, REASON_SHELL_REVERSIBLE),
        };
        return Some(observation(
            target,
            operation,
            effect,
            ObservationConfidence::Heuristic,
            reason,
        ));
    }

    if command_name == "gh" {
        return Some(classify_gh_segment(&tokens, target));
    }

    if let Some(mutation) = block_bash_policy::curl_remote_mutation(segment) {
        let target = if block_bash_policy::targets_github_api(segment) {
            explicit_github_repository(segment)
                .map(explicit_remote_target)
                .unwrap_or_else(unknown_remote_target)
        } else {
            unknown_remote_target()
        };
        return Some(observation(
            target,
            if mutation {
                "remote.mutation"
            } else {
                "remote.query"
            }
            .to_string(),
            if mutation {
                GovernanceEffect::Protected
            } else {
                GovernanceEffect::Observe
            },
            ObservationConfidence::Heuristic,
            if mutation {
                REASON_SHELL_PROTECTED
            } else {
                REASON_SHELL_READ_ONLY
            },
        ));
    }

    let (effect, reason) = if is_destructive_local_command(&command_name, &tokens[1..]) {
        (GovernanceEffect::Protected, REASON_SHELL_PROTECTED)
    } else if workflow_policy::is_read_only_segment(segment) {
        (GovernanceEffect::Observe, REASON_SHELL_READ_ONLY)
    } else {
        (GovernanceEffect::Reversible, REASON_SHELL_REVERSIBLE)
    };
    Some(observation(
        target,
        format!("local.{command_name}"),
        effect,
        ObservationConfidence::Heuristic,
        reason,
    ))
}

fn classify_gh_segment(tokens: &[&str], default_target: TargetClassification) -> EffectObservation {
    let category = tokens.get(1).copied().unwrap_or("unknown");
    let action = tokens.get(2).copied().unwrap_or("unknown");
    let operation = if category == "api" {
        "gh.api".to_string()
    } else {
        format!("gh.{category}.{action}")
    };
    let explicit_repository = explicit_gh_repository(tokens).or_else(|| {
        (category == "api")
            .then(|| explicit_github_repository(&tokens.join(" ")))
            .flatten()
    });
    let target = explicit_repository
        .map(explicit_remote_target)
        .unwrap_or_else(|| {
            if category == "api" {
                unknown_remote_target()
            } else {
                default_target
            }
        });

    let read_only = matches!(
        (category, action),
        ("auth", "status")
            | ("repo", "view" | "list")
            | ("issue", "view" | "list" | "status")
            | ("pr", "view" | "list" | "status" | "checks" | "reviews")
            | ("run", "view" | "list" | "watch")
            | ("release", "view" | "list" | "download")
    ) || (category == "api"
        && !block_bash_policy::is_github_remote_mutation(&tokens.join(" ")));
    let draft_reversible = category == "pr"
        && (action == "edit"
            || (action == "create"
                && tokens
                    .iter()
                    .any(|token| matches!(*token, "--draft" | "-d")))
            || (action == "ready" && tokens.contains(&"--undo")));
    let effect = if read_only {
        GovernanceEffect::Observe
    } else if draft_reversible {
        GovernanceEffect::Reversible
    } else {
        GovernanceEffect::Protected
    };

    let reason = match effect {
        GovernanceEffect::Observe => REASON_SHELL_READ_ONLY,
        GovernanceEffect::Reversible => REASON_SHELL_REVERSIBLE,
        GovernanceEffect::Protected => REASON_SHELL_PROTECTED,
    };
    observation(
        target,
        operation,
        effect,
        ObservationConfidence::Heuristic,
        reason,
    )
}

#[derive(Debug, Clone, Copy)]
struct ParsedGitCommand<'a> {
    directory: Option<&'a str>,
    subcommand: Option<&'a str>,
    args: &'a [&'a str],
}

fn parse_git_command<'a>(tokens: &'a [&'a str]) -> ParsedGitCommand<'a> {
    let mut directory = None;
    let mut index = 0;
    loop {
        match tokens.get(index).copied() {
            Some("-C") if tokens.get(index + 1).is_some() => {
                directory = tokens.get(index + 1).copied();
                index += 2;
            }
            Some("-c") if tokens.get(index + 1).is_some() => index += 2,
            Some("--no-pager" | "-P") => index += 1,
            _ => break,
        }
    }
    let subcommand = tokens.get(index).copied();
    let args = tokens.get(index + 1..).unwrap_or(&[]);
    ParsedGitCommand {
        directory,
        subcommand,
        args,
    }
}

fn is_destructive_git_command(subcommand: Option<&str>, args: &[&str]) -> bool {
    match subcommand {
        Some("worktree") => true,
        Some("branch") => {
            !workflow_policy::is_read_only_segment(&format!("git branch {}", args.join(" ")))
        }
        Some("reset" | "rebase" | "update-ref" | "restore") => true,
        Some("checkout" | "switch") => checkout_or_switch_has_protected_effect(subcommand, args),
        Some("commit") => args
            .iter()
            .any(|arg| *arg == "--amend" || arg.starts_with("--amend=")),
        Some("tag") => tag_has_destructive_ref_flag(args),
        Some("clean") => args.iter().any(|arg| {
            *arg == "--force"
                || arg
                    .strip_prefix('-')
                    .is_some_and(|flags| flags.contains('f'))
        }),
        Some("push") => true,
        _ => false,
    }
}

fn checkout_or_switch_has_protected_effect(subcommand: Option<&str>, args: &[&str]) -> bool {
    let checkout = subcommand == Some("checkout");
    if checkout && args.contains(&"--") {
        return true;
    }

    args.iter().any(|arg| {
        if matches!(
            *arg,
            "--force"
                | "--orphan"
                | "--discard-changes"
                | "--create"
                | "--force-create"
                | "--ours"
                | "--theirs"
                | "--patch"
                | "--pathspec-from-file"
                | "--pathspec-file-nul"
        ) {
            return true;
        }
        let Some(flags) = arg
            .strip_prefix('-')
            .filter(|flags| !flags.starts_with('-'))
        else {
            return false;
        };
        flags.chars().any(|flag| {
            if checkout {
                matches!(flag, 'b' | 'B' | 'f' | 'p')
            } else {
                matches!(flag, 'c' | 'C' | 'f')
            }
        })
    })
}

fn tag_has_destructive_ref_flag(args: &[&str]) -> bool {
    args.iter().any(|arg| {
        if matches!(*arg, "--delete" | "--force") {
            return true;
        }
        arg.strip_prefix('-')
            .filter(|flags| !flags.starts_with('-'))
            .is_some_and(|flags| flags.chars().any(|flag| matches!(flag, 'd' | 'f')))
    })
}

fn is_destructive_local_command(command_name: &str, args: &[&str]) -> bool {
    matches!(
        command_name,
        "rm" | "rmdir" | "unlink" | "truncate" | "shred" | "chmod" | "chown"
    ) || (command_name == "dd"
        && args
            .iter()
            .any(|arg| *arg == "of" || arg.starts_with("of=")))
}

fn explicit_gh_repository(tokens: &[&str]) -> Option<String> {
    for (index, token) in tokens.iter().enumerate() {
        if matches!(*token, "--repo" | "-R") {
            return tokens.get(index + 1).map(|value| clean_token(value));
        }
        if let Some(value) = token.strip_prefix("--repo=") {
            return Some(clean_token(value));
        }
    }
    None
}

fn explicit_github_repository(command: &str) -> Option<String> {
    let normalized = command.replace('\\', "/");
    let marker = normalized
        .find("/repos/")
        .map(|index| index + "/repos/".len())
        .or_else(|| {
            normalized
                .find("repos/")
                .map(|index| index + "repos/".len())
        })?;
    let mut parts = normalized[marker..].split(['/', '?', '#', '\'', '"', ' ']);
    let owner = parts.next().filter(|value| !value.is_empty())?;
    let repository = parts.next().filter(|value| !value.is_empty())?;
    Some(format!("{owner}/{repository}"))
}

fn explicit_remote_target(repository: String) -> TargetClassification {
    TargetClassification {
        target: RepositoryTarget::ExplicitRemote(repository),
        confidence: ObservationConfidence::Heuristic,
        reason: Some(REASON_EXPLICIT_REMOTE),
    }
}

fn unknown_remote_target() -> TargetClassification {
    TargetClassification {
        target: RepositoryTarget::UnknownRemote,
        confidence: ObservationConfidence::Heuristic,
        reason: Some(REASON_UNKNOWN_REMOTE),
    }
}

fn event_path_target(event: &HookEvent, cwd: &Path, managed_root: &Path) -> TargetClassification {
    let parsed_paths = classification_event_paths(event);
    parsed_paths
        .iter()
        .map(|path| target_for_path(Path::new(path), cwd, managed_root))
        .reduce(security_stronger_path_target)
        .unwrap_or_else(|| target_for_path(cwd, cwd, managed_root))
}

fn event_targets_authority_surface(event: &HookEvent, cwd: &Path) -> bool {
    classification_event_paths(event)
        .iter()
        .any(|path| path_targets_authority_surface(Path::new(path), cwd))
}

fn classification_event_paths(event: &HookEvent) -> Vec<String> {
    let mut paths = workflow_policy::event_target_paths(event);
    if let Some(input) = event.tool_input.as_ref() {
        for key in ["path", "notebook_path"] {
            if let Some(path) = input.get(key).and_then(serde_json::Value::as_str) {
                paths.push(path.to_string());
            }
        }
        if event.tool_name.as_deref() == Some("MultiEdit") {
            paths.extend(
                input
                    .get("edits")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|edit| edit.get("file_path"))
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
            );
        }
    }
    paths
}

fn security_stronger_path_target(
    current: TargetClassification,
    candidate: TargetClassification,
) -> TargetClassification {
    if path_target_security_rank(&candidate) > path_target_security_rank(&current) {
        candidate
    } else {
        current
    }
}

fn path_target_security_rank(target: &TargetClassification) -> u8 {
    match (&target.target, target.confidence) {
        (RepositoryTarget::ExternalPath, _) => 2,
        (RepositoryTarget::ManagedCurrent, ObservationConfidence::Heuristic) => 1,
        _ => 0,
    }
}

fn path_targets_authority_surface(path: &Path, cwd: &Path) -> bool {
    let portable = path.to_string_lossy().replace('\\', "/");
    let portable = Path::new(&portable);
    let absolute = if portable.is_absolute() {
        portable.to_path_buf()
    } else {
        cwd.join(portable)
    };
    lexical_normalize(&absolute).components().any(|component| {
        matches!(component, std::path::Component::Normal(part) if part.to_string_lossy().eq_ignore_ascii_case(".git") || part.to_string_lossy().eq_ignore_ascii_case(".gwt"))
    })
}

fn target_for_path(path: &Path, cwd: &Path, managed_root: &Path) -> TargetClassification {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let managed = lexical_normalize(managed_root);
    let target = lexical_normalize(&absolute);
    if target == managed {
        TargetClassification {
            target: RepositoryTarget::ManagedCurrent,
            confidence: ObservationConfidence::Exact,
            reason: None,
        }
    } else if target.starts_with(&managed) {
        TargetClassification {
            target: RepositoryTarget::ManagedCurrent,
            confidence: ObservationConfidence::Heuristic,
            reason: Some(REASON_LEXICAL_MANAGED_PATH),
        }
    } else {
        TargetClassification {
            target: RepositoryTarget::ExternalPath,
            confidence: ObservationConfidence::Heuristic,
            reason: Some(REASON_LEXICAL_EXTERNAL),
        }
    }
}

fn lexical_normalize(path: &Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn clean_token(token: &str) -> String {
    token
        .trim_matches(|character| matches!(character, '\'' | '"'))
        .to_string()
}

fn effect_rank(effect: GovernanceEffect) -> u8 {
    match effect {
        GovernanceEffect::Observe => 0,
        GovernanceEffect::Reversible => 1,
        GovernanceEffect::Protected => 2,
    }
}

fn effect_name(effect: GovernanceEffect) -> &'static str {
    match effect {
        GovernanceEffect::Observe => "observe",
        GovernanceEffect::Reversible => "reversible",
        GovernanceEffect::Protected => "protected",
    }
}
