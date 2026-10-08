//! Issue #4624: inspect a reused Issue checkout before launching its next agent.

use std::path::Path;

use gwt_core::coordination::{
    AuthorKind, BoardEntryDraft, BoardEntryKind, BoardMention, BoardMentionTargetKind,
};

/// Called under the branch materialization lock, including preset working dirs.
/// A freshness failure is diagnostic, never a reason to reset/stash or to hide
/// an otherwise usable checkout from its agent.
pub(super) fn refresh_and_record(repo: &Path, branch: &str, worktree: &Path) {
    let body = match inspect_and_refresh(repo, branch, worktree) {
        Ok(body) => body,
        Err(error) => format!(
            "起動時の base 確認: branch={branch} base=origin/develop\n確認失敗: {error}\n鮮度は保証できません。担当が確認してください。"
        ),
    };
    tracing::info!(branch, worktree = %worktree.display(), report = %body, "launch base freshness");
    let mut draft = BoardEntryDraft::new(AuthorKind::System, "gwt", BoardEntryKind::Status, body);
    draft.origin.branch = branch.to_string();
    draft.mentions = vec![BoardMention::new(BoardMentionTargetKind::Branch, branch)];
    if let Ok(entry) = draft.finalize() {
        if let Err(error) = gwt::board_provider::post_entry_outcome(repo, entry) {
            tracing::warn!(%error, branch, "launch base freshness Board delivery failed; see launch log");
        }
    }
}

