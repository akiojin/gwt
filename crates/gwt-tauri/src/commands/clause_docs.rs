//! CLAUDE.md / AGENTS.md / GEMINI.md check/fix command

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use gwt_core::{
    config::{inject_managed_skills_block, write_atomic, Settings},
    git::Remote,
    worktree::WorktreeManager,
    StructuredError,
};
use serde::Serialize;
use tracing::instrument;

use crate::commands::project::resolve_repo_path_for_project_root;

const CLAUDE_MD_DEFAULT_CONTENT: &str = r#"# CLAUDE.md

このファイルは、このリポジトリで作業するエージェント向けの運用ガイドです。

## ワークフロー設計

### Planモード

- 実装前に、目的・影響範囲・検証方法を短く明文化してください。
- 大きな変更では、段階ごとの完了条件を先に定義してください。

### サブエージェント

- 独立して進められる作業は分割し、担当範囲を明確にして並列化してください。
- 最終的な統合担当は差分整合と最終検証を実施してください。

### 自己改善

- エラーや失敗の再発防止策を作業中に更新し、次の実行へ反映してください。

### 実行前チェック

- 既存実装・関連ドキュメントを確認してから変更してください。
- 仕様と実装がずれないよう、判断根拠を残してください。

### エレガンス

- 単純で保守しやすい実装を優先し、不要な複雑性を避けてください。

### 自律修正

- 問題検出時は、原因特定→修正→再検証まで一連で完了してください。

## タスク管理

### tasks/todo.md

- 実装タスク、進捗、検証結果をこのファイルで管理してください。

### tasks/lessons.md

- 失敗事例と再発防止策を記録し、次回着手前に確認してください。

## コア原則

- シンプルさを最優先にする。
- 手抜きの回避と検証の徹底を守る。
- 影響範囲を最小化し、既存動作を壊さない。
"#;

const CLAUDE_MD: &str = "CLAUDE.md";
const AGENTS_MD: &str = "AGENTS.md";
const GEMINI_MD: &str = "GEMINI.md";
const CLAUDE_REF: &str = "@CLAUDE.md";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionDocsCheckResult {
    pub worktree_path: String,
    pub checked_files: Vec<String>,
    pub updated_files: Vec<String>,
}

fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let Some((first, rest)) = branch.split_once('/') else {
        return branch;
    };
    if remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    branch
}

fn resolve_worktree_path_for_branch(repo_path: &Path, branch_ref: &str) -> Result<PathBuf, String> {
    let branch_ref = branch_ref.trim();
    if branch_ref.is_empty() {
        return Err("Branch is required".to_string());
    }

    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;
    let remotes = Remote::list(repo_path).unwrap_or_default();
    let normalized = strip_known_remote_prefix(branch_ref, &remotes);

    let mut worktree = manager
        .get_by_branch_basic(normalized)
        .map_err(|e| e.to_string())?;
    if worktree.is_none() && normalized != branch_ref {
        worktree = manager
            .get_by_branch_basic(branch_ref)
            .map_err(|e| e.to_string())?;
    }

    let Some(worktree) = worktree else {
        return Err(format!("Worktree not found for branch: {branch_ref}"));
    };

    if !worktree.is_active() || !worktree.path.exists() {
        return Err(format!("Worktree is not active for branch: {branch_ref}"));
    }

    Ok(worktree.path)
}

fn ensure_claude_md(path: &Path) -> Result<bool, String> {
    match std::fs::read_to_string(path) {
        Ok(content) if !content.trim().is_empty() => Ok(false),
        Ok(_) => {
            write_atomic(path, CLAUDE_MD_DEFAULT_CONTENT).map_err(|e| e.to_string())?;
            Ok(true)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            write_atomic(path, CLAUDE_MD_DEFAULT_CONTENT).map_err(|e| e.to_string())?;
            Ok(true)
        }
        Err(err) => Err(err.to_string()),
    }
}

fn ensure_claude_ref_file(path: &Path) -> Result<bool, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            if content.contains(CLAUDE_REF) {
                Ok(false)
            } else {
                let mut out = String::from(CLAUDE_REF);
                out.push('\n');
                out.push_str(&content);
                write_atomic(path, &out).map_err(|e| e.to_string())?;
                Ok(true)
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            write_atomic(path, &format!("{CLAUDE_REF}\n")).map_err(|e| e.to_string())?;
            Ok(true)
        }
        Err(err) => Err(err.to_string()),
    }
}

