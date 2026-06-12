//! SPEC-2359 Phase W-16 (FR-402): branch-keyed registry of machine-local
//! agent sessions.
//!
//! The Workspace surface shows the union of worktrees and Work records, but
//! the bulk of a project's history lives in the machine-local session ledger
//! (`~/.gwt/sessions/*.toml`), which records `branch`, `repo_hash`, and the
//! conversation `session_history` for every launch. This module groups that
//! ledger by canonical branch identity so each Workspace (branch) row can
//! surface its sessions even when `works.json` never recorded an agent for
//! the branch.
//!
//! Invariants (FR-402):
//! - Only sessions whose `repo_hash` matches the active project participate;
//!   TOMLs without a `repo_hash` are excluded to avoid cross-project
//!   mis-attachment.
//! - Sessions already present on a row (same gwt session id) are never
//!   duplicated.
//! - At most [`REGISTRY_SESSION_CAP`] registry sessions ride the wire per
//!   Workspace (the workspace payload feeds every connected client; unbounded
//!   session fan-out amplifies the WebSocket eviction storm). The uncapped
//!   count is carried separately via `session_agent_total`.

use std::collections::HashMap;

use crate::runtime_support::normalize_branch_name;

/// Maximum registry-derived agents attached to one Workspace row (FR-402).
pub const REGISTRY_SESSION_CAP: usize = 8;

/// Group this project's sessions by canonical branch identity, newest first
/// (`last_activity_at` descending, id ascending as a deterministic tiebreak).
pub fn branch_session_registry<'a>(
    sessions: &'a [gwt_agent::Session],
    project_repo_hash: Option<&str>,
) -> HashMap<String, Vec<&'a gwt_agent::Session>> {
    let Some(project_repo_hash) = project_repo_hash else {
        return HashMap::new();
    };
    let mut registry: HashMap<String, Vec<&gwt_agent::Session>> = HashMap::new();
    for session in sessions {
        if session.repo_hash.as_deref() != Some(project_repo_hash) {
            continue;
        }
        let branch = session.branch.trim();
        if branch.is_empty() {
            continue;
        }
        registry
            .entry(normalize_branch_name(branch))
            .or_default()
            .push(session);
    }
    for group in registry.values_mut() {
        group.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| left.id.cmp(&right.id))
        });
    }
    registry
}

/// Select up to `cap` registry sessions for `branch` that are not already
/// represented by `existing_session_ids`. Returns the capped selection plus
/// the uncapped number of additional sessions (for `session_agent_total`).
pub fn registry_sessions_for_branch<'a>(
    registry: &HashMap<String, Vec<&'a gwt_agent::Session>>,
    branch: Option<&str>,
    existing_session_ids: &[&str],
    cap: usize,
) -> (Vec<&'a gwt_agent::Session>, usize) {
    let Some(branch) = branch.map(str::trim).filter(|value| !value.is_empty()) else {
        return (Vec::new(), 0);
    };
    let Some(group) = registry.get(&normalize_branch_name(branch)) else {
        return (Vec::new(), 0);
    };
    let additions: Vec<&gwt_agent::Session> = group
        .iter()
        .filter(|session| !existing_session_ids.contains(&session.id.as_str()))
        .copied()
        .collect();
    let total = additions.len();
    (additions.into_iter().take(cap).collect(), total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn session(
        id: &str,
        branch: &str,
        repo_hash: Option<&str>,
        last_activity_minute: u32,
    ) -> gwt_agent::Session {
        let mut session = gwt_agent::Session::new(
            std::path::PathBuf::from("/tmp/none"),
            branch,
            gwt_agent::AgentId::ClaudeCode,
        );
        session.id = id.to_string();
        session.repo_hash = repo_hash.map(str::to_string);
        session.last_activity_at = Utc
            .with_ymd_and_hms(2026, 6, 10, 12, last_activity_minute, 0)
            .unwrap();
        session
    }

    /// FR-402: only sessions whose repo_hash matches the project participate;
    /// TOMLs without repo_hash are excluded; `origin/X` and `X` group together;
    /// groups are newest-first.
    #[test]
    fn registry_filters_by_repo_hash_and_groups_by_canonical_branch() {
        let sessions = vec![
            session("s-dev-old", "develop", Some("hash-a"), 1),
            session("s-dev-new", "origin/develop", Some("hash-a"), 30),
            session("s-other-project", "develop", Some("hash-b"), 40),
            session("s-no-hash", "develop", None, 50),
        ];

        let registry = branch_session_registry(&sessions, Some("hash-a"));

        let develop = registry.get("develop").expect("develop group");
        let ids: Vec<&str> = develop.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["s-dev-new", "s-dev-old"],
            "same project only, origin/ normalized, newest first"
        );
        assert!(
            branch_session_registry(&sessions, None).is_empty(),
            "unknown project hash attaches nothing"
        );
    }

    /// FR-402: dedup against record agents, cap the wire payload, and report
    /// the uncapped addition count for `session_agent_total`.
    #[test]
    fn selection_dedupes_caps_and_reports_total() {
        let sessions: Vec<gwt_agent::Session> = (0..12)
            .map(|index| {
                session(
                    &format!("s-{index:02}"),
                    "work/foo",
                    Some("hash-a"),
                    index as u32,
                )
            })
            .collect();
        let registry = branch_session_registry(&sessions, Some("hash-a"));

        let (additions, total) = registry_sessions_for_branch(
            &registry,
            Some("work/foo"),
            &["s-11"],
            REGISTRY_SESSION_CAP,
        );

        assert_eq!(
            total, 11,
            "12 sessions minus 1 already-present record agent"
        );
        assert_eq!(additions.len(), REGISTRY_SESSION_CAP);
        assert_eq!(
            additions[0].id, "s-10",
            "newest non-duplicated session first (s-11 deduped)"
        );
        assert!(additions.iter().all(|s| s.id != "s-11"));

        let (none, zero) = registry_sessions_for_branch(&registry, Some("work/unknown"), &[], 8);
        assert!(none.is_empty());
        assert_eq!(zero, 0);
    }
}
