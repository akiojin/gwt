//! Public API contract tests for gwt-skills managed asset distribution.
//!
//! gwt's Start Work / Launch materialization depends on these surfaces:
//! skill bundle distribution into a worktree, stale-asset pruning,
//! gwt-coordination guidance generation, and `.git/info/exclude`
//! management. These tests pin that contract against a throwaway git
//! repository fixture.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use gwt_core::process::hidden_command;
use gwt_skills::coordination_guidance::{generate_coordination_guidance, render_skill_md};
use gwt_skills::distribute::distribute_to_worktree;
use gwt_skills::git_exclude::update_git_exclude;
use gwt_skills::SessionKind;

/// Create a real (empty) git repository so asset distribution and
/// `.git/info/exclude` resolution behave as they do in a gwt worktree.
fn init_git_repo(path: &Path) {
    let status = hidden_command("git")
        .args(["init", "--quiet"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init failed for {}", path.display());
}

#[test]
fn distribute_to_worktree_materializes_claude_and_codex_skill_bundles() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());

    let report = distribute_to_worktree(dir.path()).expect("distribute bundle");

    assert!(report.files_written > 0, "bundle must write files");
    for skill_md in [
        dir.path().join(".claude/skills/gwt-verify/SKILL.md"),
        dir.path().join(".codex/skills/gwt-verify/SKILL.md"),
    ] {
        assert!(
            skill_md.is_file(),
            "expected bundled skill at {}",
            skill_md.display()
        );
    }

    let has_gwt_command = fs::read_dir(dir.path().join(".claude/commands"))
        .expect("commands dir")
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.file_name().to_string_lossy().starts_with("gwt-"));
    assert!(
        has_gwt_command,
        "at least one gwt-* command must be written"
    );
}

#[test]
fn repo_keeps_managed_claude_and_codex_skill_assets_in_parity() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let claude_root = workspace_root.join(".claude/skills");
    let codex_root = workspace_root.join(".codex/skills");

    let claude_files = collect_gwt_skill_files(&claude_root);
    let codex_files = collect_gwt_skill_files(&codex_root);

    assert_eq!(
        claude_files, codex_files,
        "managed gwt-* skill asset file lists must match between .claude and .codex"
    );

    for relative in claude_files {
        let claude = fs::read(claude_root.join(&relative))
            .unwrap_or_else(|err| panic!("read Claude asset {relative:?}: {err}"));
        let codex = fs::read(codex_root.join(&relative))
            .unwrap_or_else(|err| panic!("read Codex asset {relative:?}: {err}"));
        assert!(
            claude == codex,
            "managed gwt-* skill asset must be byte-identical between .claude and .codex: {relative:?}"
        );
    }
}

#[test]
fn distribute_to_worktree_is_idempotent_for_skill_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());
    let skill_md = dir.path().join(".claude/skills/gwt-verify/SKILL.md");

    distribute_to_worktree(dir.path()).expect("first distribute");
    let first = fs::read_to_string(&skill_md).expect("read after first run");

    distribute_to_worktree(dir.path()).expect("second distribute");
    let second = fs::read_to_string(&skill_md).expect("read after second run");

    assert_eq!(first, second, "re-distribution must be byte-identical");
}

fn collect_gwt_skill_files(skills_root: &Path) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    for entry in fs::read_dir(skills_root)
        .unwrap_or_else(|err| panic!("read skills root {}: {err}", skills_root.display()))
    {
        let entry = entry.expect("read skill root entry");
        let file_type = entry.file_type().expect("read skill root entry type");
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_dir() && name.starts_with("gwt-") {
            collect_files_relative_to(skills_root, &entry.path(), &mut files);
        }
    }
    files
}

fn collect_files_relative_to(root: &Path, current: &Path, files: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(current)
        .unwrap_or_else(|err| panic!("read asset dir {}: {err}", current.display()))
    {
        let entry = entry.expect("read asset entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("read asset entry type");
        if file_type.is_dir() {
            collect_files_relative_to(root, &path, files);
        } else if file_type.is_file() {
            files.insert(
                path.strip_prefix(root)
                    .expect("asset path must be below root")
                    .to_path_buf(),
            );
        }
    }
}

#[test]
fn distribute_to_worktree_prunes_stale_managed_skills() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());

    let stale = dir.path().join(".claude/skills/gwt-retired-test-skill");
    fs::create_dir_all(&stale).expect("create stale skill dir");
    fs::write(stale.join("SKILL.md"), "retired").expect("write stale skill");

    distribute_to_worktree(dir.path()).expect("distribute bundle");

    assert!(
        !stale.exists(),
        "gwt-* skills outside the current bundle must be pruned"
    );
}

#[test]
fn generate_coordination_guidance_writes_skill_for_claude_and_codex() {
    let dir = tempfile::tempdir().expect("tempdir");

    generate_coordination_guidance(dir.path(), SessionKind::Execution).expect("generate guidance");

    for skill_md in [
        dir.path().join(".claude/skills/gwt-coordination/SKILL.md"),
        dir.path().join(".codex/skills/gwt-coordination/SKILL.md"),
    ] {
        let content = fs::read_to_string(&skill_md)
            .unwrap_or_else(|e| panic!("read {}: {e}", skill_md.display()));
        assert!(content.contains("gwt-coordination"));
        assert!(
            content.contains("\"operation\":\"board.post\""),
            "guidance must instruct Board posting via gwtd JSON envelopes"
        );
    }
}

#[test]
fn render_skill_md_embeds_frontmatter_name_and_description() {
    let md = render_skill_md(SessionKind::Execution);
    assert!(md.starts_with("---\n"), "must start with YAML frontmatter");
    assert!(md.contains("name: gwt-coordination"));
    assert!(md.contains("\"operation\":\"board.post\""));
}

#[test]
fn update_git_exclude_inserts_managed_block_and_preserves_user_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());
    let exclude_path = dir.path().join(".git/info/exclude");
    fs::create_dir_all(exclude_path.parent().unwrap()).expect("info dir");
    fs::write(&exclude_path, "user-entry.txt\n").expect("seed user entry");

    update_git_exclude(dir.path()).expect("first update");
    update_git_exclude(dir.path()).expect("second update (idempotency)");

    let content = fs::read_to_string(&exclude_path).expect("read exclude");
    assert!(
        content.contains("user-entry.txt"),
        "user entries must be preserved"
    );
    assert_eq!(
        content.matches("# gwt-managed-begin").count(),
        1,
        "managed block must not be duplicated on repeated calls"
    );
    assert_eq!(content.matches("# gwt-managed-end").count(), 1);
}