/// Inject or update the managed skills block in a markdown file.
///
/// Returns `true` if the file was modified, `false` if unchanged or missing.
pub(crate) fn ensure_managed_skills_block(path: &Path) -> Result<bool, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.to_string()),
    };

    let updated = inject_managed_skills_block(&content)?;
    if updated == content {
        return Ok(false);
    }
    write_atomic(path, &updated).map_err(|e| e.to_string())?;
    Ok(true)
}

#[instrument(
    skip_all,
    fields(command = "check_and_fix_agent_instruction_docs", project_path, branch)
)]
#[tauri::command]
pub fn check_and_fix_agent_instruction_docs(
    project_path: String,
    branch: String,
) -> Result<InstructionDocsCheckResult, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "check_and_fix_agent_instruction_docs"))?;
    let worktree_path = resolve_worktree_path_for_branch(&repo_path, &branch)
        .map_err(|e| StructuredError::internal(&e, "check_and_fix_agent_instruction_docs"))?;

    let checked_files = vec![
        CLAUDE_MD.to_string(),
        AGENTS_MD.to_string(),
        GEMINI_MD.to_string(),
    ];
    let mut updated_files = Vec::new();

    let claude_path = worktree_path.join(CLAUDE_MD);
    if ensure_claude_md(&claude_path)
        .map_err(|e| StructuredError::internal(&e, "check_and_fix_agent_instruction_docs"))?
    {
        updated_files.push(CLAUDE_MD.to_string());
    }

    // Inject managed skills block based on settings
    let settings = Settings::load_global().ok();
    let skill_prefs = settings
        .as_ref()
        .and_then(|s| s.agent.skill_registration.as_ref());

    if skill_prefs.map(|p| p.inject_claude_md).unwrap_or(true)
        && ensure_managed_skills_block(&claude_path)
            .map_err(|e| StructuredError::internal(&e, "check_and_fix_agent_instruction_docs"))?
        && !updated_files.contains(&CLAUDE_MD.to_string())
    {
        updated_files.push(CLAUDE_MD.to_string());
    }

    let agents_path = worktree_path.join(AGENTS_MD);
    if ensure_claude_ref_file(&agents_path)
        .map_err(|e| StructuredError::internal(&e, "check_and_fix_agent_instruction_docs"))?
    {
        updated_files.push(AGENTS_MD.to_string());
    }

    if skill_prefs.map(|p| p.inject_agents_md).unwrap_or(false)
        && ensure_managed_skills_block(&agents_path)
            .map_err(|e| StructuredError::internal(&e, "check_and_fix_agent_instruction_docs"))?
        && !updated_files.contains(&AGENTS_MD.to_string())
    {
        updated_files.push(AGENTS_MD.to_string());
    }

    let gemini_path = worktree_path.join(GEMINI_MD);
    if ensure_claude_ref_file(&gemini_path)
        .map_err(|e| StructuredError::internal(&e, "check_and_fix_agent_instruction_docs"))?
    {
        updated_files.push(GEMINI_MD.to_string());
    }

    if skill_prefs.map(|p| p.inject_gemini_md).unwrap_or(false)
        && ensure_managed_skills_block(&gemini_path)
            .map_err(|e| StructuredError::internal(&e, "check_and_fix_agent_instruction_docs"))?
        && !updated_files.contains(&GEMINI_MD.to_string())
    {
        updated_files.push(GEMINI_MD.to_string());
    }

    Ok(InstructionDocsCheckResult {
        worktree_path: worktree_path.to_string_lossy().to_string(),
        checked_files,
        updated_files,
    })
}

#[cfg(test)]
mod tests {
    use gwt_core::process::command;
    use tempfile::TempDir;

    use super::*;

