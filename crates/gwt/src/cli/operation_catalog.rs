//! The JSON-envelope operation names, so a mistyped one can be answered with
//! the names it could have meant (Issue #4449 AC-4).
//!
//! An agent that ran the `workspace.prune` a refusal message recommended got
//! `unknown subcommand: workspace.prune` and nothing else. The operation it
//! wanted, `workspace.projection_prune`, is one segment away and is not
//! discoverable from anywhere the agent could reach; three of them lost over
//! an hour to it on 2026-09-16 before someone read the dispatch source.
//!
//! Suggesting requires *enumerating* the operations, which the dispatch
//! `match` cannot do at runtime — hence this list, and
//! `catalog_covers_every_dispatched_operation`, which fails the moment
//! `json_envelope.rs` gains or loses an arm without this list following.
//!
//! Whether guidance and refusal text name operations that exist is a separate
//! problem, owned by Issue #4396.

/// One dispatched operation plus the spellings that resolve to it.
///
/// `name` is the canonical spelling — the one guidance and refusals print.
/// `aliases` are the accepted variants (usually the `-` spelling of a `_`
/// segment), which resolve but are never recommended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
}

/// Every operation `crate::cli::json_envelope` dispatches, canonical name
/// first. Kept sorted so a diff against the dispatch source stays readable.
pub const OPERATIONS: &[Operation] = &[
    Operation {
        name: "actions.job_logs",
        aliases: &["actions.job-logs"],
    },
    Operation {
        name: "actions.logs",
        aliases: &[],
    },
    Operation {
        name: "actions.rerun",
        aliases: &[],
    },
    Operation {
        name: "board.config.show",
        aliases: &["board.config-show"],
    },
    Operation {
        name: "board.post",
        aliases: &[],
    },
    Operation {
        name: "board.show",
        aliases: &[],
    },
    Operation {
        name: "branch.prune_merged",
        aliases: &["branch.prune-merged"],
    },
    Operation {
        name: "build.abort",
        aliases: &[],
    },
    Operation {
        name: "build.complete",
        aliases: &[],
    },
    Operation {
        name: "build.phase",
        aliases: &[],
    },
    Operation {
        name: "build.start",
        aliases: &[],
    },
    Operation {
        name: "concern.create",
        aliases: &[],
    },
    Operation {
        name: "concern.list",
        aliases: &[],
    },
    Operation {
        name: "concern.measure",
        aliases: &[],
    },
    Operation {
        name: "concern.resolve",
        aliases: &[],
    },
    Operation {
        name: "concern.update",
        aliases: &[],
    },
    Operation {
        name: "daemon.start",
        aliases: &[],
    },
    Operation {
        name: "daemon.status",
        aliases: &[],
    },
    Operation {
        name: "daemon.subscribe",
        aliases: &[],
    },
    Operation {
        name: "diagnostics.cpu",
        aliases: &[],
    },
    Operation {
        name: "discuss.clear_next_question",
        aliases: &["discuss.clear-next-question"],
    },
    Operation {
        name: "discuss.goal_failed",
        aliases: &["discuss.goal-failed"],
    },
    Operation {
        name: "discuss.goal_pending",
        aliases: &["discuss.goal-pending"],
    },
    Operation {
        name: "discuss.goal_skipped",
        aliases: &["discuss.goal-skipped"],
    },
    Operation {
        name: "discuss.goal_started",
        aliases: &["discuss.goal-started"],
    },
    Operation {
        name: "discuss.park",
        aliases: &[],
    },
    Operation {
        name: "discuss.reject",
        aliases: &[],
    },
    Operation {
        name: "discuss.resolve",
        aliases: &[],
    },
    Operation {
        name: "discussion.update",
        aliases: &[],
    },
    Operation {
        name: "errors.list",
        aliases: &[],
    },
    Operation {
        name: "execution.adopt",
        aliases: &[],
    },
    Operation {
        name: "execution.blocked",
        aliases: &[],
    },
    Operation {
        name: "execution.complete",
        aliases: &[],
    },
    Operation {
        name: "execution.continue",
        aliases: &[],
    },
    Operation {
        name: "execution.release_prepared",
        aliases: &[],
    },
    Operation {
        name: "execution.reopen",
        aliases: &[],
    },
    Operation {
        name: "execution.repair",
        aliases: &[],
    },
    Operation {
        name: "execution.status",
        aliases: &[],
    },
    Operation {
        name: "github.budget",
        aliases: &[],
    },
    Operation {
        name: "hook.doctor",
        aliases: &[],
    },
    Operation {
        name: "hook.health",
        aliases: &[],
    },
    Operation {
        name: "hook.register_codex_managed_hook_trust",
        aliases: &["hook.register-codex-managed-hook-trust"],
    },
    Operation {
        name: "hook.register_codex_managed_project_trust",
        aliases: &["hook.register-codex-managed-project-trust"],
    },
    Operation {
        name: "index.cancel",
        aliases: &[],
    },
    Operation {
        name: "index.rebuild",
        aliases: &[],
    },
    Operation {
        name: "index.repair",
        aliases: &[],
    },
    Operation {
        name: "index.status",
        aliases: &[],
    },
    Operation {
        name: "intake.outcome.record",
        aliases: &["intake.outcome-record"],
    },
    Operation {
        name: "issue.close",
        aliases: &[],
    },
    Operation {
        name: "issue.comment",
        aliases: &[],
    },
    Operation {
        name: "issue.comments",
        aliases: &[],
    },
    Operation {
        name: "issue.create",
        aliases: &[],
    },
    Operation {
        name: "issue.edit",
        aliases: &[],
    },
    Operation {
        name: "issue.label",
        aliases: &[],
    },
    Operation {
        name: "issue.linked_prs",
        aliases: &["issue.linked-prs"],
    },
    Operation {
        name: "issue.monitor.config.set",
        aliases: &["issue.monitor.config-set"],
    },
    Operation {
        name: "issue.monitor.failover",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.launch_now",
        aliases: &["issue.monitor.launch-now"],
    },
    Operation {
        name: "issue.monitor.priority.move",
        aliases: &["issue.monitor.priority-move"],
    },
    Operation {
        name: "issue.monitor.priority.set",
        aliases: &["issue.monitor.priority-set"],
    },
    Operation {
        name: "issue.monitor.profiles",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.profiles.set",
        aliases: &["issue.monitor.profiles-set"],
    },
    Operation {
        name: "issue.monitor.question.answer",
        aliases: &["issue.monitor.question-answer"],
    },
    Operation {
        name: "issue.monitor.questions",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.quota_hold.clear",
        aliases: &["issue.monitor.quota-hold.clear"],
    },
    Operation {
        name: "issue.monitor.quota_hold.list",
        aliases: &["issue.monitor.quota-hold.list"],
    },
    Operation {
        name: "issue.monitor.reconcile",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.release_idle",
        aliases: &["issue.monitor.release-idle"],
    },
    Operation {
        name: "issue.monitor.requeue",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.review_verdict",
        aliases: &["issue.monitor.review-verdict"],
    },
    Operation {
        name: "issue.monitor.status",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.stop",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.wait",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.wait.invalidate",
        aliases: &["issue.monitor.wait-invalidate"],
    },
    Operation {
        name: "issue.reopen",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.audit",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.create",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.edit",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.list",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.pull",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.read",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.rename",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.repair",
        aliases: &[],
    },
    Operation {
        name: "issue.spec.section",
        aliases: &[],
    },
    Operation {
        name: "issue.view",
        aliases: &[],
    },
    Operation {
        name: "memory.add",
        aliases: &[],
    },
    Operation {
        name: "pane.close",
        aliases: &["pane.stop"],
    },
    Operation {
        name: "pane.list",
        aliases: &[],
    },
    Operation {
        name: "pane.read",
        aliases: &[],
    },
    Operation {
        name: "pane.recover",
        aliases: &[],
    },
    Operation {
        name: "pane.send",
        aliases: &[],
    },
    Operation {
        name: "perf.startup",
        aliases: &[],
    },
    Operation {
        name: "perf.summary",
        aliases: &[],
    },
    Operation {
        name: "perf.violations",
        aliases: &[],
    },
    Operation {
        name: "plan.abort",
        aliases: &[],
    },
    Operation {
        name: "plan.complete",
        aliases: &[],
    },
    Operation {
        name: "plan.phase",
        aliases: &[],
    },
    Operation {
        name: "plan.start",
        aliases: &[],
    },
    Operation {
        name: "pm.message.send",
        aliases: &["pm.pane.send"],
    },
    Operation {
        name: "pm.status",
        aliases: &[],
    },
    Operation {
        name: "pm.stop",
        aliases: &["pm.deregister"],
    },
    Operation {
        name: "pr.checks",
        aliases: &[],
    },
    Operation {
        name: "pr.comment",
        aliases: &[],
    },
    Operation {
        name: "pr.create",
        aliases: &[],
    },
    Operation {
        name: "pr.current",
        aliases: &[],
    },
    Operation {
        name: "pr.draft",
        aliases: &[],
    },
    Operation {
        name: "pr.edit",
        aliases: &[],
    },
    Operation {
        name: "pr.list",
        aliases: &[],
    },
    Operation {
        name: "pr.ready",
        aliases: &[],
    },
    Operation {
        name: "pr.review_threads",
        aliases: &["pr.review-threads"],
    },
    Operation {
        name: "pr.review_threads.reply_and_resolve",
        aliases: &["pr.review-threads.reply-and-resolve"],
    },
    Operation {
        name: "pr.reviews",
        aliases: &[],
    },
    Operation {
        name: "pr.update_branch",
        aliases: &["pr.update-branch"],
    },
    Operation {
        name: "pr.view",
        aliases: &[],
    },
    Operation {
        name: "register.abort",
        aliases: &[],
    },
    Operation {
        name: "register.complete",
        aliases: &[],
    },
    Operation {
        name: "register.phase",
        aliases: &[],
    },
    Operation {
        name: "register.start",
        aliases: &[],
    },
    Operation {
        name: "release.status",
        aliases: &[],
    },
    Operation {
        name: "search",
        aliases: &[],
    },
    Operation {
        name: "verify.adjudicate",
        aliases: &[],
    },
    Operation {
        name: "verify.lease.acquire",
        aliases: &["verify.lease-acquire"],
    },
    Operation {
        name: "verify.lease.extend",
        aliases: &["verify.lease-extend"],
    },
    Operation {
        name: "verify.lease.hold",
        aliases: &[],
    },
    Operation {
        name: "verify.lease.release",
        aliases: &["verify.lease-release"],
    },
    Operation {
        name: "verify.lease.status",
        aliases: &["verify.lease-status"],
    },
    Operation {
        name: "verify.plan",
        aliases: &[],
    },
    Operation {
        name: "verify.run",
        aliases: &[],
    },
    Operation {
        name: "workflow.bypass",
        aliases: &[],
    },
    Operation {
        name: "workspace.candidates",
        aliases: &[],
    },
    Operation {
        name: "workspace.create",
        aliases: &[],
    },
    Operation {
        name: "workspace.ensure",
        aliases: &[],
    },
    Operation {
        name: "workspace.join",
        aliases: &[],
    },
    Operation {
        name: "workspace.projection_list",
        aliases: &["workspace.projection-list"],
    },
    Operation {
        name: "workspace.projection_prune",
        aliases: &["workspace.projection-prune"],
    },
    Operation {
        name: "workspace.store_consolidate",
        aliases: &["workspace.store-consolidate"],
    },
    Operation {
        name: "workspace.update",
        aliases: &[],
    },
    Operation {
        name: "workspace.work_prune",
        aliases: &["workspace.work-prune"],
    },
    Operation {
        name: "worktree.gc_build_artifacts",
        aliases: &["worktree.gc-build-artifacts"],
    },
];

