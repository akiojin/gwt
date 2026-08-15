//! `pr.*` JSON operation family module (SPEC-1942 SC-027 split).
//!
//! - `mod.rs` (this file): argv `parse`, dispatch `run`, render helpers, and
//!   the family `tests` block.
//! - `gh.rs`: every `gh` CLI / graphql wrapper, plus the small response-shape
//!   parsers (`parse_pr_checks_*`, `parse_available_fields`, ...) that exist
//!   solely to interpret gh output.
//!
//! All `gh.rs` items are re-exported via `pub(super) use gh::*;` so external
//! callers (`cli::env`, `cli::run`, `cli::tests`) keep accessing them via
//! `super::pr::name` / `crate::cli::pr::name` exactly as before.

mod gh;

#[allow(unused_imports)]
pub(super) use gh::{
    comment_on_pr_via_gh, convert_pr_to_draft_via_gh, create_pr_via_gh, edit_or_create_repo_guard,
    edit_pr_via_gh, extract_pr_url, fetch_current_pr_via_gh, fetch_pr_checks_via_gh,
    fetch_pr_review_thread_state_via_gh, fetch_pr_review_threads_via_gh, fetch_pr_reviews_via_gh,
    mark_pr_ready_via_gh, parse_available_fields, parse_pr_checks_items_json,
    parse_pr_checks_items_response, parse_pr_number_from_url,
    reply_and_resolve_pr_review_threads_via_gh, review_thread_has_comment_body,
    should_reply_to_review_thread, should_resolve_review_thread,
};

use gwt_git::PrStatus;
use gwt_github::SpecOpsError;

use crate::cli::{CliEnv, CliParseError, PrChecksSummary, PrCommand, PrReview, PrReviewThread};

pub(super) fn parse(args: &[String]) -> Result<PrCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("current") => {
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Current)
        }
        Some("create") => parse_pr_create_args(it.collect::<Vec<_>>().as_slice()),
        Some("edit") => parse_pr_edit_args(it.collect::<Vec<_>>().as_slice()),
        Some("view") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::View { number })
        }
        Some("ready") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Ready { number })
        }
        Some("draft") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Draft { number })
        }
        Some("comment") => {
            let number = super::parse_required_number(it.next())?;
            super::expect_flag(it.next(), "-f")?;
            let file = it.next().ok_or(CliParseError::MissingFlag("-f"))?.clone();
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Comment { number, file })
        }
        Some("reviews") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Reviews { number })
        }
        Some("review-threads") => match it.next().map(String::as_str) {
            Some("reply-and-resolve") => {
                let number = super::parse_required_number(it.next())?;
                super::expect_flag(it.next(), "-f")?;
                let file = it.next().ok_or(CliParseError::MissingFlag("-f"))?.clone();
                super::ensure_no_remaining_args(it)?;
                Ok(PrCommand::ReviewThreadsReplyAndResolve { number, file })
            }
            Some(number_arg) => {
                let number = number_arg
                    .parse()
                    .map_err(|_| CliParseError::InvalidNumber(number_arg.to_string()))?;
                super::ensure_no_remaining_args(it)?;
                Ok(PrCommand::ReviewThreads { number })
            }
            None => Err(CliParseError::Usage),
        },
        Some("checks") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Checks { number })
        }
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

fn dispatch_pr_mutation<T>(
    expected: Option<&gwt_agent::SessionExecutionBinding>,
    operation: impl FnOnce() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let Some(expected) = expected else {
        return operation();
    };
    match crate::cli::execution_state::with_current_pr_mutation_execution_binding_lease(
        &gwt_core::paths::gwt_sessions_dir(),
        expected,
        operation,
    )? {
        Some(result) => result,
        None => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "PR mutation refused: the owning Session or execution generation changed before external dispatch; retry from the current owning Session",
        )),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: PrCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    // SPEC-3248 P8b (T-112/FR-037/AS-33): Ready handoffs — a non-draft PR
    // creation or `pr.ready` — require the current session's execution to be
    // settled or backed by fresh verification evidence. A terminally blocked
    // execution refuses every PR mutation, draft creation and edits
    // included; an active execution keeps the mid-work Draft flow available.
    let is_pr_mutation = matches!(
        cmd,
        PrCommand::Create { .. }
            | PrCommand::CreateBody { .. }
            | PrCommand::Edit { .. }
            | PrCommand::EditBody { .. }
            | PrCommand::Ready { .. }
    );
    let mut mutation_binding = None;
    if is_pr_mutation {
        let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
        let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        mutation_binding = match crate::cli::execution_state::snapshot_pr_mutation_execution_binding(
            &worktree,
            session_id.as_deref(),
        ) {
            Ok(binding) => binding,
            Err(_) => {
                out.push_str("PR mutation refused: current execution authority requires the exact durable owning Session and generation binding; relaunch or continue the owning Session before retrying.\n");
                return Ok(2);
            }
        };
        let is_ready_handoff = matches!(
            cmd,
            PrCommand::Create { draft: false, .. }
                | PrCommand::CreateBody { draft: false, .. }
                | PrCommand::Ready { .. }
        );
        if let Some(refusal) =
            crate::cli::execution_state::pr_handoff_refusal(env.repo_path(), is_ready_handoff)
        {
            out.push_str(&refusal);
            out.push('\n');
            return Ok(2);
        }
        if let Some(refusal) = crate::cli::verification_record::branch_push_refusal(&worktree) {
            out.push_str(&refusal);
            out.push('\n');
            return Ok(2);
        }
        // T-247: a Ready handoff with unsettled non-PR obligations is
        // premature — draft creation and edits stay available mid-work.
        if is_ready_handoff {
            if let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                if let Some(refusal) = crate::cli::action_obligation::open_obligation_refusal(
                    &worktree,
                    &session_id,
                    &[crate::cli::action_obligation::ObligationKind::Pr],
                ) {
                    out.push_str(&format!("PR handoff refused: {refusal}\n"));
                    return Ok(2);
                }
            }
        }
    }
    let cmd_settles_pr_obligation = is_pr_mutation
        || matches!(
            cmd,
            PrCommand::Comment { .. } | PrCommand::CommentBody { .. }
        );
    let code = match cmd {
        PrCommand::Current => {
            match env.fetch_current_pr().map_err(super::io_as_api_error)? {
                Some(pr) => {
                    sync_workspace_pr_metadata(env, &pr, None);
                    render_pr(out, &pr);
                }
                None => out.push_str("no current pull request\n"),
            }
            0
        }
        PrCommand::Create {
            base,
            head,
            title,
            file,
            labels,
            draft,
        } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let pr = dispatch_pr_mutation(mutation_binding.as_ref(), || {
                env.create_pr(&base, head.as_deref(), &title, &body, &labels, draft)
            })
            .map_err(super::io_as_api_error)?;
            sync_workspace_pr_metadata(env, &pr, head.as_deref());
            out.push_str("created pull request\n");
            render_pr(out, &pr);
            0
        }
        PrCommand::CreateBody {
            base,
            head,
            title,
            body,
            labels,
            draft,
        } => {
            let pr = dispatch_pr_mutation(mutation_binding.as_ref(), || {
                env.create_pr(&base, head.as_deref(), &title, &body, &labels, draft)
            })
            .map_err(super::io_as_api_error)?;
            sync_workspace_pr_metadata(env, &pr, head.as_deref());
            out.push_str("created pull request\n");
            render_pr(out, &pr);
            0
        }
        PrCommand::Edit {
            number,
            title,
            file,
            add_labels,
        } => {
            let body = file
                .as_deref()
                .map(|path| env.read_file(path).map_err(super::io_as_api_error))
                .transpose()?;
            let pr = dispatch_pr_mutation(mutation_binding.as_ref(), || {
                env.edit_pr(number, title.as_deref(), body.as_deref(), &add_labels)
            })
            .map_err(super::io_as_api_error)?;
            out.push_str("updated pull request\n");
            render_pr(out, &pr);
            0
        }
        PrCommand::EditBody {
            number,
            title,
            body,
            add_labels,
        } => {
            let pr = dispatch_pr_mutation(mutation_binding.as_ref(), || {
                env.edit_pr(number, title.as_deref(), body.as_deref(), &add_labels)
            })
            .map_err(super::io_as_api_error)?;
            out.push_str("updated pull request\n");
            render_pr(out, &pr);
            0
        }
        PrCommand::View { number } => {
            let pr = env.fetch_pr(number).map_err(super::io_as_api_error)?;
            render_pr(out, &pr);
            0
        }
        PrCommand::Ready { number } => {
            // Takes an explicit PR number like view/edit/checks, so it must not
            // sync workspace PR metadata (that path is reserved for current/create
            // where the PR is provably the workspace's own). Draft↔Ready also does
            // not change pr.state (OPEN→OPEN), so there is no metadata to refresh.
            let pr = dispatch_pr_mutation(mutation_binding.as_ref(), || env.mark_pr_ready(number))
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!("marked pull request #{number} ready for review\n"));
            render_pr(out, &pr);
            0
        }
        PrCommand::Draft { number } => {
            let pr = env
                .convert_pr_to_draft(number)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!("converted pull request #{number} to draft\n"));
            render_pr(out, &pr);
            0
        }
        PrCommand::Comment { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            env.comment_on_pr(number, &body)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!("created comment on PR #{number}\n"));
            0
        }
        PrCommand::CommentBody { number, body } => {
            env.comment_on_pr(number, &body)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!("created comment on PR #{number}\n"));
            0
        }
        PrCommand::Reviews { number } => {
            let reviews = env
                .fetch_pr_reviews(number)
                .map_err(super::io_as_api_error)?;
            render_pr_reviews(out, &reviews);
            0
        }
        PrCommand::ReviewThreads { number } => {
            let threads = env
                .fetch_pr_review_threads(number)
                .map_err(super::io_as_api_error)?;
            render_pr_review_threads(out, &threads);
            0
        }
        PrCommand::ReviewThreadsReplyAndResolve { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let resolved = env
                .reply_and_resolve_pr_review_threads(number, &body)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!(
                "replied to and resolved {resolved} review threads on PR #{number}\n"
            ));
            0
        }
        PrCommand::ReviewThreadsReplyAndResolveBody { number, body } => {
            let resolved = env
                .reply_and_resolve_pr_review_threads(number, &body)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!(
                "replied to and resolved {resolved} review threads on PR #{number}\n"
            ));
            0
        }
        PrCommand::Checks { number } => {
            let report = env
                .fetch_pr_checks(number)
                .map_err(super::io_as_api_error)?;
            render_pr_checks(out, &report);
            0
        }
    };
    if cmd_settles_pr_obligation && code == 0 {
        // P11: successful PR mutations settle open PR obligations.
        if let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            crate::cli::action_obligation::settle_kinds_best_effort(
                &worktree,
                &session_id,
                &[crate::cli::action_obligation::ObligationKind::Pr],
                "pr mutation",
            );
        }
    }
    Ok(code)
}