    fn run_git(path: &Path, args: &[&str]) {
        let out = command("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("git command should run");
        assert!(
            out.status.success(),
            "git command failed: git {} => {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn setup_repo_with_feature_worktree() -> (TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_path = temp.path().join("repo");
        std::fs::create_dir_all(&repo_path).expect("create repo dir");

        run_git(&repo_path, &["init"]);
        run_git(&repo_path, &["config", "user.email", "test@example.com"]);
        run_git(&repo_path, &["config", "user.name", "test"]);

        std::fs::write(repo_path.join("README.md"), "# test\n").expect("write README");
        run_git(&repo_path, &["add", "README.md"]);
        run_git(&repo_path, &["commit", "-m", "init"]);

        run_git(&repo_path, &["branch", "feature/docs-check"]);

        let worktree_path = temp.path().join("wt-feature-docs-check");
        let worktree_path_str = worktree_path.to_string_lossy().to_string();
        let out = command("git")
            .args(["worktree", "add", &worktree_path_str, "feature/docs-check"])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree add should run");
        assert!(
            out.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        (temp, repo_path, worktree_path)
    }

    #[test]
    fn check_and_fix_creates_missing_docs() {
        let (_temp, repo_path, worktree_path) = setup_repo_with_feature_worktree();
        let project_path = repo_path.to_string_lossy().to_string();

        let out =
            check_and_fix_agent_instruction_docs(project_path, "feature/docs-check".to_string())
                .expect("check/fix should succeed");

        assert_eq!(out.checked_files, vec![CLAUDE_MD, AGENTS_MD, GEMINI_MD]);
        assert_eq!(out.updated_files, vec![CLAUDE_MD, AGENTS_MD, GEMINI_MD]);
        let expected_worktree_path =
            dunce::canonicalize(&worktree_path).expect("canonicalize expected worktree path");
        let actual_worktree_path = dunce::canonicalize(Path::new(&out.worktree_path))
            .expect("canonicalize actual worktree path");
        assert_eq!(actual_worktree_path, expected_worktree_path);

        let claude =
            std::fs::read_to_string(worktree_path.join(CLAUDE_MD)).expect("read CLAUDE.md");
        assert!(claude.contains("## ワークフロー設計"));
        assert!(claude.contains("## タスク管理"));
        assert!(claude.contains("<!-- BEGIN gwt managed skills -->"));

        let agents =
            std::fs::read_to_string(worktree_path.join(AGENTS_MD)).expect("read AGENTS.md");
        assert_eq!(agents, "@CLAUDE.md\n");

        let gemini =
            std::fs::read_to_string(worktree_path.join(GEMINI_MD)).expect("read GEMINI.md");
        assert_eq!(gemini, "@CLAUDE.md\n");
    }

    #[test]
    fn check_and_fix_preserves_existing_contents_and_only_patches_missing_ref() {
        let (_temp, repo_path, worktree_path) = setup_repo_with_feature_worktree();
        let project_path = repo_path.to_string_lossy().to_string();

        let claude_path = worktree_path.join(CLAUDE_MD);
        let agents_path = worktree_path.join(AGENTS_MD);
        let gemini_path = worktree_path.join(GEMINI_MD);

        std::fs::write(&claude_path, "# custom claude\n").expect("write CLAUDE.md");
        std::fs::write(&agents_path, "custom agents instructions\n").expect("write AGENTS.md");
        std::fs::write(&gemini_path, "@CLAUDE.md\ncustom gemini instructions\n")
            .expect("write GEMINI.md");

        let out =
            check_and_fix_agent_instruction_docs(project_path, "feature/docs-check".to_string())
                .expect("check/fix should succeed");

        // CLAUDE.md is updated (skills block injected by default), AGENTS.md gets @CLAUDE.md ref
        assert!(out.updated_files.contains(&CLAUDE_MD.to_string()));
        assert!(out.updated_files.contains(&AGENTS_MD.to_string()));

        let claude = std::fs::read_to_string(&claude_path).expect("read CLAUDE.md");
        assert!(claude.contains("# custom claude"));
        assert!(claude.contains("<!-- BEGIN gwt managed skills -->"));

        let agents = std::fs::read_to_string(&agents_path).expect("read AGENTS.md");
        assert!(agents.starts_with("@CLAUDE.md\n"));
        assert!(agents.contains("custom agents instructions"));

        let gemini = std::fs::read_to_string(&gemini_path).expect("read GEMINI.md");
        assert_eq!(gemini, "@CLAUDE.md\ncustom gemini instructions\n");
    }

    #[test]
    fn check_and_fix_returns_error_when_worktree_is_missing() {
        let (_temp, repo_path, _worktree_path) = setup_repo_with_feature_worktree();
        let project_path = repo_path.to_string_lossy().to_string();

        let out =
            check_and_fix_agent_instruction_docs(project_path, "feature/not-found".to_string());
        assert!(out.is_err());
    }

    #[test]
    fn ensure_claude_md_returns_error_for_non_utf8_content_without_overwrite() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file_path = temp.path().join(CLAUDE_MD);
        let original = vec![0xff, 0xfe, 0x41, 0x00];
        std::fs::write(&file_path, &original).expect("write non-utf8 CLAUDE.md");

        let out = ensure_claude_md(&file_path);
        assert!(out.is_err());
        let after = std::fs::read(&file_path).expect("read CLAUDE.md bytes");
        assert_eq!(after, original);
    }

    #[test]
    fn ensure_claude_ref_file_returns_error_for_non_utf8_content_without_overwrite() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file_path = temp.path().join(AGENTS_MD);
        let original = vec![0xff, 0xfe, 0x41, 0x00];
        std::fs::write(&file_path, &original).expect("write non-utf8 AGENTS.md");

        let out = ensure_claude_ref_file(&file_path);
        assert!(out.is_err());
        let after = std::fs::read(&file_path).expect("read AGENTS.md bytes");
        assert_eq!(after, original);
    }

    #[test]
    fn check_and_fix_creates_docs_with_skills_block() {
        let (_temp, repo_path, worktree_path) = setup_repo_with_feature_worktree();
        let project_path = repo_path.to_string_lossy().to_string();

        let out =
            check_and_fix_agent_instruction_docs(project_path, "feature/docs-check".to_string())
                .expect("check/fix should succeed");

        assert!(out.updated_files.contains(&CLAUDE_MD.to_string()));

        let claude =
            std::fs::read_to_string(worktree_path.join(CLAUDE_MD)).expect("read CLAUDE.md");
        assert!(claude.contains("<!-- BEGIN gwt managed skills -->"));
        assert!(claude.contains("<!-- END gwt managed skills -->"));
        assert!(claude.contains("## Available Skills & Commands (gwt)"));
    }

    #[test]
    fn check_and_fix_updates_existing_claude_md_with_skills_block() {
        let (_temp, repo_path, worktree_path) = setup_repo_with_feature_worktree();
        let project_path = repo_path.to_string_lossy().to_string();

        let claude_path = worktree_path.join(CLAUDE_MD);
        let original = "# My Project\n\nCustom instructions here.\n";
        std::fs::write(&claude_path, original).expect("write CLAUDE.md");

        let out =
            check_and_fix_agent_instruction_docs(project_path, "feature/docs-check".to_string())
                .expect("check/fix should succeed");

        assert!(out.updated_files.contains(&CLAUDE_MD.to_string()));

        let claude = std::fs::read_to_string(&claude_path).expect("read CLAUDE.md");
        // Original content preserved
        assert!(claude.contains("# My Project"));
        assert!(claude.contains("Custom instructions here."));
        // Skills block injected
        assert!(claude.contains("<!-- BEGIN gwt managed skills -->"));
    }

    #[test]
    fn check_and_fix_replaces_outdated_skills_block() {
        let (_temp, repo_path, worktree_path) = setup_repo_with_feature_worktree();
        let project_path = repo_path.to_string_lossy().to_string();

        let claude_path = worktree_path.join(CLAUDE_MD);
        let old_content = "# Project\n\n<!-- BEGIN gwt managed skills -->\nold content\n<!-- END gwt managed skills -->\n";
        std::fs::write(&claude_path, old_content).expect("write CLAUDE.md");

        let out =
            check_and_fix_agent_instruction_docs(project_path, "feature/docs-check".to_string())
                .expect("check/fix should succeed");

        assert!(out.updated_files.contains(&CLAUDE_MD.to_string()));

        let claude = std::fs::read_to_string(&claude_path).expect("read CLAUDE.md");
        assert!(claude.contains("# Project"));
        // Old content replaced with current skills block
        assert!(!claude.contains("old content"));
        assert!(claude.contains("## Available Skills & Commands (gwt)"));
    }

    #[test]
    fn agents_md_not_injected_by_default() {
        let (_temp, repo_path, worktree_path) = setup_repo_with_feature_worktree();
        let project_path = repo_path.to_string_lossy().to_string();

        let _out =
            check_and_fix_agent_instruction_docs(project_path, "feature/docs-check".to_string())
                .expect("check/fix should succeed");

        let agents =
            std::fs::read_to_string(worktree_path.join(AGENTS_MD)).expect("read AGENTS.md");
        // Default: inject_agents_md = false, so no skills block
        assert!(!agents.contains("<!-- BEGIN gwt managed skills -->"));
    }

    #[test]
    fn ensure_managed_skills_block_returns_false_for_missing_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("NONEXISTENT.md");

        let result = ensure_managed_skills_block(&path).expect("should not error");
        assert!(!result);
    }

    #[test]
    fn ensure_managed_skills_block_injects_into_existing_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(CLAUDE_MD);
        std::fs::write(&path, "# Existing\n").expect("write");

        let result = ensure_managed_skills_block(&path).expect("should succeed");
        assert!(result);

        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("# Existing"));
        assert!(content.contains("<!-- BEGIN gwt managed skills -->"));
    }

    #[test]
    fn ensure_managed_skills_block_is_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(CLAUDE_MD);
        std::fs::write(&path, "# Existing\n").expect("write");

        let first = ensure_managed_skills_block(&path).expect("first call");
        assert!(first);

        let second = ensure_managed_skills_block(&path).expect("second call");
        assert!(!second); // No change
    }
}
