//! T-060 (SPEC #1942) — block-git-dir-override golden tests.

use gwt::cli::hook::block_git_dir_override;

fn block(cmd: &str) {
    assert!(
        block_git_dir_override::evaluate_bash_command(cmd).is_some(),
        "expected BLOCK for {cmd:?}"
    );
}

fn allow(cmd: &str) {
    assert!(
        block_git_dir_override::evaluate_bash_command(cmd).is_none(),
        "expected ALLOW for {cmd:?}"
    );
}

#[test]
fn git_dir_flag_on_git_invocation_is_allowed() {
    // The original MJS helper actually keys off the ENV override forms
    // (`GIT_DIR=...`, `env GIT_DIR=...`, `export GIT_DIR=...`,
    // `declare -x GIT_DIR=...`). A bare `git --git-dir=...` flag is
    // allowed. This test pins that behaviour so we do not
    // accidentally widen the rule.
    allow("git --git-dir=/other/.git status");
}

#[test]
fn env_var_prefix_git_dir_is_blocked() {
    block("GIT_DIR=/other/.git git status");
}

#[test]
fn env_command_setting_git_dir_is_blocked() {
    block("env GIT_DIR=/other/.git git status");
}

#[test]
fn export_git_dir_is_blocked() {
    block("export GIT_DIR=/other/.git");
}

#[test]
fn declare_x_git_dir_is_blocked() {
    block("declare -x GIT_DIR=/other/.git");
}

#[test]
fn git_work_tree_override_is_blocked() {
    block("GIT_WORK_TREE=/somewhere git status");
    block("export GIT_WORK_TREE=/somewhere");
}

#[test]
fn plain_git_status_is_allowed() {
    allow("git status");
}

#[test]
fn innocuous_var_with_similar_prefix_is_allowed() {
    // `GIT_DIRECTORY` (not a real git var) must not false-positive.
    allow("GIT_DIRECTORY=/tmp echo hi");
}

#[test]
fn shared_repository_identity_config_writes_are_blocked() {
    for command in [
        "git config core.bare false",
        "git config --local core.worktree /mnt/e/gwt/work/issue-5255",
        "git -C /repo config --replace-all core.bare false",
        "git --git-dir=/repo/shared.git config set core.worktree /tmp/worktree",
        "git config --worktree core.worktree /tmp/worktree",
        "git config --unset core.bare",
        "git config unset core.worktree",
        "git config --add 'core.bare' false",
        "env LANG=C /usr/bin/git -c user.name=Test config --type bool core.BARE false",
        "wsl -d Ubuntu --exec bash -lc 'git config core.bare false'",
    ] {
        block(command);
    }
}

#[test]
fn repository_config_reads_and_unrelated_writes_remain_allowed() {
    for command in [
        "git config core.bare",
        "git config --get core.worktree",
        "git config --get core.worktree /tmp/read-value-pattern",
        "git config get core.bare",
        "git config --show-origin --get-regexp 'core\\.worktree|core\\.bare'",
        "git config --local user.email agent@example.invalid",
        "git config set --value core.bare user.email agent@example.invalid",
        "git -c core.bare=false status",
        "echo 'git config core.bare false'",
    ] {
        allow(command);
    }
}

#[test]
fn explicit_fixture_config_is_allowed_but_shared_config_is_blocked() {
    let temp = tempfile::tempdir().unwrap();
    let bare = temp.path().join("shared.git");
    std::fs::create_dir_all(bare.join("objects")).unwrap();
    std::fs::create_dir_all(bare.join("refs")).unwrap();
    std::fs::write(bare.join("HEAD"), "ref: refs/heads/develop\n").unwrap();
    std::fs::write(bare.join("config"), "[core]\n bare = true\n").unwrap();
    let fixture = temp.path().join("fixture config");
    std::fs::write(&fixture, "").unwrap();
    let evaluate = |path: &std::path::Path| {
        gwt::cli::hook::block_bash_policy::evaluate_bash_command(
            &format!("git config --file '{}' core.bare false", path.display()),
            &bare,
        )
    };
    assert!(evaluate(&fixture).is_none());
    let decision = evaluate(&bare.join("config")).unwrap();
    assert!(decision
        .detail()
        .contains("git config --show-origin --get core.bare"));
    assert!(decision.detail().contains("board.post"));
    assert!(evaluate(std::path::Path::new("config")).is_some());

    let worktree = temp.path().join("worktree");
    let admin = bare.join("worktrees/task");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )
    .unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    assert!(gwt::cli::hook::block_bash_policy::evaluate_bash_command(
        &format!(
            "git config --file '{}' core.bare false",
            bare.join("config").display()
        ),
        &worktree,
    )
    .is_some());

    let nested = worktree.join("src");
    std::fs::create_dir_all(&nested).unwrap();
    for (command, blocked) in [
        ("git config --file '../../shared.git/config' core.bare false", true),
        ("git config --file '../../fixture config' core.bare false", false),
        ("git -C .. config --file '../shared.git/config' core.bare false", true),
        ("git -C .. config --file '../fixture config' core.bare false", false),
        ("git --git-dir='../../shared.git' config --file '../../shared.git/config' core.bare false", true),
        ("git --git-dir='../../shared.git' config --file '../../fixture config' core.bare false", false),
    ] {
        let event = serde_json::from_value(serde_json::json!({
            "tool_name": "exec_command",
            "tool_input": {"cmd": command},
            "cwd": nested,
        }))
        .unwrap();
        let decision = gwt::cli::hook::block_bash_policy::evaluate(&event, &worktree).unwrap();
        assert_eq!(
            decision != gwt::cli::hook::HookOutput::Silent,
            blocked,
            "{command}"
        );
    }

    let fixture_dir = temp.path().join("fixture");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    std::fs::write(fixture_dir.join("config"), "").unwrap();
    for (workdir, blocked) in [
        (bare.as_path(), true),
        (std::path::Path::new("../../shared.git"), true),
        (fixture_dir.as_path(), false),
    ] {
        let event = serde_json::from_value(serde_json::json!({
            "tool_name": "exec_command",
            "tool_input": {"cmd": "git config --file config core.bare false", "workdir": workdir},
            "cwd": nested,
        }))
        .unwrap();
        let decision = gwt::cli::hook::block_bash_policy::evaluate(&event, &worktree).unwrap();
        assert_eq!(
            decision != gwt::cli::hook::HookOutput::Silent,
            blocked,
            "{}",
            workdir.display()
        );
    }
}

#[test]
fn codex_shell_payloads_reject_shared_config_writes_and_nested_wsl_overrides() {
    for command in [
        "git config core.bare false",
        "wsl -d Ubuntu --exec bash -lc 'GIT_DIR=/mnt/e/gwt/shared.git git status'",
    ] {
        let event = serde_json::from_value(serde_json::json!({
            "tool_name": "exec_command",
            "tool_input": {"cmd": command},
        }))
        .unwrap();
        let decision =
            gwt::cli::hook::block_bash_policy::evaluate(&event, std::path::Path::new("/repo"))
                .unwrap();
        assert_ne!(decision, gwt::cli::hook::HookOutput::Silent, "{command}");
    }
}
