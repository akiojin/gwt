//! gwt-repository-only self-improvement Stop hook.
//!
//! This hook is intentionally separate from the shared gwt Managed Hooks
//! dispatcher. It belongs to the `akiojin/gwt` repository's own development
//! loop, not to arbitrary projects managed by gwt.

use std::{path::Path, process::Command};

use super::{envelope::stop_hook_active_from, HookOutput};

pub fn handle_with_input(worktree_root: &Path, input: &str) -> HookOutput {
    // SPEC-3248 (hooks v2 P3): whether this Stop gate fires is a lane policy,
    // resolved from the worktree lane file (source of truth) via the shared
    // HookContext. A lane whose profile disables `self_improvement_stop`
    // (intake today) suppresses the gate. Replaces the SPEC-3247 ad-hoc
    // `SessionKind::from_env().is_intake()` branch.
    let suppress = !super::context::HookContext::for_worktree(worktree_root)
        .lane
        .policy_flags
        .self_improvement_stop;
    evaluate(worktree_root, stop_hook_active_from(input), suppress)
}

/// Decide the self-improvement Stop block.
///
/// SPEC-3247 FR-003 / AS-4 → SPEC-3248 (hooks v2): this is a producing-work Stop
/// gate. A lane whose profile disables `self_improvement_stop` (intake today)
/// must never be forced to handle improvement candidates before stopping, so
/// `suppressed_by_lane` short-circuits to [`HookOutput::Silent`] alongside the
/// existing `stop_hook_active` / non-gwt-repo guards.
pub fn evaluate(
    worktree_root: &Path,
    stop_hook_active: bool,
    suppressed_by_lane: bool,
) -> HookOutput {
    if stop_hook_active || suppressed_by_lane || !is_gwt_repository(worktree_root) {
        return HookOutput::Silent;
    }

    let candidates =
        crate::cli::improvement::pending_high_confidence_contract_violations(worktree_root);
    if candidates.is_empty() {
        return HookOutput::Silent;
    }

    let mut reason = String::from(
        "High-confidence gwt self-improvement candidate requires handling before stopping.\n\n",
    );
    reason.push_str("Unhandled candidates:\n");
    for candidate in &candidates {
        reason.push_str(&format!(
            "- {} [{}]: {}\n",
            candidate.id, candidate.target_artifact, candidate.summary
        ));
    }
    reason.push_str(
        "\nNext action: run `improvement.promote_issue` for actionable gwt-caused problems, \
run `improvement.dismiss` with a reason for false positives, or explicitly park the candidate \
before stopping.",
    );
    HookOutput::StopBlock { reason }
}

pub fn is_gwt_repository(worktree_root: &Path) -> bool {
    origin_remote_url(worktree_root)
        .and_then(|url| github_slug_from_remote_url(&url))
        .is_some_and(|slug| slug == "akiojin/gwt")
}

fn origin_remote_url(worktree_root: &Path) -> Option<String> {
    let root = gwt_core::paths::resolve_current_worktree_root(worktree_root);
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn github_slug_from_remote_url(url: &str) -> Option<String> {
    let value = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("http://github.com/"))
        .or_else(|| value.strip_prefix("ssh://git@github.com/"))
        .or_else(|| value.strip_prefix("git@github.com:"))?;
    let slug = rest
        .trim_matches('/')
        .trim_end_matches(".git")
        .to_ascii_lowercase();
    let mut parts = slug.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::github_slug_from_remote_url;

    #[test]
    fn parses_github_remote_urls() {
        for url in [
            "https://github.com/akiojin/gwt.git",
            "https://github.com/akiojin/gwt",
            "git@github.com:akiojin/gwt.git",
            "ssh://git@github.com/akiojin/gwt.git",
        ] {
            assert_eq!(
                github_slug_from_remote_url(url).as_deref(),
                Some("akiojin/gwt"),
                "{url}"
            );
        }
    }

    #[test]
    fn rejects_non_github_urls() {
        assert_eq!(github_slug_from_remote_url("file:///tmp/gwt.git"), None);
        assert_eq!(
            github_slug_from_remote_url("https://example.com/akiojin/gwt.git"),
            None
        );
    }
}