fn sync_workspace_pr_metadata<E: CliEnv>(env: &E, pr: &PrStatus, requested_head: Option<&str>) {
    let work_item_id = gwt_core::workspace_projection::mutate_existing_workspace_projection(
        env.repo_path(),
        |projection| {
            let stored_branch = projection
                .git_details
                .as_ref()
                .and_then(|details| details.branch.as_deref());
            if !should_sync_workspace_pr_metadata(env.repo_path(), stored_branch, requested_head) {
                return Ok(None);
            }
            let Some(details) = projection.git_details.as_mut() else {
                return Ok(None);
            };
            details.pr_number = Some(pr.number);
            details.pr_state = Some(pr.state.to_string());
            details.pr_url = (!pr.url.trim().is_empty()).then_some(pr.url.clone());
            details.pr_created_at = pr.created_at;
            projection.updated_at = chrono::Utc::now();
            Ok(Some(projection.id.clone()))
        },
    )
    .ok()
    .flatten()
    .flatten();

    // SPEC-2359 US-37 / FR-117: auto-emit Done for the linked Workspace WorkItem
    // when the PR transitions to merged. The helper is idempotent per work_item_id,
    // so repeated polling does not duplicate Done events.
    if pr.state.to_string().eq_ignore_ascii_case("merged") {
        let Some(work_item_id) = work_item_id else {
            return;
        };
        let _ = gwt_core::workspace_projection::emit_workspace_done_event_if_absent(
            env.repo_path(),
            &work_item_id,
            chrono::Utc::now(),
        );
    }
}

fn should_sync_workspace_pr_metadata(
    repo_path: &std::path::Path,
    stored_branch: Option<&str>,
    requested_head: Option<&str>,
) -> bool {
    let current_branch = current_branch_name(repo_path);
    if let Some(requested_head) = requested_head {
        return requested_head_matches_workspace_branch(
            repo_path,
            requested_head,
            stored_branch,
            current_branch.as_deref(),
        );
    }
    if let (Some(current_branch), Some(stored_branch)) = (current_branch.as_deref(), stored_branch)
    {
        return current_branch == stored_branch;
    }
    true
}

fn requested_head_matches_workspace_branch(
    repo_path: &std::path::Path,
    requested_head: &str,
    stored_branch: Option<&str>,
    current_branch: Option<&str>,
) -> bool {
    let (requested_owner, requested_branch) = split_head_owner_and_branch(requested_head);
    if let Some(requested_owner) = requested_owner {
        let Some((repo_owner, _repo_name)) = gh::github_remote_owner_and_repo(repo_path) else {
            return false;
        };
        if !requested_owner.eq_ignore_ascii_case(&repo_owner) {
            return false;
        }
    }
    stored_branch == Some(requested_branch) || current_branch == Some(requested_branch)
}

fn split_head_owner_and_branch(head: &str) -> (Option<&str>, &str) {
    match head.split_once(':') {
        Some((owner, branch)) if !owner.is_empty() && !branch.is_empty() => (Some(owner), branch),
        _ => (None, head),
    }
}

fn current_branch_name(repo_path: &std::path::Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["branch", "--show-current"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

fn parse_pr_create_args(args: &[&String]) -> Result<PrCommand, CliParseError> {
    let mut base: Option<String> = None;
    let mut head: Option<String> = None;
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();
    let mut draft = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--base" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--base"));
                }
                base = Some(args[i].clone());
            }
            "--head" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--head"));
                }
                head = Some(args[i].clone());
            }
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--label"));
                }
                labels.push(args[i].clone());
            }
            "--draft" => draft = true,
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }
    Ok(PrCommand::Create {
        base: base.ok_or(CliParseError::MissingFlag("--base"))?,
        head,
        title: title.ok_or(CliParseError::MissingFlag("--title"))?,
        file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        labels,
        draft,
    })
}

fn parse_pr_edit_args(args: &[&String]) -> Result<PrCommand, CliParseError> {
    let Some(number_arg) = args.first() else {
        return Err(CliParseError::Usage);
    };
    let number = number_arg
        .parse()
        .map_err(|_| CliParseError::InvalidNumber((*number_arg).clone()))?;
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut add_labels: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--add-label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--add-label"));
                }
                add_labels.push(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }
    if title.is_none() && file.is_none() && add_labels.is_empty() {
        return Err(CliParseError::Usage);
    }
    Ok(PrCommand::Edit {
        number,
        title,
        file,
        add_labels,
    })
}

pub(super) fn render_pr(out: &mut String, pr: &PrStatus) {
    out.push_str(&format!("#{} [{}] {}\n", pr.number, pr.state, pr.title));
    out.push_str(&format!("url: {}\n", pr.url));
    out.push_str(&format!("ci: {}\n", pr.ci_status));
    out.push_str(&format!("mergeable: {}\n", pr.effective_merge_status()));
    out.push_str(&format!("merge_state: {}\n", pr.merge_state_status));
    out.push_str(&format!("review: {}\n", pr.review_status));
}

pub(super) fn render_pr_checks(out: &mut String, summary: &PrChecksSummary) {
    out.push_str(&format!("summary: {}\n", summary.summary));
    out.push_str(&format!("ci: {}\n", summary.ci_status));
    out.push_str(&format!("merge: {}\n", summary.merge_status));
    out.push_str(&format!("review: {}\n", summary.review_status));
    if summary.checks.is_empty() {
        out.push_str("no checks\n");
        return;
    }
    for check in &summary.checks {
        out.push_str(&format!(
            "- {} [{} / {}]\n",
            check.name, check.state, check.conclusion
        ));
        if !check.workflow.is_empty() {
            out.push_str(&format!("  workflow: {}\n", check.workflow));
        }
        if !check.url.is_empty() {
            out.push_str(&format!("  url: {}\n", check.url));
        }
    }
}

pub(super) fn render_pr_reviews(out: &mut String, reviews: &[PrReview]) {
    if reviews.is_empty() {
        out.push_str("no reviews\n");
        return;
    }
    for review in reviews {
        out.push_str(&format!(
            "=== review:{} [{}] by {} at {} ===\n",
            review.id, review.state, review.author, review.submitted_at
        ));
        if !review.body.is_empty() {
            out.push_str(&review.body);
            out.push('\n');
        }
    }
}