/// Canonical spellings only — the names worth suggesting.
pub fn canonical_names() -> impl Iterator<Item = &'static str> {
    OPERATIONS.iter().map(|operation| operation.name)
}

/// Every spelling that resolves, aliases included.
pub fn names() -> impl Iterator<Item = &'static str> {
    OPERATIONS.iter().flat_map(|operation| {
        std::iter::once(operation.name).chain(operation.aliases.iter().copied())
    })
}

/// The `namespace.` an operation name sits in, or `None` for a bare name.
fn namespace_of(name: &str) -> Option<&str> {
    name.find('.').map(|index| &name[..index + 1])
}

fn segments(name: &str) -> Vec<&str> {
    name.split(['.', '_', '-'])
        .filter(|s| !s.is_empty())
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ac) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, bc) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ac != bc);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Implemented operations a caller who typed `unknown` most likely meant.
///
/// Segment containment comes first, because the misses that actually happen
/// are renames rather than typos: `workspace.prune` is every segment of
/// `workspace.projection_prune` minus one, a distance no edit-distance
/// threshold tight enough to be useful would ever accept. Edit distance is the
/// fallback for genuine typos. At most three, closest first.
pub fn suggestions(unknown: &str) -> Vec<&'static str> {
    let wanted = segments(unknown);
    let mut scored: Vec<(usize, usize, &'static str)> = Vec::new();
    for candidate in canonical_names() {
        if candidate == unknown {
            continue;
        }
        let candidate_segments = segments(candidate);
        let contains_all = !wanted.is_empty()
            && wanted
                .iter()
                .all(|segment| candidate_segments.contains(segment));
        let distance = levenshtein(unknown, candidate);
        if contains_all {
            // Rank the tightest superset first: `workspace.work_prune` and
            // `workspace.projection_prune` both qualify, and the caller wants
            // to see both, shortest detour first.
            scored.push((0, candidate.len(), candidate));
        } else if distance <= unknown.len().div_ceil(4).max(2) {
            scored.push((1, distance, candidate));
        }
    }
    scored.sort();
    scored.truncate(3);
    scored.into_iter().map(|(_, _, name)| name).collect()
}

