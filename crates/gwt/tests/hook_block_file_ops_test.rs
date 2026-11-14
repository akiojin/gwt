//! T-050 (SPEC #1942) — block-file-ops golden tests.

use std::path::{Path, PathBuf};

use gwt::cli::hook::{block_file_ops, HookOutput};

fn root() -> PathBuf {
    std::env::temp_dir().join("gwt-test-worktree")
}

fn outside_root() -> PathBuf {
    outside_root_for(&std::env::temp_dir())
}

fn outside_root_for(temp: &Path) -> PathBuf {
    // Unix also allows /tmp even when TMPDIR points to a nested child.
    temp.ancestors().last().unwrap().join("gwt-test-outside")
}

#[test]
fn rejection_fixture_is_outside_shared_temp_even_with_nested_tmpdir() {
    let shared_temp = std::env::temp_dir();
    let nested_temp = shared_temp.join("gwt-ci");
    assert!(!outside_root_for(&nested_temp).starts_with(&shared_temp));
}

#[test]
fn session_scratchpad_and_verification_artifacts_are_allowed() {
    let scratch = std::env::temp_dir().join("claude-501/project/session/scratchpad");
    let artifact = std::env::temp_dir().join("gwt-verification/failed log.txt");
    for command in [
        format!("mkdir -p '{}'", scratch.display()),
        format!("touch '{}/probe.txt'", scratch.display()),
        format!(
            "cp '{}/probe.txt' '{}'",
            scratch.display(),
            artifact.display()
        ),
        format!("mv '{}' '{}.saved'", artifact.display(), artifact.display()),
        format!("rm '{}.saved'", artifact.display()),
        format!("rmdir '{}'", scratch.display()),
    ] {
        assert!(
            block_file_ops::evaluate_bash_command(&command, &root()).is_none(),
            "expected allowed temporary operation: {command}"
        );
    }
}

#[test]
fn temporary_roots_and_traversal_outside_them_are_blocked() {
    let temp = dunce::canonicalize(std::env::temp_dir()).unwrap();
    let escaped = temp
        .join("../".repeat(temp.components().count()))
        .join("gwt-test-outside");
    for target in [temp.clone(), escaped] {
        let command = format!("rm -rf '{}'", target.display());
        assert!(block_file_ops::evaluate_bash_command(&command, &root()).is_some());
    }
}

#[test]
fn temporary_exception_does_not_allow_unresolved_shell_patterns_or_external_option_targets() {
    let temp = std::env::temp_dir().join("gwt-verification");
    for command in [
        format!("touch {}/*", temp.display()),
        format!("touch {}/{{a,b}}", temp.display()),
        format!(
            "cp {}/log --target-directory={}",
            temp.display(),
            outside_root().display()
        ),
        format!("mv -t{} {}/log", outside_root().display(), temp.display()),
        format!("cp -rt{} {}/log", outside_root().display(), temp.display()),
        format!(
            "mv --target={} {}/log",
            outside_root().display(),
            temp.display()
        ),
    ] {
        assert!(
            block_file_ops::evaluate_bash_command(&command, &root()).is_some(),
            "must reject unresolved or external target: {command}"
        );
    }
}

#[test]
fn refusal_shows_allowed_temporary_artifact_recovery() {
    let command = format!("cp '{}/failure.log' /etc/failure.log", root().display());
    let Some(HookOutput::PreToolUsePermission { detail, .. }) =
        block_file_ops::evaluate_bash_command(&command, &root())
    else {
        panic!("expected a denial");
    };
    assert!(detail.contains(&std::env::temp_dir().display().to_string()));
    assert!(detail.contains("scratchpad"));
    assert!(detail.contains("cp"));
}

#[test]
fn temporary_symlink_escape_is_blocked_even_for_missing_destinations() {
    let temp = tempfile::tempdir().unwrap();
    let link = temp.path().join("outside");
    let destination = outside_root().parent().unwrap().to_path_buf();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&destination, &link).unwrap();
    #[cfg(windows)]
    assert!(gwt_core::process::hidden_command("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&destination)
        .output()
        .unwrap()
        .status
        .success());
    let deny = |target: PathBuf| {
        let command = format!("touch '{}'", target.display());
        assert!(
            block_file_ops::evaluate_bash_command(&command, &root()).is_some(),
            "must reject symlink escape: {command}"
        );
    };
    deny(link.join("new/file.log"));
    // Win32 normalizes `..` before following junctions; Unix follows symlinks first.
    #[cfg(unix)]
    deny(link.join("../gwt-outside.log"));
}

#[test]
fn temporary_environment_path_is_allowed_when_resolved() {
    // Read the harness environment without mutating process-wide state.
    if let Some(name) = ["TMPDIR", "TEMP", "TMP"].into_iter().find(|name| {
        std::env::var_os(name).is_some_and(|value| {
            dunce::canonicalize(value).ok() == dunce::canonicalize(std::env::temp_dir()).ok()
        })
    }) {
        let command = format!("touch \"${{{name}}}/gwt-verification/probe.log\"");
        assert!(block_file_ops::evaluate_bash_command(&command, &root()).is_none());
    }
}

#[test]
fn rm_rf_root_slash_is_blocked() {
    let decision = block_file_ops::evaluate_bash_command("rm -rf /", &root());
    assert!(decision.is_some(), "rm -rf / must be blocked");
}

#[test]
fn rm_rf_home_shortcut_is_blocked() {
    let decision = block_file_ops::evaluate_bash_command("rm -rf ~", &root());
    assert!(decision.is_some(), "rm -rf ~ must be blocked");
}

#[test]
fn rm_inside_worktree_relative_path_is_allowed() {
    // Relative paths resolve against the current process cwd, which
    // during test execution is the gwt repo. Because our synthetic
    // tempdir-backed root does NOT match that cwd, relative
    // paths will be considered *outside* — so for this test we use an
    // absolute path under the synthetic root.
    let decision = block_file_ops::evaluate_bash_command(
        &format!("rm -rf {}/target", root().display()),
        &root(),
    );
    assert!(
        decision.is_none(),
        "rm -rf <path-under-root> must be allowed, got {decision:?}"
    );
}

#[test]
fn mkdir_inside_worktree_is_allowed() {
    let decision = block_file_ops::evaluate_bash_command(
        &format!("mkdir {}/new-dir", root().display()),
        &root(),
    );
    assert!(decision.is_none(), "mkdir under the root must be allowed");
}

#[test]
fn non_file_op_command_is_ignored() {
    let decision = block_file_ops::evaluate_bash_command("echo rm /etc", &root());
    assert!(
        decision.is_none(),
        "echo rm is not a file-op segment start, must not be blocked"
    );
}

#[test]
fn cp_to_path_outside_worktree_is_blocked() {
    let decision = block_file_ops::evaluate_bash_command(
        &format!(
            "cp {}/foo.txt {}/foo.txt",
            root().display(),
            outside_root().display()
        ),
        &root(),
    );
    assert!(
        decision.is_some(),
        "cp targeting outside the worktree must be blocked even if the source is inside"
    );
}

#[test]
fn flags_are_not_treated_as_file_paths() {
    // `rm -rf --no-preserve-root <root>/target` has only one path arg —
    // the flags must not trigger a
    // false-positive block.
    let decision = block_file_ops::evaluate_bash_command(
        &format!("rm -rf --no-preserve-root {}/target", root().display()),
        &root(),
    );
    assert!(decision.is_none(), "flags must not be treated as paths");
}