fn git_output(worktree: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = gwt_core::process::run_git_logged(args, Some(worktree))
        .map_err(|error| format!("git {args:?}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn git_text(worktree: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(git_output(worktree, args)?)
        .map(|text| text.trim().to_string())
        .map_err(|error| error.to_string())
}

fn inspect_and_refresh(repo: &Path, branch: &str, worktree: &Path) -> Result<String, String> {
    // A caller-provided cwd is not proof that this is the requested checkout.
    let actual_repo =
        gwt_git::worktree::main_worktree_root(worktree).map_err(|error| error.to_string())?;
    if !super::same_worktree_path(repo, &actual_repo) {
        return Err(
            "preset working directory belongs to another repository; unchanged".to_string(),
        );
    }
    let root = git_text(worktree, &["rev-parse", "--show-toplevel"])?;
    let actual_branch = git_text(worktree, &["symbolic-ref", "--short", "HEAD"])?;
    if actual_branch != branch || !super::same_worktree_path(Path::new(&root), worktree) {
        return Err(
            "preset working directory does not match the requested worktree/branch; unchanged"
                .to_string(),
        );
    }
    let manager = gwt_git::WorktreeManager::new(repo);
    manager.fetch_origin().map_err(|error| error.to_string())?;
    let before = git_text(worktree, &["rev-parse", "HEAD"])?;
    let base = git_text(worktree, &["rev-parse", "origin/develop^{commit}"])?;
    let divergence =
        gwt_git::git_divergence(worktree, &before, &base).map_err(|error| error.to_string())?;
    let decision = if divergence.ahead > 0 {
        "unique_commits: 固有 commit を保全しました".to_string()
    } else if divergence.behind == 0 {
        "up_to_date: base からの遅れはありません".to_string()
    } else if has_agent_changes(worktree)? {
        "agent_changes: 担当の未コミット変更を保全しました".to_string()
    } else if super::live_session_holding_worktree(&gwt_core::paths::gwt_sessions_dir(), worktree)
        .is_some()
    {
        "live_session: 稼働中の担当の checkout を保全しました".to_string()
    } else {
        // Pin the observed SHA; another worktree may fetch while we hold this
        // branch's lock. merge updates HEAD, index and files together and
        // refuses collisions with untracked shards without removing them.
        // `merge` normally permits overwriting ignored files. An agent may
        // have put work there before the base began tracking that path.
        match git_output(
            worktree,
            &[
                "merge",
                "--ff-only",
                "--no-overwrite-ignore",
                "--quiet",
                &base,
            ],
        ) {
            Ok(_) => "fast-forward: 観測した base へ更新しました".to_string(),
            Err(error) => format!("fast_forward_failed: {error}"),
        }
    };
    let after = git_text(worktree, &["rev-parse", "HEAD"])?;
    Ok(format!(
        "起動時の base 確認: branch={branch} base=origin/develop ahead={} behind={}\nhead_before={before}\nhead_after={after}\nbase_sha={base}\n判断: {decision}",
        divergence.ahead, divergence.behind,
    ))
}

fn has_agent_changes(worktree: &Path) -> Result<bool, String> {
    let status = git_output(
        worktree,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    // Only untracked immutable event shards are gwt-owned exceptions. Never
    // exempt tracked edits/deletions or the whole .gwt namespace. With -z a
    // tracked rename already makes this true before its second path is read.
    Ok(status
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .any(|entry| !entry.strip_prefix(b"?? ").is_some_and(is_event_shard)))
}

fn is_event_shard(path: &[u8]) -> bool {
    let Some(relative) = path.strip_prefix(b".gwt/work/events/") else {
        return false;
    };
    let Some(separator) = relative.iter().position(|byte| *byte == b'/') else {
        return false;
    };
    let (bucket, rest) = relative.split_at(separator);
    let file = &rest[1..];
    let Some(digest) = file.strip_suffix(b".jsonl") else {
        return false;
    };
    bucket.len() == 2
        && digest.len() == 64
        && digest.starts_with(bucket)
        && digest
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
    };

    use crate::launch_runtime::{
        resolve_launch_worktree_request,
        tests::{init_launch_test_repo, run_git},
    };
    use gwt_core::test_support::ScopedGwtHome;

    const BRANCH: &str = "work/issue-4624";

    fn stale_worktree(root: &Path) -> (PathBuf, PathBuf) {
        let repo = init_launch_test_repo(root);
        let worktree = root.join("issue");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                BRANCH,
                worktree.to_str().unwrap(),
                "origin/develop",
            ],
        );
        fs::write(repo.join("README.md"), "new base\n").unwrap();
        run_git(&repo, &["commit", "-am", "advance base"]);
        run_git(&repo, &["push", "origin", "develop"]);
        (repo, worktree)
    }

    fn launch(repo: &Path, mut preset: Option<PathBuf>) {
        resolve_launch_worktree_request(
            repo,
            Some(BRANCH),
            &mut None,
            &mut preset,
            &mut HashMap::new(),
        )
        .unwrap();
    }

    fn report(worktree: &Path) -> String {
        let snapshot = gwt::board_provider::load_snapshot(worktree).unwrap();
        let entry = snapshot
            .board
            .entries
            .last()
            .expect("launch records base freshness even at zero lag");
        assert!(entry
            .mentions
            .iter()
            .any(|mention| mention.target == BRANCH));
        entry.body.clone()
    }

    fn head(worktree: &Path) -> String {
        let output = gwt_core::process::hidden_command("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(worktree)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn clean_ancestor_refreshes_head_index_and_files_and_records_zero_lag_on_next_launch() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, worktree) = stale_worktree(temp.path());
        let before = head(&worktree);
        launch(&repo, None);
        assert_eq!(
            head(&worktree),
            head(&repo),
            "clean ancestor must fast-forward before launch"
        );
        assert_eq!(
            fs::read_to_string(worktree.join("README.md")).unwrap(),
            "new base\n"
        );
        run_git(&worktree, &["diff", "--exit-code", "HEAD"]);
        let evidence = report(&worktree);
        assert!(evidence.contains("behind=1"), "{evidence}");
        assert!(
            evidence.contains(&before) && evidence.contains(&head(&repo)),
            "{evidence}"
        );
        assert!(evidence.contains("fast-forward"), "{evidence}");
        launch(&repo, None);
        assert!(report(&worktree).contains("behind=0"));
    }

    #[test]
    fn unique_commits_are_preserved_and_lag_is_delivered() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, worktree) = stale_worktree(temp.path());
        fs::write(worktree.join("agent.txt"), "keep\n").unwrap();
        run_git(&worktree, &["add", "agent.txt"]);
        run_git(&worktree, &["commit", "-m", "agent work"]);
        let before = head(&worktree);
        launch(&repo, None);
        assert_eq!(head(&worktree), before);
        let evidence = report(&worktree);
        assert!(
            evidence.contains("ahead=1") && evidence.contains("behind=1"),
            "{evidence}"
        );
        assert!(evidence.contains("unique_commits"), "{evidence}");
    }

    #[test]
    fn uncommitted_agent_work_is_preserved_even_when_merge_would_not_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, worktree) = stale_worktree(temp.path());
        fs::write(worktree.join("agent.txt"), "keep\n").unwrap();
        let before = head(&worktree);
        launch(&repo, None);
        assert_eq!(head(&worktree), before);
        assert_eq!(
            fs::read_to_string(worktree.join("agent.txt")).unwrap(),
            "keep\n"
        );
        let evidence = report(&worktree);
        assert!(
            evidence.contains("behind=1") && evidence.contains("agent_changes"),
            "{evidence}"
        );
    }

    #[test]
    fn gwt_event_shards_do_not_prevent_preset_worktree_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, worktree) = stale_worktree(temp.path());
        let shard = worktree.join(format!(".gwt/work/events/aa/{}.jsonl", "a".repeat(64)));
        fs::create_dir_all(shard.parent().unwrap()).unwrap();
        fs::write(&shard, "{}\n").unwrap();
        launch(&repo, Some(worktree.clone()));
        assert_eq!(head(&worktree), head(&repo));
        assert_eq!(fs::read_to_string(shard).unwrap(), "{}\n");
        assert!(report(&worktree).contains("fast-forward"));
    }

    #[test]
    fn preset_from_another_repository_with_the_same_branch_is_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let primary = temp.path().join("primary");
        let foreign = temp.path().join("foreign");
        fs::create_dir_all(&primary).unwrap();
        fs::create_dir_all(&foreign).unwrap();
        let (repo, _) = stale_worktree(&primary);
        let (_, other_worktree) = stale_worktree(&foreign);
        let before = head(&other_worktree);
        launch(&repo, Some(other_worktree.clone()));
        assert_eq!(
            head(&other_worktree),
            before,
            "preset cwd must belong to the requested repository"
        );
    }

    #[test]
    fn ignored_agent_file_is_not_overwritten_when_base_starts_tracking_it() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, worktree) = stale_worktree(temp.path());
        fs::write(repo.join(".git/info/exclude"), "agent.txt\n").unwrap();
        fs::write(worktree.join("agent.txt"), "uncommitted agent work\n").unwrap();
        fs::write(repo.join("agent.txt"), "base content\n").unwrap();
        run_git(&repo, &["add", "-f", "agent.txt"]);
        run_git(&repo, &["commit", "-m", "start tracking ignored path"]);
        run_git(&repo, &["push", "origin", "develop"]);
        let before = head(&worktree);
        launch(&repo, None);
        assert_eq!(
            fs::read_to_string(worktree.join("agent.txt")).unwrap(),
            "uncommitted agent work\n"
        );
        assert_eq!(head(&worktree), before);
        assert!(report(&worktree).contains("fast_forward_failed"));
    }

    #[test]
    fn tracked_and_staged_agent_changes_prevent_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, worktree) = stale_worktree(temp.path());
        let before = head(&worktree);
        fs::write(worktree.join("README.md"), "agent change\n").unwrap();
        launch(&repo, None);
        assert_eq!(head(&worktree), before);
        assert!(report(&worktree).contains("agent_changes"));
        run_git(&worktree, &["add", "README.md"]);
        launch(&repo, None);
        assert_eq!(head(&worktree), before);
        assert_eq!(
            fs::read_to_string(worktree.join("README.md")).unwrap(),
            "agent change\n"
        );
        assert!(report(&worktree).contains("agent_changes"));
    }

    #[test]
    fn non_shard_files_in_gwt_namespace_are_agent_changes() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, worktree) = stale_worktree(temp.path());
        let before = head(&worktree);
        fs::create_dir_all(worktree.join(".gwt/work/events")).unwrap();
        fs::write(worktree.join(".gwt/work/events/agent-note.txt"), "keep\n").unwrap();
        launch(&repo, None);
        assert_eq!(head(&worktree), before);
        assert!(report(&worktree).contains("agent_changes"));
    }

    #[test]
    fn live_holder_keeps_its_clean_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (repo, worktree) = stale_worktree(temp.path());
        let before = head(&worktree);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        fs::create_dir_all(&sessions).unwrap();
        let session = gwt_agent::Session::new(&worktree, BRANCH, gwt_agent::AgentId::ClaudeCode);
        session.save(&sessions).unwrap();
        let runtime =
            gwt_agent::runtime_state_path_for_pid(&sessions, std::process::id(), &session.id);
        fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
            .save(&runtime)
            .unwrap();
        launch(&repo, Some(worktree.clone()));
        assert_eq!(head(&worktree), before);
        assert!(report(&worktree).contains("live_session"));
    }
}