/// The `unknown subcommand` response, with the candidates a caller needs to
/// fix the call without reading the dispatch source.
///
/// The existing wording opens the message unchanged so log greps still match;
/// the candidates follow it. When nothing is close, the namespace's actual
/// operations are listed rather than nothing — an invented name like
/// `pr.merge` was never a typo of anything, and the caller still has to learn
/// what `pr.` does have.
///
/// Also reached by the argv parser, whose unknown values are flags and
/// subcommand words rather than operations — `--draft` would otherwise collect
/// `pr.draft` as a candidate and send the caller to the wrong surface. Only a
/// dotted name is treated as an attempted operation.
pub fn unknown_subcommand_message(unknown: &str) -> String {
    let mut message = format!("unknown subcommand: {unknown}");
    if !unknown.contains('.') {
        return message;
    }
    let suggestions = suggestions(unknown);
    if !suggestions.is_empty() {
        let names: Vec<String> = suggestions.iter().map(|name| format!("`{name}`")).collect();
        message.push_str(&format!(" (did you mean {}?)", names.join(", ")));
        return message;
    }
    let Some(namespace) = namespace_of(unknown) else {
        return message;
    };
    let siblings: Vec<&str> = canonical_names()
        .filter(|name| name.starts_with(namespace))
        .collect();
    if siblings.is_empty() {
        return message;
    }
    message.push_str(&format!(
        " (no operation by that name; `{namespace}` operations: {})",
        siblings.join(", ")
    ));
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The anti-drift guard. `json_envelope.rs` dispatches on string literals
    /// in one `match`; this reads those literals back out of the source and
    /// requires the catalog to be exactly that set. Without it the catalog is
    /// one more hand-written list waiting to rot — and a suggestion drawn from
    /// a rotted list is worse than no suggestion, because the caller believes
    /// it.
    #[test]
    fn catalog_covers_every_dispatched_operation() {
        let source = include_str!("json_envelope.rs");
        let mut dispatched: std::collections::BTreeSet<String> = Default::default();
        let mut inside = false;
        for line in source.lines() {
            if line.contains("let command = match envelope.operation.as_str() {") {
                inside = true;
                continue;
            }
            if !inside {
                continue;
            }
            if line.starts_with("        other => {") {
                break;
            }
            let arm = line
                .strip_prefix("        \"")
                .map(|_| line)
                .or_else(|| line.strip_prefix("        | \"").map(|_| line));
            let Some(arm) = arm else { continue };
            for literal in arm.split('"').skip(1).step_by(2) {
                dispatched.insert(literal.to_string());
            }
        }
        assert!(
            dispatched.len() > 100,
            "the dispatch scan found only {} operations; the match shape changed and this guard is no longer reading it",
            dispatched.len()
        );
        let catalogued: std::collections::BTreeSet<String> =
            names().map(|name| name.to_string()).collect();
        let missing: Vec<&String> = dispatched.difference(&catalogued).collect();
        let phantom: Vec<&String> = catalogued.difference(&dispatched).collect();
        assert!(
            missing.is_empty(),
            "json_envelope.rs dispatches operations the catalog does not list: {missing:?}"
        );
        assert!(
            phantom.is_empty(),
            "the catalog lists operations json_envelope.rs does not dispatch: {phantom:?}"
        );
    }

    /// Aliases resolve, so they belong in the drift check, but suggesting one
    /// would teach the caller the spelling gwt does not print anywhere else.
    #[test]
    fn aliases_resolve_but_are_never_suggested() {
        assert!(names().any(|name| name == "pr.update-branch"));
        assert!(!canonical_names().any(|name| name == "pr.update-branch"));
        assert!(!suggestions("pr.update-branchh").contains(&"pr.update-branch"));
    }

    /// An argv flag is not an operation: it matches nothing and has no
    /// namespace, so it must keep the bare message rather than collect a
    /// nonsense candidate list.
    #[test]
    fn a_bare_argv_flag_keeps_the_unchanged_message() {
        assert_eq!(
            unknown_subcommand_message("--draft"),
            "unknown subcommand: --draft"
        );
        assert_eq!(
            unknown_subcommand_message("card"),
            "unknown subcommand: card"
        );
    }
}