pub(super) fn render_pr_review_threads(out: &mut String, threads: &[PrReviewThread]) {
    if threads.is_empty() {
        out.push_str("no review threads\n");
        return;
    }
    for thread in threads {
        out.push_str(&format!(
            "=== thread:{} resolved={} outdated={} path={} line={} ===\n",
            thread.id,
            thread.is_resolved,
            thread.is_outdated,
            if thread.path.is_empty() {
                "-"
            } else {
                thread.path.as_str()
            },
            thread
                .line
                .map(|line| line.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        for comment in &thread.comments {
            out.push_str(&format!(
                "--- comment:{} by {} ({}) ---\n{}\n",
                comment.id, comment.author, comment.updated_at, comment.body
            ));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn seeded_pr() -> gwt_git::PrStatus {
        gwt_git::PrStatus {
            number: 7,
            title: "CLI family split".to_string(),
            state: gwt_git::pr_status::PrState::Open,
            url: "https://example.com/pr/7".to_string(),
            created_at: None,
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            review_status: "APPROVED".to_string(),
        }
    }

    fn persist_pr_generation_session(
        worktree: &std::path::Path,
        session_id: &str,
        identity: gwt_agent::ExecutionBindingIdentity,
    ) {
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 42,
        };
        let mut session =
            gwt_agent::Session::new(worktree, "work/pr-authority", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.project_state_root = Some(worktree.to_path_buf());
        session.linked_issue_number = Some(owner.number);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session_id.to_string(),
            repo_hash: session.repo_hash.clone().expect("fixture repository hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        session
            .set_execution_binding(Some(binding))
            .expect("bind PR authority Session");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("persist PR authority Session");
    }

    fn initialize_pr_generation_authority(
        worktree: &std::path::Path,
        session_id: &str,
    ) -> gwt_agent::ExecutionBindingIdentity {
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 42,
        };
        crate::cli::execution_state::materialize_at_launch(
            worktree,
            owner.kind,
            owner.number,
            session_id,
            "launch",
            false,
        )
        .expect("materialize PR execution authority");
        crate::cli::execution_state::ensure_generation_ledger(
            worktree,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize PR generation ledger");
        crate::cli::execution_state::current_execution_binding(worktree, owner)
            .expect("read PR generation authority")
            .expect("current PR generation binding")
    }

    fn assert_pr_mutations_refuse_without_caller_authority(
        worktree: &std::path::Path,
        session_id: Option<&str>,
    ) {
        let _session = match session_id {
            Some(session_id) => {
                gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id)
            }
            None => gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV),
        };
        let mut env = crate::cli::TestEnv::new(worktree.to_path_buf());
        env.seed_pr(7, seeded_pr());
        env.seed_created_pr(seeded_pr());
        let mutations = [
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("ready"),
                body: s("body"),
                labels: vec![],
                draft: false,
            },
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("draft"),
                body: s("body"),
                labels: vec![],
                draft: true,
            },
            PrCommand::EditBody {
                number: 7,
                title: Some(s("updated")),
                body: None,
                add_labels: vec![],
            },
            PrCommand::Ready { number: 7 },
        ];
        for mutation in mutations {
            let mut out = String::new();
            let code = run(&mut env, mutation, &mut out).expect("run PR mutation gate");
            assert_eq!(code, 2, "unauthorized PR mutation was not refused: {out}");
            assert!(
                out.contains("authority"),
                "refusal must describe the caller authority requirement without reflecting identity: {out}"
            );
        }
        assert!(env.pr_create_call_log.is_empty(), "create reached GitHub");
        assert!(env.pr_edit_call_log.is_empty(), "edit reached GitHub");
        assert!(env.pr_ready_call_log.is_empty(), "ready reached GitHub");
    }

    #[test]
    fn pr_mutations_fail_closed_for_missing_or_foreign_generation_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let worktree = tempfile::tempdir().expect("PR authority repository");
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let identity = initialize_pr_generation_authority(worktree.path(), "session-current");
        persist_pr_generation_session(worktree.path(), "session-current", identity.clone());
        persist_pr_generation_session(worktree.path(), "session-foreign-secret", identity);

        assert_pr_mutations_refuse_without_caller_authority(worktree.path(), None);
        assert_pr_mutations_refuse_without_caller_authority(
            worktree.path(),
            Some("session-foreign-secret"),
        );
    }

    #[test]
    fn pr_mutations_fail_closed_for_missing_or_stale_durable_session_binding() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let worktree = tempfile::tempdir().expect("PR authority repository");
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let mut identity = initialize_pr_generation_authority(worktree.path(), "session-current");

        assert_pr_mutations_refuse_without_caller_authority(
            worktree.path(),
            Some("session-current"),
        );

        identity.binding_id.push_str("-stale");
        persist_pr_generation_session(worktree.path(), "session-current", identity);
        assert_pr_mutations_refuse_without_caller_authority(
            worktree.path(),
            Some("session-current"),
        );
    }

    #[test]
    fn pr_mutations_fail_closed_when_execution_authority_is_unreadable() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let worktree = tempfile::tempdir().expect("PR authority repository");
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let identity = initialize_pr_generation_authority(worktree.path(), "session-current");
        persist_pr_generation_session(worktree.path(), "session-current", identity);
        let trusted_record = crate::cli::trusted_store::trusted_dir_for_worktree(worktree.path())
            .expect("trusted worktree directory")
            .join("execution-control.json");
        std::fs::remove_file(&trusted_record).expect("remove readable trusted authority");
        std::fs::create_dir(&trusted_record).expect("replace authority with unreadable directory");
        assert!(
            crate::cli::execution_state::pr_handoff_refusal(worktree.path(), false).is_some(),
            "the PR-specific mutation gate itself must fail closed on an authority read error",
        );

        assert_pr_mutations_refuse_without_caller_authority(
            worktree.path(),
            Some("session-current"),
        );
    }

    #[test]
    fn pr_mutations_allow_the_exact_current_generation_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let worktree = tempfile::tempdir().expect("PR authority repository");
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let identity = initialize_pr_generation_authority(worktree.path(), "session-current");
        persist_pr_generation_session(worktree.path(), "session-current", identity.clone());
        let _session = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "session-current",
        );
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        env.seed_pr(7, seeded_pr());
        env.seed_created_pr(seeded_pr());

        let mut out = String::new();
        assert_eq!(
            run(
                &mut env,
                PrCommand::CreateBody {
                    base: s("develop"),
                    head: None,
                    title: s("draft"),
                    body: s("body"),
                    labels: vec![],
                    draft: true,
                },
                &mut out,
            )
            .expect("create draft as exact current Session"),
            0,
            "{out}",
        );
        out.clear();
        assert_eq!(
            run(
                &mut env,
                PrCommand::EditBody {
                    number: 7,
                    title: Some(s("updated")),
                    body: None,
                    add_labels: vec![],
                },
                &mut out,
            )
            .expect("edit as exact current Session"),
            0,
            "{out}",
        );

        crate::cli::verification_record::save_plan(
            worktree.path(),
            &crate::cli::verification_record::VerificationPlanRecord {
                session_id: "session-current".to_string(),
                owner_number: Some(42),
                execution_binding: Some(identity),
                commands: vec!["git --version".to_string()],
                derived: false,
                surfaces: Vec::new(),
                generated_outputs: Vec::new(),
                worktree_fingerprint: String::new(),
                created_at: chrono::Utc::now(),
                content_hash: String::new(),
            },
        )
        .expect("save generation-bound verification plan");
        crate::cli::verification_record::run_verification(
            worktree.path(),
            "session-current",
            &["git --version".to_string()],
        )
        .expect("run generation-bound verification");

        out.clear();
        assert_eq!(
            run(
                &mut env,
                PrCommand::CreateBody {
                    base: s("develop"),
                    head: None,
                    title: s("ready"),
                    body: s("body"),
                    labels: vec![],
                    draft: false,
                },
                &mut out,
            )
            .expect("create ready PR as exact current Session"),
            0,
            "{out}",
        );
        out.clear();
        assert_eq!(
            run(&mut env, PrCommand::Ready { number: 7 }, &mut out)
                .expect("mark PR ready as exact current Session"),
            0,
            "{out}",
        );

        assert_eq!(env.pr_create_call_log.len(), 2);
        assert_eq!(env.pr_edit_call_log.len(), 1);
        assert_eq!(env.pr_ready_call_log, vec![7]);
    }

    #[test]
    fn pr_mutations_allow_the_exact_completed_generation_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let worktree = tempfile::tempdir().expect("completed PR authority repository");
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let identity = initialize_pr_generation_authority(worktree.path(), "session-completed");
        persist_pr_generation_session(worktree.path(), "session-completed", identity);
        let _session = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "session-completed",
        );
        assert!(matches!(
            crate::cli::execution_state::settle(
                worktree.path(),
                "session-completed",
                crate::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle completed PR generation"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));

        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        env.seed_pr(7, seeded_pr());
        env.seed_created_pr(seeded_pr());

        let mut out = String::new();
        assert_eq!(
            run(
                &mut env,
                PrCommand::CreateBody {
                    base: s("develop"),
                    head: None,
                    title: s("completed handoff"),
                    body: s("body"),
                    labels: vec![],
                    draft: false,
                },
                &mut out,
            )
            .expect("create PR from completed generation"),
            0,
            "{out}",
        );
        out.clear();
        assert_eq!(
            run(
                &mut env,
                PrCommand::EditBody {
                    number: 7,
                    title: Some(s("completed update")),
                    body: None,
                    add_labels: vec![],
                },
                &mut out,
            )
            .expect("edit PR from completed generation"),
            0,
            "{out}",
        );
        out.clear();
        assert_eq!(
            run(&mut env, PrCommand::Ready { number: 7 }, &mut out)
                .expect("mark PR ready from completed generation"),
            0,
            "{out}",
        );

        assert_eq!(env.pr_create_call_log.len(), 1);
        assert_eq!(env.pr_edit_call_log.len(), 1);
        assert_eq!(env.pr_ready_call_log, vec![7]);
    }

    #[test]
    fn pr_create_accepts_completed_lifecycle_receipt_and_handoff_prompt() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let session_id = "session-completed-settled-handoff";
        let active_identity = initialize_pr_generation_authority(&fixture.repo, session_id);
        persist_pr_generation_session(&fixture.repo, session_id, active_identity.clone());
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);

        fixture.append_event("terminal-update-awaiting-delivery");
        let pending_receipt = crate::cli::verification_record::save_work_event_settlement_record(
            &fixture.repo,
            session_id,
            true,
        )
        .expect("record the terminal Work update before delivery");
        assert!(pending_receipt.obligation_open);
        assert!(!pending_receipt.status.is_settled());
        fixture.stage_events();
        fixture.commit("chore(work): record terminal Work event");
        fixture.push();

        let receipt = crate::cli::verification_record::save_work_event_settlement_record(
            &fixture.repo,
            session_id,
            false,
        )
        .expect("persist settled Active-generation Work receipt");
        assert!(receipt.status.is_settled());
        assert!(!receipt.obligation_open);
        assert_eq!(receipt.execution_binding, Some(active_identity.clone()));

        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut verify_out = String::new();
        assert_eq!(
            crate::cli::verification_record::run(
                &mut env,
                crate::cli::verification_record::VerifyCommand::Plan {
                    commands: vec!["git --version".to_string()],
                    derive: false,
                },
                &mut verify_out,
            )
            .expect("register canonical verification plan"),
            0,
            "{verify_out}",
        );
        verify_out.clear();
        assert_eq!(
            crate::cli::verification_record::run(
                &mut env,
                crate::cli::verification_record::VerifyCommand::Run {
                    commands: vec!["git --version".to_string()],
                },
                &mut verify_out,
            )
            .expect("run canonical verification"),
            0,
            "{verify_out}",
        );

        let mut completion_out = String::new();
        assert_eq!(
            crate::cli::execution_state::run(
                &mut env,
                crate::cli::execution_state::ExecutionCommand::Complete,
                &mut completion_out,
            )
            .expect("complete generation through the canonical command"),
            0,
            "{completion_out}",
        );
        let completed_identity = crate::cli::execution_state::current_execution_binding(
            &fixture.repo,
            crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 42,
            },
        )
        .expect("read completed generation binding")
        .expect("completed generation binding");
        assert_eq!(
            active_identity.generation_id,
            completed_identity.generation_id
        );
        assert_eq!(active_identity.binding_id, completed_identity.binding_id);
        assert_ne!(
            active_identity.ledger_head_hash,
            completed_identity.ledger_head_hash
        );
        let diagnosis = crate::cli::execution_state::diagnose(&fixture.repo, Some(session_id));
        assert_eq!(
            diagnosis.work_event_receipt_matches_current_generation,
            Some(true),
            "Completed diagnosis must accept the authentic Active lifecycle prefix",
        );

        crate::cli::action_obligation::mark_from_prompt(&fixture.repo, session_id, "進めて")
            .expect("record Completed handoff prompt");
        let events_path = fixture.repo.join(".gwt/work/events.jsonl");
        let events_before = std::fs::read(&events_path).expect("read Work events before PR");
        env.seed_created_pr(seeded_pr());

        let mut out = String::new();
        assert_eq!(
            run(
                &mut env,
                PrCommand::CreateBody {
                    base: s("develop"),
                    head: None,
                    title: s("completed settled handoff"),
                    body: s("body"),
                    labels: vec![],
                    draft: false,
                },
                &mut out,
            )
            .expect("create Ready PR from settled Completed generation"),
            0,
            "{out}",
        );

        assert_eq!(env.pr_create_call_log.len(), 1);
        assert!(crate::cli::action_obligation::open_kinds(&fixture.repo, session_id).is_empty());
        assert_eq!(
            std::fs::read(&events_path).expect("read Work events after PR"),
            events_before,
            "post-completion handoff must not append a terminal Work event",
        );
        assert_eq!(
            crate::cli::execution_state::current_execution_binding(
                &fixture.repo,
                crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    number: 42,
                },
            )
            .expect("read binding after PR")
            .expect("binding after PR"),
            completed_identity,
            "PR handoff must not create or rewrite an execution generation",
        );
    }

    #[test]
    fn pushed_branch_pr_operations_do_not_require_a_work_event_receipt() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let session_id = "session-receiptless-pr";
        let identity = initialize_pr_generation_authority(&fixture.repo, session_id);
        persist_pr_generation_session(&fixture.repo, session_id, identity);
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);

        crate::cli::verification_record::save_plan(
            &fixture.repo,
            &crate::cli::verification_record::VerificationPlanRecord {
                session_id: session_id.to_string(),
                owner_number: Some(42),
                execution_binding: None,
                commands: vec!["git --version".to_string()],
                derived: false,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                generated_outputs: Vec::new(),
                created_at: chrono::Utc::now(),
                content_hash: String::new(),
            },
        )
        .expect("save verification plan");
        crate::cli::verification_record::run_verification(
            &fixture.repo,
            session_id,
            &["git --version".to_string()],
        )
        .expect("record fresh verification");

        assert!(
            crate::cli::verification_record::load_work_event_settlement_record(&fixture.repo)
                .expect("load absent receipt")
                .is_none(),
            "the fixture must prove receiptless PR delivery"
        );

        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        env.seed_created_pr(seeded_pr());
        env.seed_pr(7, seeded_pr());

        for command in [
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("receiptless create"),
                body: s("body"),
                labels: vec![],
                draft: false,
            },
            PrCommand::EditBody {
                number: 7,
                title: Some(s("receiptless edit")),
                body: None,
                add_labels: vec![],
            },
            PrCommand::Ready { number: 7 },
        ] {
            let mut out = String::new();
            let code = run(&mut env, command, &mut out).expect("run receiptless PR operation");
            assert_eq!(code, 0, "{out}");
        }

        crate::cli::action_obligation::mark_from_prompt(
            &fixture.repo,
            session_id,
            "PR #7 にコメントを追加して",
        )
        .expect("arm PR comment obligation");
        assert_eq!(
            crate::cli::action_obligation::open_kinds(&fixture.repo, session_id),
            vec![crate::cli::action_obligation::ObligationKind::Pr]
        );
        let mut out = String::new();
        let code = run(
            &mut env,
            PrCommand::CommentBody {
                number: 7,
                body: s("receiptless comment"),
            },
            &mut out,
        )
        .expect("run receiptless PR comment");
        assert_eq!(code, 0, "{out}");
        assert!(
            crate::cli::action_obligation::open_kinds(&fixture.repo, session_id).is_empty(),
            "a successful pr.comment must settle the PR obligation"
        );

        assert_eq!(env.pr_create_call_log.len(), 1);
        assert_eq!(env.pr_edit_call_log.len(), 1);
        assert_eq!(env.pr_ready_call_log, vec![7]);
        assert_eq!(env.pr_comments.len(), 1);
    }

    #[test]
    fn receiptless_pr_mutation_still_requires_head_on_the_configured_upstream() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let session_id = "session-unpushed-pr";
        let identity = initialize_pr_generation_authority(&fixture.repo, session_id);
        persist_pr_generation_session(&fixture.repo, session_id, identity);
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        fixture.append_event("receiptless-unpushed-head");
        fixture.stage_events();
        fixture.commit("chore(work): leave HEAD ahead of upstream");

        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        env.seed_pr(7, seeded_pr());
        let mut out = String::new();
        let code = run(
            &mut env,
            PrCommand::EditBody {
                number: 7,
                title: Some(s("must remain pushed")),
                body: None,
                add_labels: vec![],
            },
            &mut out,
        )
        .expect("run unpushed PR mutation");

        assert_eq!(code, 2, "{out}");
        assert!(out.contains("current HEAD is not contained"), "{out}");
        assert!(env.pr_edit_call_log.is_empty());
    }

    #[test]
    fn pr_mutation_ignores_missing_unsettled_or_unauthorized_receipt() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let session_id = "session-receipt-pretransport";
        let active_identity = initialize_pr_generation_authority(&fixture.repo, session_id);
        persist_pr_generation_session(&fixture.repo, session_id, active_identity.clone());
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        env.seed_created_pr(seeded_pr());

        let assert_dispatched_without_receipt_gate = |env: &mut crate::cli::TestEnv, case: &str| {
            let previous_dispatches = env.pr_create_call_log.len();
            let mut out = String::new();
            let code = run(
                env,
                PrCommand::CreateBody {
                    base: s("develop"),
                    head: None,
                    title: format!("receipt case: {case}"),
                    body: s("body"),
                    labels: vec![],
                    draft: true,
                },
                &mut out,
            )
            .expect("run receipt-independent PR mutation");
            assert_eq!(code, 0, "{case}: {out}");
            assert_eq!(
                env.pr_create_call_log.len(),
                previous_dispatches + 1,
                "{case}: receipt state must not gate external PR dispatch",
            );
        };

        let missing_diagnosis = crate::cli::execution_state::diagnose(&fixture.repo, None);
        assert_eq!(missing_diagnosis.work_event_receipt_generation_id, None);
        assert_eq!(
            missing_diagnosis.work_event_receipt_matches_current_generation, None,
            "status must represent the absence of a receipt without claiming authority",
        );
        assert!(
            crate::cli::verification_record::work_event_settlement_refusal(&fixture.repo).is_some(),
            "the mutation gate must refuse a missing receipt when the Work log exists",
        );
        assert_dispatched_without_receipt_gate(&mut env, "missing");

        let valid = crate::cli::verification_record::save_work_event_settlement_record(
            &fixture.repo,
            session_id,
            false,
        )
        .expect("persist valid generation-scoped receipt");
        assert!(valid.status.is_settled());
        let mut cases = Vec::new();

        let mut foreign = valid.clone();
        foreign.session_id = "session-receipt-foreign".to_string();
        cases.push(("foreign", foreign));

        let mut different_generation = valid.clone();
        different_generation
            .execution_binding
            .as_mut()
            .expect("valid receipt is generation-bound")
            .generation_id = "generation-foreign".to_string();
        cases.push(("different-generation", different_generation));

        let mut arbitrary_head = valid.clone();
        arbitrary_head
            .execution_binding
            .as_mut()
            .expect("valid receipt is generation-bound")
            .ledger_head_hash = "arbitrary-ledger-head".to_string();
        cases.push(("arbitrary-head", arbitrary_head));

        let mut unbound = valid.clone();
        unbound.execution_binding = None;
        cases.push(("unbound", unbound));

        for (case, receipt) in cases {
            crate::cli::verification_record::persist_work_event_settlement_record(
                &fixture.repo,
                &receipt,
            )
            .expect("persist unauthorized receipt fixture");
            assert_dispatched_without_receipt_gate(&mut env, case);
        }

        crate::cli::verification_record::persist_work_event_settlement_record(
            &fixture.repo,
            &valid,
        )
        .expect("restore valid receipt before unsettled case");
        assert!(matches!(
            crate::cli::execution_state::settle(
                &fixture.repo,
                session_id,
                crate::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("complete the receipt-owning generation"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        assert_eq!(
            crate::cli::execution_state::diagnose(&fixture.repo, Some(session_id))
                .work_event_receipt_matches_current_generation,
            Some(true),
            "the unsettled case must start from an authentic Completed lifecycle prefix",
        );
        fixture.append_event("terminal-update-not-delivered");
        assert_dispatched_without_receipt_gate(&mut env, "unsettled");
        let unsettled_diagnosis = crate::cli::execution_state::diagnose(&fixture.repo, None);
        assert_eq!(
            unsettled_diagnosis.work_event_receipt_matches_current_generation,
            Some(true),
            "status must preserve the authentic lifecycle-prefix receipt as audit state",
        );
        assert_eq!(
            unsettled_diagnosis.settlement_severity, "clear",
            "receipt-independent PR dispatch must not refresh or mutate Work bookkeeping",
        );
    }

    #[test]
    fn pr_mutation_ignores_predecessor_receipt() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let predecessor_session = "session-receipt-predecessor";
        initialize_pr_generation_authority(&fixture.repo, predecessor_session);
        let receipt = crate::cli::verification_record::save_work_event_settlement_record(
            &fixture.repo,
            predecessor_session,
            false,
        )
        .expect("persist predecessor Work receipt");
        assert!(receipt.status.is_settled());
        assert!(matches!(
            crate::cli::execution_state::settle(
                &fixture.repo,
                predecessor_session,
                crate::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("complete predecessor generation"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));

        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 42,
        };
        let successor_session = "session-receipt-successor";
        let request = crate::cli::execution_state::SuccessorRequest {
            operation_id: "operation-receipt-successor".to_string(),
            principal_id: "principal-receipt-successor".to_string(),
            work_id: Some("work-receipt-successor".to_string()),
            source: "continue-work".to_string(),
            session_binding_id: "binding-receipt-successor".to_string(),
            initial_session_id: successor_session.to_string(),
            entrypoint: "continue-work".to_string(),
            requested_at: chrono::Utc::now(),
        };
        crate::cli::execution_state::prepare_successor(&fixture.repo, owner, &request)
            .expect("prepare successor generation");
        crate::cli::execution_state::activate_successor(&fixture.repo, owner, &request)
            .expect("activate successor generation");
        let successor_identity =
            crate::cli::execution_state::current_execution_binding(&fixture.repo, owner)
                .expect("read successor generation binding")
                .expect("successor generation binding exists");
        persist_pr_generation_session(&fixture.repo, successor_session, successor_identity);
        let _session = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            successor_session,
        );
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        env.seed_created_pr(seeded_pr());

        let mut out = String::new();
        assert_eq!(
            run(
                &mut env,
                PrCommand::CreateBody {
                    base: s("develop"),
                    head: None,
                    title: s("predecessor receipt"),
                    body: s("body"),
                    labels: vec![],
                    draft: true,
                },
                &mut out,
            )
            .expect("run receipt-independent PR mutation"),
            0,
            "{out}",
        );
        assert!(
            !env.pr_create_call_log.is_empty(),
            "a predecessor receipt must not block external PR dispatch",
        );
    }

    #[test]
    fn older_active_generation_can_verify_and_mutate_pr_through_its_durable_binding() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let older_session = "session-older-active-pr";
        let older_identity = initialize_pr_generation_authority(&fixture.repo, older_session);
        persist_pr_generation_session(&fixture.repo, older_session, older_identity);

        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 42,
        };
        let successor_session = "session-newer-active-pr";
        let request = crate::cli::execution_state::SuccessorRequest {
            operation_id: "operation-newer-active-pr".to_string(),
            principal_id: "gwt-host-launch".to_string(),
            work_id: None,
            source: crate::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
            session_binding_id: "binding-newer-active-pr".to_string(),
            initial_session_id: successor_session.to_string(),
            entrypoint: "launch".to_string(),
            requested_at: chrono::Utc::now(),
        };
        crate::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
            &fixture.repo,
            owner,
            &request,
        )
        .expect("prepare newer independent generation");
        crate::cli::execution_state::activate_successor(&fixture.repo, owner, &request)
            .expect("activate newer independent generation");
        let successor_identity =
            crate::cli::execution_state::current_execution_binding(&fixture.repo, owner)
                .expect("read newer generation binding")
                .expect("newer generation binding exists");
        persist_pr_generation_session(&fixture.repo, successor_session, successor_identity);

        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, older_session);
        let mut verify_env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut verify_out = String::new();
        assert_eq!(
            crate::cli::verification_record::run(
                &mut verify_env,
                crate::cli::verification_record::VerifyCommand::Plan {
                    commands: vec!["git --version".to_string()],
                    derive: false,
                },
                &mut verify_out,
            )
            .expect("register older-generation verification plan"),
            0,
            "{verify_out}",
        );
        verify_out.clear();
        assert_eq!(
            crate::cli::verification_record::run(
                &mut verify_env,
                crate::cli::verification_record::VerifyCommand::Run {
                    commands: vec!["git --version".to_string()],
                },
                &mut verify_out,
            )
            .expect("run older-generation verification"),
            0,
            "{verify_out}",
        );

        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        env.seed_created_pr(seeded_pr());
        let mut out = String::new();
        assert_eq!(
            run(
                &mut env,
                PrCommand::CreateBody {
                    base: s("develop"),
                    head: None,
                    title: s("older active generation"),
                    body: s("body"),
                    labels: vec![],
                    draft: false,
                },
                &mut out,
            )
            .expect("run older active generation PR handoff"),
            0,
            "{out}",
        );
        assert_eq!(env.pr_create_call_log.len(), 1);

        assert!(matches!(
            crate::cli::execution_state::settle(
                &fixture.repo,
                older_session,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "older generation blocker".to_string(),
                    missing_verification: None,
                },
            )
            .expect("settle the exact older generation"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let mut blocked_out = String::new();
        assert_eq!(
            run(
                &mut env,
                PrCommand::EditBody {
                    number: 7,
                    title: Some(s("must not dispatch")),
                    body: None,
                    add_labels: vec![],
                },
                &mut blocked_out,
            )
            .expect("evaluate older blocked generation PR policy"),
            2,
            "{blocked_out}",
        );
        assert!(blocked_out.contains("terminally blocked"), "{blocked_out}");
        assert!(
            env.pr_edit_call_log.is_empty(),
            "the exact older Blocked generation must refuse before dispatch",
        );
    }

    #[test]
    fn pr_mutation_refuses_capability_rotation_before_external_dispatch() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let worktree = tempfile::tempdir().expect("PR authority repository");
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let identity = initialize_pr_generation_authority(worktree.path(), "session-current");
        persist_pr_generation_session(worktree.path(), "session-current", identity);
        let _session = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "session-current",
        );
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        crate::cli::trusted_store::set_write_lease_acquired_hook(move || {
            gwt_agent::rotate_session_execution_capability(&sessions_dir, "session-current")
                .expect("rotate capability after the PR gate snapshot");
        });
        let mut env = crate::cli::TestEnv::new(worktree.path().to_path_buf());
        env.seed_created_pr(seeded_pr());
        let mut out = String::new();

        let error = run(
            &mut env,
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("draft"),
                body: s("body"),
                labels: vec![],
                draft: true,
            },
            &mut out,
        )
        .expect_err("a rotated capability must refuse dispatch");

        assert!(error
            .to_string()
            .contains("changed before external dispatch"));
        assert!(
            env.pr_create_call_log.is_empty(),
            "authority loss must be detected before the GitHub mutation",
        );
    }

    #[test]
    fn pr_family_parse_directly_handles_current() {
        let cmd = parse(&[s("current")]).expect("parse pr family command");
        assert_eq!(cmd, PrCommand::Current);
    }

    // SPEC-3248 P8b (T-112/AS-33): Ready handoffs are gated on the current
    // session's execution state and verification evidence; the Draft flow
    // stays available.
    #[test]
    fn pr_handoff_gate_blocks_ready_paths_until_evidence_or_settlement() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-pr");
        let tmp = tempfile::tempdir().expect("tempdir");
        crate::cli::execution_state::materialize_at_launch(
            tmp.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            42,
            "sess-pr",
            "launch",
            false,
        )
        .unwrap();

        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_pr(7, seeded_pr());
        env.seed_created_pr(seeded_pr());

        // Active execution without evidence: non-draft create and Ready refuse.
        let mut out = String::new();
        let code = run(
            &mut env,
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("t"),
                body: s("b"),
                labels: vec![],
                draft: false,
            },
            &mut out,
        )
        .expect("run pr create");
        assert_eq!(code, 2, "{out}");
        assert!(out.contains("PR handoff refused"), "{out}");
        assert!(
            env.pr_create_call_log.is_empty(),
            "create must not reach gh"
        );

        let mut out = String::new();
        let code = run(&mut env, PrCommand::Ready { number: 7 }, &mut out).expect("run pr ready");
        assert_eq!(code, 2, "{out}");
        assert!(env.pr_ready_call_log.is_empty(), "ready must not reach gh");

        // Draft creation stays available mid-work.
        let mut out = String::new();
        let code = run(
            &mut env,
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("t"),
                body: s("b"),
                labels: vec![],
                draft: true,
            },
            &mut out,
        )
        .expect("run pr create draft");
        assert_eq!(code, 0, "{out}");
        assert_eq!(env.pr_create_call_log.len(), 1);

        // Fresh evidence (plan + covering run) unlocks the Ready handoff.
        crate::cli::verification_record::save_plan(
            tmp.path(),
            &crate::cli::verification_record::VerificationPlanRecord {
                session_id: "sess-pr".to_string(),
                owner_number: Some(42),
                execution_binding: None,
                commands: vec!["git --version".to_string()],
                derived: false,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                generated_outputs: Vec::new(),
                created_at: chrono::Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        crate::cli::verification_record::run_verification(
            tmp.path(),
            "sess-pr",
            &["git --version".to_string()],
        )
        .unwrap();
        let mut out = String::new();
        let code = run(&mut env, PrCommand::Ready { number: 7 }, &mut out).expect("run pr ready");
        assert_eq!(code, 0, "{out}");
        assert_eq!(env.pr_ready_call_log, vec![7]);

        // A terminally blocked execution refuses entirely, even with evidence.
        crate::cli::execution_state::materialize_at_launch(
            tmp.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            42,
            "sess-pr",
            "launch",
            false,
        )
        .unwrap();
        crate::cli::execution_state::settle(
            tmp.path(),
            "sess-pr",
            crate::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "environment blocker".to_string(),
                missing_verification: None,
            },
        )
        .unwrap();
        let mut out = String::new();
        let code = run(&mut env, PrCommand::Ready { number: 7 }, &mut out).expect("run pr ready");
        assert_eq!(code, 2, "{out}");
        assert!(out.contains("terminally blocked"), "{out}");

        // AS-33: a blocked execution refuses every PR mutation — draft
        // creation and edits included.
        let mut out = String::new();
        let code = run(
            &mut env,
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("t"),
                body: s("b"),
                labels: vec![],
                draft: true,
            },
            &mut out,
        )
        .expect("run pr create draft while blocked");
        assert_eq!(code, 2, "{out}");
        let mut out = String::new();
        let code = run(
            &mut env,
            PrCommand::EditBody {
                number: 7,
                title: Some(s("t2")),
                body: None,
                add_labels: vec![],
            },
            &mut out,
        )
        .expect("run pr edit while blocked");
        assert_eq!(code, 2, "{out}");
        assert!(env.pr_edit_call_log.is_empty(), "edit must not reach gh");
    }

    #[test]
    fn work_event_settlement_gate_blocks_pr_mutations_but_keeps_recovery_surfaces() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "sess-pr-settlement");
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        crate::cli::execution_state::materialize_at_launch(
            &fixture.repo,
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            42,
            "sess-pr-settlement",
            "launch",
            false,
        )
        .unwrap();
        crate::cli::verification_record::save_plan(
            &fixture.repo,
            &crate::cli::verification_record::VerificationPlanRecord {
                session_id: "sess-pr-settlement".to_string(),
                owner_number: Some(42),
                execution_binding: None,
                commands: vec!["git --version".to_string()],
                derived: false,
                worktree_fingerprint: String::new(),
                surfaces: Vec::new(),
                generated_outputs: Vec::new(),
                created_at: chrono::Utc::now(),
                content_hash: String::new(),
            },
        )
        .unwrap();
        crate::cli::verification_record::run_verification(
            &fixture.repo,
            "sess-pr-settlement",
            &["git --version".to_string()],
        )
        .unwrap();
        fixture.append_event("terminal-update-awaiting-delivery");

        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        env.seed_pr(7, seeded_pr());
        env.seed_created_pr(seeded_pr());
        let blocked_commands = [
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("ready"),
                body: s("body"),
                labels: vec![],
                draft: false,
            },
            PrCommand::CreateBody {
                base: s("develop"),
                head: None,
                title: s("draft"),
                body: s("body"),
                labels: vec![],
                draft: true,
            },
            PrCommand::EditBody {
                number: 7,
                title: Some(s("updated")),
                body: Some(s("updated body")),
                add_labels: vec![],
            },
            PrCommand::Ready { number: 7 },
        ];
        for command in blocked_commands {
            let mut out = String::new();
            let code = run(&mut env, command, &mut out).expect("run PR settlement gate");
            assert_eq!(code, 2, "{out}");
            assert!(out.contains(".gwt/work/events.jsonl"), "{out}");
            assert!(out.contains("commit"), "{out}");
            assert!(out.contains("push"), "{out}");
        }
        assert!(
            env.pr_create_call_log.is_empty(),
            "create must not reach gh"
        );
        assert!(env.pr_edit_call_log.is_empty(), "edit must not reach gh");
        assert!(env.pr_ready_call_log.is_empty(), "ready must not reach gh");

        let mut out = String::new();
        let code = run(&mut env, PrCommand::Draft { number: 7 }, &mut out)
            .expect("run PR draft recovery surface");
        assert_eq!(code, 0, "{out}");
        assert_eq!(env.pr_draft_call_log, vec![7]);

        let mut out = String::new();
        let code = run(
            &mut env,
            PrCommand::CommentBody {
                number: 7,
                body: s("Work delivery is blocked; keeping the PR in Draft."),
            },
            &mut out,
        )
        .expect("run PR blocker comment recovery surface");
        assert_eq!(code, 0, "{out}");
        assert_eq!(env.pr_comments.len(), 1);
    }

    #[test]
    fn pr_family_run_directly_renders_current_pr() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_current_pr(Some(seeded_pr()));

        let mut out = String::new();
        let code = run(&mut env, PrCommand::Current, &mut out).expect("run pr family");

        assert_eq!(code, 0);
        assert!(out.contains("#7 [OPEN] CLI family split"));
        assert_eq!(env.pr_current_call_count, 1);
        assert!(env.client.call_log().is_empty());
    }

    #[test]
    fn pr_family_ready_and_draft_dispatch_through_env() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_pr(7, seeded_pr());

        let mut out = String::new();
        let code = run(&mut env, PrCommand::Ready { number: 7 }, &mut out).expect("run pr ready");
        assert_eq!(code, 0);
        assert!(
            out.contains("marked pull request #7 ready for review"),
            "unexpected ready output: {out}"
        );
        assert!(out.contains("#7 [OPEN] CLI family split"));
        assert_eq!(env.pr_ready_call_log, vec![7]);

        let mut out = String::new();
        let code = run(&mut env, PrCommand::Draft { number: 7 }, &mut out).expect("run pr draft");
        assert_eq!(code, 0);
        assert!(
            out.contains("converted pull request #7 to draft"),
            "unexpected draft output: {out}"
        );
        assert_eq!(env.pr_draft_call_log, vec![7]);
    }

    #[test]
    fn pr_family_current_persists_workspace_pr_metadata() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut env = crate::cli::TestEnv::new(home.path().join("cache"));
        env.repo_path = repo.clone();
        env.seed_current_pr(Some(gwt_git::PrStatus {
            number: 2538,
            title: "Active Work title".to_string(),
            state: gwt_git::pr_status::PrState::Open,
            url: "https://github.com/akiojin/gwt/pull/2538".to_string(),
            created_at: Some("2026-05-07T08:20:00Z".parse().expect("created_at")),
            ci_status: "PENDING".to_string(),
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: "UNKNOWN".to_string(),
            review_status: "REVIEW_REQUIRED".to_string(),
        }));
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/20260507-0808".to_string()),
            worktree_path: Some(repo.join("work/20260507-0808")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let mut out = String::new();
        let code = run(&mut env, PrCommand::Current, &mut out).expect("run pr current");

        assert_eq!(code, 0);
        assert!(out.contains("#2538 [OPEN] Active Work title"));
        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let details = projection.git_details.expect("git details");
        assert_eq!(details.branch.as_deref(), Some("work/20260507-0808"));
        assert_eq!(details.base_branch.as_deref(), Some("origin/develop"));
        assert_eq!(details.pr_number, Some(2538));
        assert_eq!(details.pr_state.as_deref(), Some("OPEN"));
        assert_eq!(
            details.pr_url.as_deref(),
            Some("https://github.com/akiojin/gwt/pull/2538")
        );
        assert_eq!(
            details.pr_created_at.expect("pr_created_at").to_rfc3339(),
            "2026-05-07T08:20:00+00:00"
        );
    }

    #[test]
    fn pr_family_create_skips_workspace_pr_metadata_for_non_current_head() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut env = crate::cli::TestEnv::new(home.path().join("cache"));
        env.repo_path = repo.clone();
        env.files.insert("body.md".to_string(), "Body".to_string());
        env.seed_created_pr(gwt_git::PrStatus {
            number: 2540,
            title: "Other branch PR".to_string(),
            state: gwt_git::pr_status::PrState::Open,
            url: "https://github.com/akiojin/gwt/pull/2540".to_string(),
            created_at: Some("2026-05-07T08:30:00Z".parse().expect("created_at")),
            ci_status: "PENDING".to_string(),
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: "UNKNOWN".to_string(),
            review_status: "REVIEW_REQUIRED".to_string(),
        });
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/20260507-0808".to_string()),
            worktree_path: Some(repo.join("work/20260507-0808")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let mut out = String::new();
        let code = run(
            &mut env,
            PrCommand::Create {
                base: "develop".to_string(),
                head: Some("feature/other".to_string()),
                title: "Other branch PR".to_string(),
                file: "body.md".to_string(),
                labels: vec![],
                draft: false,
            },
            &mut out,
        )
        .expect("run pr create");

        assert_eq!(code, 0);
        assert!(out.contains("#2540 [OPEN] Other branch PR"));
        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let details = projection.git_details.expect("git details");
        assert_eq!(details.branch.as_deref(), Some("work/20260507-0808"));
        assert_eq!(details.pr_number, None);
        assert_eq!(details.pr_state, None);
        assert_eq!(details.pr_url, None);
        assert_eq!(details.pr_created_at, None);
    }

    fn init_fake_current_branch_repo(repo_path: &std::path::Path) {
        let init = gwt_core::process::hidden_command("git")
            .args(["init", "-b", "main"])
            .current_dir(repo_path)
            .status()
            .expect("git init");
        assert!(init.success());
        let remote = gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/akiojin/gwt.git",
            ])
            .current_dir(repo_path)
            .status()
            .expect("git remote add");
        assert!(remote.success());
        let checkout = gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/20260507-0808"])
            .current_dir(repo_path)
            .status()
            .expect("git checkout");
        assert!(checkout.success());
    }

    #[test]
    fn fetch_current_pr_via_gh_chooses_newest_pr_for_current_branch() {
        with_fake_gh("multi-pr-current", |repo_path| {
            init_fake_current_branch_repo(repo_path);

            let pr = fetch_current_pr_via_gh(repo_path)
                .expect("fetch current pr")
                .expect("current branch pr");

            assert_eq!(pr.number, 2538);
            assert_eq!(pr.title, "Newer PR");
            assert_eq!(
                pr.created_at.expect("created_at").to_rfc3339(),
                "2026-05-07T08:20:00+00:00"
            );
        });
    }

    #[test]
    fn fetch_current_pr_via_gh_ignores_same_branch_prs_from_other_repositories() {
        with_fake_gh("multi-pr-cross-fork-current", |repo_path| {
            init_fake_current_branch_repo(repo_path);

            let pr = fetch_current_pr_via_gh(repo_path)
                .expect("fetch current pr")
                .expect("current branch pr");

            assert_eq!(pr.number, 2538);
            assert_eq!(pr.title, "Current repo PR");
            assert_eq!(
                pr.created_at.expect("created_at").to_rfc3339(),
                "2026-05-07T08:20:00+00:00"
            );
        });
    }

    // -------------------------------------------------------------------
    // SPEC-1942 SC-025 follow-up: PR-family helper tests relocated from
    // cli.rs. Shared fake-gh harness lives in cli/test_support.rs.
    // -------------------------------------------------------------------

    use crate::cli::test_support::{sample_thread, with_fake_gh, ScopedEnvVar};
    use crate::cli::PrCreateCall;
    use std::io;

    #[test]
    fn review_thread_reply_is_skipped_for_duplicate_body() {
        let mut thread = sample_thread();
        thread.comments.push(crate::cli::PrReviewThreadComment {
            id: "comment-1".to_string(),
            body: "Fixed in latest commit.".to_string(),
            created_at: "2026-04-10T00:00:00Z".to_string(),
            updated_at: "2026-04-10T00:00:00Z".to_string(),
            author: "reviewer".to_string(),
        });

        assert!(!should_reply_to_review_thread(
            &thread,
            "Fixed in latest commit."
        ));
        assert!(should_resolve_review_thread(&thread));
    }

    #[test]
    fn review_thread_reply_is_skipped_for_resolved_or_outdated_threads() {
        let mut resolved = sample_thread();
        resolved.is_resolved = true;
        assert!(!should_reply_to_review_thread(&resolved, "reply"));
        assert!(!should_resolve_review_thread(&resolved));

        let mut outdated = sample_thread();
        outdated.is_outdated = true;
        assert!(!should_reply_to_review_thread(&outdated, "reply"));
        assert!(should_resolve_review_thread(&outdated));
    }

    #[test]
    fn pr_checks_response_returns_error_when_gh_fails() {
        let err = parse_pr_checks_items_response("", "auth failed", false).unwrap_err();
        assert!(
            err.to_string().contains("gh pr checks: auth failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pr_checks_response_parses_success_payload() {
        let items = parse_pr_checks_items_response(
            r#"[{"name":"test","state":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://example.com","startedAt":"2026-04-10T00:00:00Z","completedAt":"2026-04-10T00:01:00Z","workflow":"CI"}]"#,
            "",
            true,
        )
        .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "test");
        assert_eq!(items[0].conclusion, "SUCCESS");
    }

    #[test]
    fn gh_wrappers_parse_successful_responses() {
        with_fake_gh("success", |repo_path| {
            let linked = crate::cli::issue::fetch_linked_prs_via_gh(
                "akiojin",
                "gwt",
                gwt_github::IssueNumber(42),
            )
            .expect("linked");
            assert_eq!(linked.len(), 2);
            assert_eq!(linked[0].number, 12);
            assert_eq!(linked[1].state, "MERGED");

            let current = fetch_current_pr_via_gh(repo_path)
                .expect("current pr")
                .expect("current pr exists");
            assert_eq!(current.number, 12);
            assert_eq!(current.merge_state_status, "CLEAN");

            let created = create_pr_via_gh(
                "akiojin/gwt",
                repo_path,
                &PrCreateCall {
                    base: "develop".to_string(),
                    head: Some("feature/coverage".to_string()),
                    title: "Raise coverage".to_string(),
                    body: "Body".to_string(),
                    labels: vec!["coverage".to_string()],
                    draft: true,
                },
            )
            .expect("create pr");
            assert_eq!(created.number, 12);

            let edited = edit_pr_via_gh(
                "akiojin/gwt",
                repo_path,
                12,
                Some("Edited"),
                Some("Updated body"),
                &["tested".to_string()],
            )
            .expect("edit pr");
            assert_eq!(edited.number, 12);

            // Unknown labels must fail closed before any mutation: the REST
            // labels endpoint would otherwise silently auto-create the label,
            // unlike the old `gh pr edit --add-label` which rejected typos.
            let missing_label = edit_pr_via_gh(
                "akiojin/gwt",
                repo_path,
                12,
                Some("Edited"),
                None,
                &["no-such-label".to_string()],
            )
            .expect_err("unknown label must fail closed");
            assert!(
                missing_label.to_string().contains("no-such-label"),
                "unexpected error: {missing_label}"
            );

            let readied =
                mark_pr_ready_via_gh("akiojin/gwt", repo_path, 12).expect("mark pr ready");
            assert_eq!(readied.number, 12);

            let drafted =
                convert_pr_to_draft_via_gh("akiojin/gwt", repo_path, 12).expect("convert pr draft");
            assert_eq!(drafted.number, 12);

            comment_on_pr_via_gh(repo_path, 12, "done").expect("comment");

            let reviews = fetch_pr_reviews_via_gh("akiojin", "gwt", 12).expect("reviews");
            assert_eq!(reviews.len(), 1);
            assert_eq!(reviews[0].author, "reviewer");

            let threads =
                fetch_pr_review_threads_via_gh("akiojin", "gwt", 12).expect("review threads");
            assert_eq!(threads.len(), 2);
            assert_eq!(threads[0].line, Some(10));

            let resolved = reply_and_resolve_pr_review_threads_via_gh("akiojin", "gwt", 12, "done")
                .expect("reply and resolve");
            assert_eq!(resolved, 2);

            let checks = fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12).expect("checks");
            assert!(checks.summary.contains("PR #12"));
            assert_eq!(checks.checks.len(), 1);
            assert_eq!(checks.checks[0].conclusion, "SUCCESS");

            let run_log =
                crate::cli::actions::fetch_actions_run_log_via_gh(repo_path, 90).expect("run log");
            assert_eq!(run_log.trim(), "run log 90");

            let job_log =
                crate::cli::actions::fetch_actions_job_log_via_gh("akiojin", "gwt", repo_path, 91)
                    .expect("job log");
            assert_eq!(job_log, "job log 91");
        });
    }

    #[test]
    fn gh_wrappers_cover_none_fallback_and_zip_error_paths() {
        with_fake_gh("no-current-pr", |repo_path| {
            assert!(fetch_current_pr_via_gh(repo_path)
                .expect("current pr result")
                .is_none());
        });

        with_fake_gh("checks-fallback", |repo_path| {
            let checks = fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12).expect("checks");
            assert_eq!(checks.checks.len(), 1);
            assert_eq!(checks.checks[0].workflow, "coverage");
            assert_eq!(checks.checks[0].url, "https://example.test/checks/12");
        });

        with_fake_gh("behind", |repo_path| {
            let current = fetch_current_pr_via_gh(repo_path)
                .expect("current pr")
                .expect("current pr exists");
            assert_eq!(current.effective_merge_status(), "BEHIND");

            let checks = fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12).expect("checks");
            assert!(checks.summary.contains("Merge: BEHIND"));
            assert_eq!(checks.merge_status, "BEHIND");
        });

        with_fake_gh("job-log-zip", |repo_path| {
            let err =
                crate::cli::actions::fetch_actions_job_log_via_gh("akiojin", "gwt", repo_path, 91)
                    .expect_err("zip");
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert!(err.to_string().contains("zip archive"));
        });
    }

    #[test]
    fn gh_wrappers_tolerate_resolve_failure_after_remote_state_changes() {
        with_fake_gh("resolve-fails-but-resolved", |_repo_path| {
            let resolved = reply_and_resolve_pr_review_threads_via_gh("akiojin", "gwt", 12, "done")
                .expect("resolved after retry");
            assert_eq!(resolved, 2);
        });
    }

    // SPEC-2359 US-37 / T-240: PR state polling auto-done on merged

    #[test]
    fn pr_family_current_emits_workspace_auto_done_when_pr_is_merged() {
        use crate::cli::test_support::ScopedEnvVar;
        use chrono::TimeZone;

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let mut env = crate::cli::TestEnv::new(home.path().join("cache"));
        env.repo_path = repo.clone();
        env.seed_current_pr(Some(gwt_git::PrStatus {
            number: 9999,
            title: "Auto-done PR".to_string(),
            state: gwt_git::pr_status::PrState::Merged,
            url: "https://github.com/akiojin/gwt/pull/9999".to_string(),
            created_at: None,
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            review_status: "APPROVED".to_string(),
        }));

        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.id = "wi-pr-merge-auto-done".to_string();
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/20260513-0500".to_string()),
            worktree_path: Some(repo.join("work/20260513-0500")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let mut start = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            "wi-pr-merge-auto-done",
            chrono::Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap(),
        );
        start.title = Some("Auto-done PR work".to_string());
        start.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
        gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
            .expect("seed start event");

        let mut out = String::new();
        let code = run(&mut env, PrCommand::Current, &mut out).expect("run pr current");
        assert_eq!(code, 0);

        let projection_after = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        assert_eq!(
            projection_after
                .git_details
                .as_ref()
                .expect("git details")
                .pr_state
                .as_deref(),
            Some("MERGED")
        );

        let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load work items")
            .expect("work items");
        let item = work_items
            .work_items
            .iter()
            .find(|item| item.id == "wi-pr-merge-auto-done")
            .expect("work item");
        assert_eq!(
            item.status_category,
            gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
            "PR merge must auto-emit Done for the linked Workspace WorkItem",
        );
    }
}
