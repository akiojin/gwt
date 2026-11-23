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
        name: "actions.cancel",
        aliases: &[],
    },
    Operation {
        name: "actions.diagnostic.cancel",
        aliases: &[],
    },
    Operation {
        name: "actions.diagnostic.observe",
        aliases: &[],
    },
    Operation {
        name: "actions.diagnostic.policy.add",
        aliases: &[],
    },
    Operation {
        name: "actions.diagnostic.policy.rollback",
        aliases: &[],
    },
    Operation {
        name: "actions.diagnostic.policy.status",
        aliases: &[],
    },
    Operation {
        name: "actions.diagnostic.request",
        aliases: &[],
    },
    Operation {
        name: "actions.dispatch",
        aliases: &[],
    },
    Operation {
        name: "actions.job_logs",
        aliases: &["actions.job-logs"],
    },
    Operation {
        name: "actions.logs",
        aliases: &[],
    },
    Operation {
        name: "actions.queued",
        aliases: &[],
    },
    Operation {
        name: "actions.release_policy.add",
        aliases: &[],
    },
    Operation {
        name: "actions.release_policy.rollback",
        aliases: &[],
    },
    Operation {
        name: "actions.release_policy.status",
        aliases: &[],
    },
    Operation {
        name: "actions.rerun",
        aliases: &[],
    },
    Operation {
        name: "agent.registry.reconcile",
        aliases: &[],
    },
    Operation {
        name: "agent.registry.restore_schema3",
        aliases: &[],
    },
    Operation {
        name: "agent.registry.status",
        aliases: &[],
    },
    Operation {
        name: "board.config.set",
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
        name: "board.sync",
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
        name: "decision.answer",
        aliases: &[],
    },
    Operation {
        name: "decision.attach",
        aliases: &[],
    },
    Operation {
        name: "decision.defer",
        aliases: &[],
    },
    Operation {
        name: "decision.list",
        aliases: &[],
    },
    Operation {
        name: "decision.merge",
        aliases: &[],
    },
    Operation {
        name: "decision.register",
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
        name: "discussion.create",
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
        name: "execution.no_action",
        aliases: &["execution.no-action"],
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
        name: "execution.settle_delivered",
        aliases: &[],
    },
    Operation {
        name: "execution.status",
        aliases: &[],
    },
    Operation {
        name: "feedback.create",
        aliases: &[],
    },
    Operation {
        name: "feedback.process",
        aliases: &[],
    },
    Operation {
        name: "feedback.retry",
        aliases: &[],
    },
    Operation {
        name: "feedback.status",
        aliases: &[],
    },
    Operation {
        name: "fleet.disposition",
        aliases: &[],
    },
    Operation {
        name: "fleet.inventory",
        aliases: &[],
    },
    Operation {
        name: "fleet.orphan.terminate",
        aliases: &[],
    },
    Operation {
        name: "fleet.orphans",
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
        name: "issue.cache.repair",
        aliases: &[],
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
        name: "issue.monitor.tiers",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.tiers.set",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.tier.set",
        aliases: &[],
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
    // The operator's hand on the implementation queue: list what is queued,
    // put an Issue in, take one out, and reorder what is left.
    Operation {
        name: "issue.monitor.queue.adopt",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.queue.auto_refill",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.queue.demote",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.queue.list",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.queue.move",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.queue.push",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.queue.remove",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.queue.urgent_limit",
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
        name: "issue.monitor.recovery.record",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.review_verdict",
        aliases: &["issue.monitor.review-verdict"],
    },
    Operation {
        name: "issue.monitor.schedule.read",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.schedule.reassign",
        aliases: &[],
    },
    // The source Session acknowledges its own verified saved checkpoint.
    Operation {
        name: "issue.monitor.switch.ack",
        aliases: &[],
    },
    // The registered PM may cancel before the captured runtime is stopped.
    Operation {
        name: "issue.monitor.switch.cancel",
        aliases: &[],
    },
    Operation {
        name: "issue.monitor.switch.read",
        aliases: &[],
    },
    // The registered PM requests a checkpointed switch to the next planned Issue.
    Operation {
        name: "issue.monitor.switch.request",
        aliases: &[],
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
        name: "issue.monitor.verification.declare",
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
        name: "issue.spec.inspection.complete",
        aliases: &["issue.spec.inspection-complete"],
    },
    Operation {
        name: "issue.spec.lint",
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
        name: "pane.health",
        aliases: &[],
    },
    Operation {
        name: "pane.layout.place",
        aliases: &[],
    },
    Operation {
        name: "pane.list",
        aliases: &[],
    },
    Operation {
        name: "pane.name",
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
        name: "pm.agent.add",
        aliases: &[],
    },
    Operation {
        name: "pm.approval.request",
        aliases: &[],
    },
    Operation {
        name: "pm.approval.revoke",
        aliases: &[],
    },
    Operation {
        name: "pm.approval.status",
        aliases: &[],
    },
    Operation {
        name: "pm.auto_start.set",
        aliases: &[],
    },
    Operation {
        name: "pm.capabilities",
        aliases: &[],
    },
    Operation {
        name: "pm.operation.execute",
        aliases: &[],
    },
    Operation {
        name: "pm.peer.send",
        aliases: &[],
    },
    Operation {
        name: "pm.plan.apply",
        aliases: &[],
    },
    Operation {
        name: "pm.plan.status",
        aliases: &[],
    },
    Operation {
        name: "pm.message.send",
        aliases: &["pm.pane.send"],
    },
    Operation {
        name: "pm.request_tasks.apply",
        aliases: &[],
    },
    Operation {
        name: "pm.request_tasks.read",
        aliases: &[],
    },
    Operation {
        name: "pm.request_tasks.capture",
        aliases: &[],
    },
    Operation {
        name: "pm.status",
        aliases: &[],
    },
    Operation {
        name: "pm.status.set",
        aliases: &[],
    },
    Operation {
        name: "pm.report.post",
        aliases: &[],
    },
    Operation {
        name: "pm.report.list",
        aliases: &[],
    },
    Operation {
        name: "pm.theme.record",
        aliases: &[],
    },
    Operation {
        name: "pm.theme.list",
        aliases: &[],
    },
    Operation {
        name: "pm.pause",
        aliases: &[],
    },
    Operation {
        name: "pm.resume",
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
        name: "pr.close",
        aliases: &[],
    },
    Operation {
        name: "pr.auto_merge.enable",
        aliases: &[],
    },
    Operation {
        name: "pr.reopen",
        aliases: &[],
    },
    Operation {
        name: "pr.review.submit",
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
        name: "pr.create_from_ci",
        aliases: &[],
    },
    Operation {
        name: "pr.current",
        aliases: &[],
    },
    Operation {
        name: "pr.edit",
        aliases: &[],
    },
    Operation {
        name: "pr.head_check",
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
        name: "project.execution.choose",
        aliases: &[],
    },
    Operation {
        name: "project.execution.status",
        aliases: &[],
    },
    Operation {
        name: "project.open",
        aliases: &[],
    },
    Operation {
        name: "provider.reset",
        aliases: &[],
    },
    Operation {
        name: "provider.reset.proposals",
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
        name: "release.metadata.set",
        aliases: &[],
    },
    Operation {
        name: "release.update.defer",
        aliases: &[],
    },
    Operation {
        name: "repository.required_checks.update",
        aliases: &[],
    },
    Operation {
        name: "repository.settings.set",
        aliases: &[],
    },
    Operation {
        name: "search",
        aliases: &[],
    },
    Operation {
        name: "update.settings.set",
        aliases: &[],
    },
    Operation {
        name: "update.settings.status",
        aliases: &[],
    },
    Operation {
        name: "verify.adjudicate",
        aliases: &[],
    },
    Operation {
        name: "verify.cancel",
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
        name: "verify.status",
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
        name: "workspace.events_reconcile",
        aliases: &[],
    },
    Operation {
        name: "workspace.join",
        aliases: &[],
    },
    Operation {
        name: "workspace.project_prune",
        aliases: &[],
    },
    Operation {
        name: "workspace.project_scan",
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
        name: "workspace.receipt",
        aliases: &[],
    },
    Operation {
        name: "workspace.reopen",
        aliases: &[],
    },
    Operation {
        name: "workspace.repair_close_proof",
        aliases: &[],
    },
    Operation {
        name: "workspace.close",
        aliases: &[],
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
        name: "workspace.work_repair",
        aliases: &[],
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

/// The help family an operation belongs to: everything before the first `.`,
/// or the whole name for a bare operation such as `search`.
pub fn family_of(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

/// Every help family the catalog implies, sorted and deduplicated.
pub fn families() -> Vec<&'static str> {
    canonical_names()
        .map(family_of)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The operation list of `gwtd --help <family>`, generated from the catalog
/// (Issue #7522).
///
/// The family help texts are hand-written prose and drifted: 87 of the
/// canonical operations were absent from them and twelve families had no help
/// at all. Appending this section to every family help makes the listing
/// complete by construction, so a newly dispatched operation is discoverable
/// the moment `catalog_covers_every_dispatched_operation` forces it into the
/// catalog. Aliases resolve but are never recommended, so only canonical
/// names are printed. `None` for a family the catalog does not know.
pub fn family_operations_section(family: &str) -> Option<String> {
    let operations: Vec<&'static str> = canonical_names()
        .filter(|name| family_of(name) == family)
        .collect();
    if operations.is_empty() {
        return None;
    }
    let mut section = format!(
        "All {family} operations ({}, generated from the operation catalog):\n",
        operations.len()
    );
    for name in operations {
        section.push_str(&format!("  {name}\n"));
    }
    Some(section)
}

/// The `Commands:` rows of the global `gwtd --help`.
///
/// `summaries` supplies the hand-written one-line description per family and
/// the display order. A catalog family without a summary still gets a row, so
/// no family can be reachable only by reading the dispatch source.
pub fn family_index(summaries: &[(&str, &str)]) -> String {
    let mut index = String::new();
    for (family, summary) in summaries {
        index.push_str(&format!("  {family:<11} {summary}\n"));
    }
    for family in families() {
        if summaries.iter().any(|(described, _)| *described == family) {
            continue;
        }
        let count = canonical_names()
            .filter(|name| family_of(name) == family)
            .count();
        index.push_str(&format!(
            "  {family:<11} {count} operation(s); see gwtd --help {family}\n"
        ));
    }
    index
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
/// `pr.nonexistent_operation` is no typo, and the caller still has to learn
/// what `pr.` does have.
///
/// Also reached by the argv parser, whose unknown values are flags and
/// subcommand words rather than operations. Only a
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
        assert!(
            names().any(|name| name == "actions.dispatch"),
            "Issue #4249 Phase 6 operation is callable"
        );
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

    #[test]
    fn gui_envelope_parity_ledger_tracks_frontend_events_and_catalog() {
        use std::collections::BTreeSet;

        let protocol = include_str!("../protocol.rs");
        let (_, declaration) = protocol
            .split_once("pub enum FrontendEvent {")
            .expect("FrontendEvent declaration");
        let (variants, _) = declaration.split_once("\n}").expect("FrontendEvent end");
        // Direct variants start at the rustfmt enum indentation and use
        // UpperCamelCase. Fields and nested attribute arguments do not.
        let expected: BTreeSet<_> = variants
            .lines()
            .filter_map(|line| line.strip_prefix("    "))
            .filter(|line| line.starts_with(|ch: char| ch.is_ascii_uppercase()))
            .map(|line| {
                line.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .next()
                    .expect("variant name")
            })
            .collect();
        assert!(
            !expected.is_empty(),
            "FrontendEvent declaration no longer matches the scan"
        );

        let ledger = include_str!("../../../../docs/gui-envelope-parity.md");
        let mut recorded = BTreeSet::new();
        for row in ledger.lines().filter(|line| line.starts_with("| `")) {
            let columns: Vec<_> = row.trim_matches('|').split('|').map(str::trim).collect();
            let [event, classification, operations, reason] = columns.as_slice() else {
                panic!("expected four cells: {row}");
            };
            let event = event.trim_matches('`');
            assert!(recorded.insert(event), "duplicate FrontendEvent: {event}");
            assert!(!reason.is_empty(), "missing rationale for {event}");
            match *classification {
                "対応" | "一部対応" => {
                    for operation in operations.split(',').map(str::trim) {
                        let operation = operation.trim_matches('`');
                        assert!(
                            canonical_names().any(|name| name == operation),
                            "{event} references a noncanonical or missing operation: {operation}"
                        );
                    }
                }
                "GUI 内部" | "対象外" | "読み取り" => {
                    assert_eq!(*operations, "—", "unexpected operation for {event}");
                }
                _ => panic!("unknown classification for {event}: {classification}"),
            }
        }
        let missing: Vec<_> = expected.difference(&recorded).copied().collect();
        let stale: Vec<_> = recorded.difference(&expected).copied().collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "FrontendEvent parity ledger drift: missing {missing:?}, stale {stale:?}"
        );
    }

    /// Issue #7522: the generated section is the whole family, canonical
    /// spellings only, and a bare operation is its own family.
    #[test]
    fn family_operations_section_lists_exactly_the_family() {
        for family in families() {
            let section = family_operations_section(family).expect("catalog family");
            let listed: Vec<&str> = section.lines().skip(1).map(str::trim).collect();
            let expected: Vec<&str> = canonical_names()
                .filter(|name| family_of(name) == family)
                .collect();
            assert_eq!(listed, expected, "{family}");
        }
        assert_eq!(family_of("search"), "search");
        assert_eq!(family_of("issue.spec.read"), "issue");
        let pr = family_operations_section("pr").unwrap();
        assert!(pr.contains("  pr.update_branch\n"), "{pr}");
        assert!(
            !pr.contains("pr.update-branch"),
            "aliases stay unlisted: {pr}"
        );
        assert_eq!(family_operations_section("nonexistent"), None);
    }

    /// Issue #7522: a family nobody wrote a summary for still gets a row.
    #[test]
    fn family_index_adds_a_row_for_every_undescribed_family() {
        let index = family_index(&[("issue", "Manage GitHub Issues"), ("lessons", "Alias")]);
        assert!(index.starts_with("  issue       Manage GitHub Issues\n  lessons     Alias\n"));
        assert_eq!(index.matches("\n  issue ").count(), 0, "{index}");
        for family in families() {
            assert!(
                index
                    .lines()
                    .any(|line| line.starts_with(&format!("  {family} "))),
                "{family} has no row: {index}"
            );
        }
        assert!(
            index.contains("  worktree    1 operation(s); see gwtd --help worktree\n"),
            "{index}"
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
