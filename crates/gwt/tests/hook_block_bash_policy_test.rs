//! T-035 (SPEC #1942 amendment) — block-bash-policy golden tests.

use std::path::{Path, PathBuf};

use gwt::cli::{
    governance::GovernanceEffect,
    hook::{
        block_bash_policy,
        effect_classifier::{
            self, ObservationConfidence, RepositoryTarget, EFFECT_OBSERVATION_REVISION,
        },
        HookOutput,
    },
};

fn root() -> PathBuf {
    std::env::temp_dir().join("gwt-test-worktree")
}

fn outside_root() -> PathBuf {
    root()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("gwt-test-outside")
}

fn block(command: &str) {
    assert!(
        block_bash_policy::evaluate_bash_command(command, &root()).is_some(),
        "expected BLOCK for {command:?}"
    );
}

fn allow(command: &str) {
    assert!(
        block_bash_policy::evaluate_bash_command(command, &root()).is_none(),
        "expected ALLOW for {command:?}"
    );
}

fn classify(command: &str) -> effect_classifier::EffectObservation {
    effect_classifier::classify_bash_command(command, &root(), &root())
        .unwrap_or_else(|| panic!("expected semantic classification for {command:?}"))
}

#[derive(Debug, Clone, Copy)]
enum ExistingHookDecision {
    Allow,
    GithubWorkflowBlock,
    GithubMutationBlock,
    WorktreeBlock,
    BranchModificationBlock,
    BranchSwitchBlock,
    FileOutsideWorktreeBlock,
}

fn expected_existing_hook_output(
    decision: ExistingHookDecision,
    command: &str,
) -> Option<HookOutput> {
    match decision {
        ExistingHookDecision::Allow => None,
        ExistingHookDecision::GithubWorkflowBlock => {
            Some(HookOutput::pre_tool_use_permission(
                "🚫 Direct GitHub workflow CLI commands are not allowed",
                format!(
                    "Use the gwt workflow surfaces instead of direct `gh issue`, `gh pr`, `gh run`, or workflow-focused `gh api` commands.\n\n\
Recommended alternatives:\n\
- read: JSON operations `issue.view`, `issue.comments`, `issue.linked_prs`\n\
- write: JSON operations `issue.create`, `issue.comment`\n\
- PR workflow: JSON operations `pr.current`, `pr.view`, `pr.create`, `pr.edit`, `pr.ready`, `pr.draft`, `pr.comment`, `pr.checks`\n\
- PR reviews: JSON operations `pr.reviews`, `pr.review_threads`, `pr.review_threads.reply_and_resolve`\n\
- Actions logs: JSON operations `actions.logs`, `actions.job_logs`\n\
- discovery: `gwt-search`, `~/.gwt/cache/issues/<repo-hash>/`\n\n\
Blocked command: {command}"
                ),
            ))
        }
        ExistingHookDecision::GithubMutationBlock => {
            Some(HookOutput::pre_tool_use_permission(
                "🚫 Direct GitHub API mutations are not allowed",
                format!(
                    "GitHub writes must go through the canonical gwt operations so the completion/PR gates and audit state see them (SPEC-3248 P10, T-217).\n\n\
Recommended alternatives:\n\
- PRs: JSON operations `pr.create`, `pr.edit`, `pr.ready`, `pr.draft`, `pr.comment`, `pr.review_threads.reply_and_resolve`\n\
- Issues/SPECs: JSON operations `issue.create`, `issue.comment`, `issue.spec.*`\n\
- lifecycle state: JSON operations `intake.outcome.record`, `execution.*`, `verify.*`\n\
- releases: the release workflow owns publishing — not agent Bash\n\n\
Blocked command: {command}"
                ),
            ))
        }
        ExistingHookDecision::WorktreeBlock => Some(HookOutput::pre_tool_use_permission(
            "🚫 Worktree commands are not allowed",
            format!(
                "Worktree management operations such as git worktree add/remove cannot be \
                 executed from within a worktree.\n\nBlocked command: {command}"
            ),
        )),
        ExistingHookDecision::BranchModificationBlock => {
            Some(HookOutput::pre_tool_use_permission(
                "🚫 Branch modification commands are not allowed",
                format!(
                    "Worktree is designed to complete work on the launched branch. Destructive \
                     branch operations such as git branch -d, git branch -m cannot be \
                     executed.\n\nBlocked command: {command}"
                ),
            ))
        }
        ExistingHookDecision::BranchSwitchBlock => Some(HookOutput::pre_tool_use_permission(
            "🚫 Branch switching commands (checkout/switch) are not allowed",
            format!(
                "Worktree is designed to complete work on the launched branch. Branch operations \
                 such as git checkout and git switch cannot be executed.\n\nBlocked command: \
                 {command}"
            ),
        )),
        ExistingHookDecision::FileOutsideWorktreeBlock => {
            let worktree_root = root();
            Some(HookOutput::pre_tool_use_permission(
                "🚫 File operations outside worktree are not allowed",
                format!(
                    "Worktree is designed to complete work within the launched directory. File operations outside the worktree cannot be executed.\n\n\
Worktree root: {}\n\
Target path: notes.tmp\n\
Blocked command: {command}\n\n\
Instead, use absolute paths within worktree, e.g., 'mkdir ./new-dir' or 'rm ./file.txt'",
                    worktree_root.display()
                ),
            ))
        }
    }
}

#[test]
fn effect_observation_schema_is_versioned_and_forward_additive() {
    let observation = classify("git status --short");

    assert_eq!(
        observation.observation_revision,
        EFFECT_OBSERVATION_REVISION
    );
    assert_eq!(observation.confidence, ObservationConfidence::Heuristic);
    assert_eq!(observation.reason, "shell_read_only_heuristic");

    let encoded = serde_json::to_value(&observation).expect("serialize effect observation");
    assert_eq!(encoded["observation_revision"], EFFECT_OBSERVATION_REVISION);
    assert_eq!(encoded["confidence"], "heuristic");
    assert_eq!(encoded["reason"], "shell_read_only_heuristic");
    assert_eq!(
        serde_json::from_value::<effect_classifier::EffectObservation>(encoded.clone())
            .expect("round-trip effect observation"),
        observation
    );

    let mut future = encoded;
    future["observation_revision"] = serde_json::json!(EFFECT_OBSERVATION_REVISION + 1);
    future["future_additive_diagnostic"] = serde_json::json!({ "detail": "ignored" });
    let decoded = serde_json::from_value::<effect_classifier::EffectObservation>(future)
        .expect("unknown additive fields stay tolerated");
    assert_eq!(
        decoded.observation_revision,
        EFFECT_OBSERVATION_REVISION + 1
    );
}

#[test]
fn semantic_classifier_covers_shell_target_and_effect_matrix() {
    let scratch = root()
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("semantic-classifier-scratch");
    let cases = [
        (
            "git status --short".to_string(),
            RepositoryTarget::ManagedCurrent,
            "git.status",
            GovernanceEffect::Observe,
            ExistingHookDecision::Allow,
        ),
        (
            format!("git -C {} log -1", scratch.display()),
            RepositoryTarget::ExternalPath,
            "git.log",
            GovernanceEffect::Observe,
            ExistingHookDecision::Allow,
        ),
        (
            "gh pr view 1949".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.pr.view",
            GovernanceEffect::Observe,
            ExistingHookDecision::GithubWorkflowBlock,
        ),
        (
            "touch notes.tmp".to_string(),
            RepositoryTarget::ManagedCurrent,
            "local.touch",
            GovernanceEffect::Reversible,
            ExistingHookDecision::FileOutsideWorktreeBlock,
        ),
        (
            "gh pr create --draft --title test --body body".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.pr.create",
            GovernanceEffect::Reversible,
            ExistingHookDecision::GithubWorkflowBlock,
        ),
        (
            "gh pr create -d --title test --body body".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.pr.create",
            GovernanceEffect::Reversible,
            ExistingHookDecision::GithubWorkflowBlock,
        ),
        (
            "gh pr create --title ready --body body".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.pr.create",
            GovernanceEffect::Protected,
            ExistingHookDecision::GithubWorkflowBlock,
        ),
        (
            "gh pr edit 1949 --title updated".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.pr.edit",
            GovernanceEffect::Reversible,
            ExistingHookDecision::GithubWorkflowBlock,
        ),
        (
            "gh pr ready 1949".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.pr.ready",
            GovernanceEffect::Protected,
            ExistingHookDecision::GithubWorkflowBlock,
        ),
        (
            "gh pr merge 1949".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.pr.merge",
            GovernanceEffect::Protected,
            ExistingHookDecision::Allow,
        ),
        (
            "gh release create v1.0.0 --notes done".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.release.create",
            GovernanceEffect::Protected,
            ExistingHookDecision::GithubMutationBlock,
        ),
        (
            "git worktree remove ../old-work".to_string(),
            RepositoryTarget::ManagedCurrent,
            "git.worktree",
            GovernanceEffect::Protected,
            ExistingHookDecision::WorktreeBlock,
        ),
        (
            "git worktree list --porcelain".to_string(),
            RepositoryTarget::ManagedCurrent,
            "git.worktree",
            GovernanceEffect::Observe,
            ExistingHookDecision::WorktreeBlock,
        ),
        (
            "git branch -D old-work".to_string(),
            RepositoryTarget::ManagedCurrent,
            "git.branch",
            GovernanceEffect::Protected,
            ExistingHookDecision::BranchModificationBlock,
        ),
        (
            "gh pr comment 1949 --body done".to_string(),
            RepositoryTarget::ManagedCurrent,
            "gh.pr.comment",
            GovernanceEffect::Protected,
            ExistingHookDecision::GithubWorkflowBlock,
        ),
        (
            "curl -X POST https://api.github.com/user/following/example".to_string(),
            RepositoryTarget::UnknownRemote,
            "remote.mutation",
            GovernanceEffect::Protected,
            ExistingHookDecision::GithubMutationBlock,
        ),
        (
            "curl https://api.github.com/repos/akiojin/gwt".to_string(),
            RepositoryTarget::ExplicitRemote("akiojin/gwt".to_string()),
            "remote.query",
            GovernanceEffect::Observe,
            ExistingHookDecision::Allow,
        ),
        (
            "gh api -X POST user/following/example".to_string(),
            RepositoryTarget::UnknownRemote,
            "gh.api",
            GovernanceEffect::Protected,
            ExistingHookDecision::GithubMutationBlock,
        ),
    ];

    for (command, expected_target, expected_operation, expected_effect, expected_decision) in cases
    {
        let observation = classify(&command);
        assert_eq!(observation.target, expected_target, "{command}");
        assert_eq!(observation.operation, expected_operation, "{command}");
        assert_eq!(observation.effect, expected_effect, "{command}");
        assert_eq!(
            block_bash_policy::evaluate_bash_command(&command, &root()),
            expected_existing_hook_output(expected_decision, &command),
            "classifier observation must preserve the exact existing HookOutput for {command}"
        );
    }
}

#[test]
fn generic_curl_remote_effects_are_method_aware_observations_only() {
    let cases = [
        (
            "curl -X POST https://example.test/resource",
            GovernanceEffect::Protected,
            "remote.mutation",
        ),
        (
            "curl https://example.test/resource --request PUT",
            GovernanceEffect::Protected,
            "remote.mutation",
        ),
        (
            "curl -XPATCH https://example.test/resource",
            GovernanceEffect::Protected,
            "remote.mutation",
        ),
        (
            "curl -X DELETE https://example.test/resource",
            GovernanceEffect::Protected,
            "remote.mutation",
        ),
        (
            "curl --data name=value https://example.test/resource",
            GovernanceEffect::Protected,
            "remote.mutation",
        ),
        (
            "curl --form file=@artifact.bin https://example.test/resource",
            GovernanceEffect::Protected,
            "remote.mutation",
        ),
        (
            "curl --upload-file artifact.bin https://example.test/resource",
            GovernanceEffect::Protected,
            "remote.mutation",
        ),
        (
            "curl https://example.test/resource",
            GovernanceEffect::Observe,
            "remote.query",
        ),
        (
            "curl -G --data q=value https://example.test/resource",
            GovernanceEffect::Observe,
            "remote.query",
        ),
        (
            "curl -X GET --data q=value https://example.test/resource",
            GovernanceEffect::Observe,
            "remote.query",
        ),
    ];

    for (command, expected_effect, expected_operation) in cases {
        let observation = classify(command);

        assert_eq!(
            observation.target,
            RepositoryTarget::UnknownRemote,
            "{command}"
        );
        assert_eq!(observation.effect, expected_effect, "{command}");
        assert_eq!(observation.operation, expected_operation, "{command}");
        assert_eq!(
            block_bash_policy::evaluate_bash_command(command, &root()),
            None,
            "generic remote observations must preserve the exact existing HookOutput for {command}"
        );
    }
}

#[test]
fn gh_api_operation_is_stable_across_flag_order() {
    for command in [
        "gh api -X POST user/following/example",
        "gh api user/following/example -X POST",
        "gh api --method=PATCH user/following/example",
        "gh api user/following/example --method=PATCH",
    ] {
        let observation = classify(command);

        assert_eq!(observation.operation, "gh.api", "{command}");
        assert_eq!(observation.effect, GovernanceEffect::Protected, "{command}");
    }
}

#[test]
fn semantic_observation_preserves_existing_block_bash_decisions() {
    let scratch = root()
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("semantic-classifier-scratch");
    let scratch_read = format!("git -C {} status --short", scratch.display());
    assert_eq!(classify(&scratch_read).effect, GovernanceEffect::Observe);
    allow(&scratch_read);

    let merge = "gh pr merge 1949";
    assert_eq!(classify(merge).effect, GovernanceEffect::Protected);
    allow(merge);

    let destructive = "git worktree remove ../old-work";
    assert_eq!(classify(destructive).effect, GovernanceEffect::Protected);
    block(destructive);

    let unknown_remote = "curl -X POST https://api.github.com/user/following/example";
    assert_eq!(classify(unknown_remote).effect, GovernanceEffect::Protected);
    block(unknown_remote);
}

#[test]
fn scratch_repository_checkout_and_switch_are_reversible_observations() {
    let scratch = root()
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("semantic-classifier-scratch");

    for subcommand in ["checkout main", "switch main"] {
        let command = format!("git -C {} {subcommand}", scratch.display());
        let observation = classify(&command);

        assert_eq!(observation.target, RepositoryTarget::ExternalPath);
        assert_eq!(observation.confidence, ObservationConfidence::Heuristic);
        assert_eq!(observation.reason, "lexical_external_repository_unverified");
        assert_eq!(observation.effect, GovernanceEffect::Reversible);
        block(&command);
    }
}

#[test]
fn lexical_external_paths_never_claim_exact_repository_authority() {
    let external_root = root()
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("unverified-external");

    for target in [
        external_root.join("sibling"),
        external_root.join("arbitrary/path"),
    ] {
        let command = format!("git -C {} status --short", target.display());
        let observation = classify(&command);

        assert_eq!(observation.target, RepositoryTarget::ExternalPath);
        assert_eq!(observation.confidence, ObservationConfidence::Heuristic);
        assert_eq!(observation.reason, "lexical_external_repository_unverified");
    }
}

#[cfg(unix)]
#[test]
fn symlinked_path_classification_is_lexical_and_never_exact() {
    use std::os::unix::fs::symlink;

    let fixture = tempfile::tempdir().expect("semantic path fixture");
    let managed = fixture.path().join("managed");
    let external = fixture.path().join("external");
    let linked = managed.join("linked-repository");
    std::fs::create_dir_all(&managed).expect("create managed fixture");
    std::fs::create_dir_all(&external).expect("create external fixture");

    let command = format!("git -C {} status --short", linked.display());
    let before = effect_classifier::classify_bash_command(&command, &managed, &managed)
        .expect("classify missing lexical path");
    symlink(&external, &linked).expect("create external repository symlink");
    let after = effect_classifier::classify_bash_command(&command, &managed, &managed)
        .expect("classify symlinked lexical path");

    assert_eq!(
        after, before,
        "classifier must not consult filesystem state"
    );
    assert_eq!(after.target, RepositoryTarget::ManagedCurrent);
    assert_eq!(after.confidence, ObservationConfidence::Heuristic);
    assert_eq!(after.reason, "lexical_managed_path_unverified");
}

#[test]
fn managed_current_checkout_and_switch_remain_protected_observations() {
    for command in ["git checkout main", "git switch main"] {
        let observation = classify(command);

        assert_eq!(observation.target, RepositoryTarget::ManagedCurrent);
        assert_eq!(observation.effect, GovernanceEffect::Protected);
        block(command);
    }
}

#[test]
fn managed_authority_and_destructive_commands_are_protected_observations_only() {
    let managed_root = root();
    let commands = [
        "git restore -- src/lib.rs".to_string(),
        format!("rm -f {}/src/lib.rs", managed_root.display()),
        format!("rmdir {}/target/tmp", managed_root.display()),
        format!("unlink {}/src/generated.rs", managed_root.display()),
        format!("truncate -s 0 {}/src/lib.rs", managed_root.display()),
        format!("shred {}/secrets.txt", managed_root.display()),
        format!("chmod 600 {}/config.json", managed_root.display()),
        format!("chown 1000:1000 {}/config.json", managed_root.display()),
        format!(
            "dd if=/dev/zero of={}/artifact.bin bs=1 count=1",
            managed_root.display()
        ),
    ];

    for command in commands {
        let observation = classify(&command);

        assert_eq!(
            observation.target,
            RepositoryTarget::ManagedCurrent,
            "{command}"
        );
        assert_eq!(observation.effect, GovernanceEffect::Protected, "{command}");
        assert_eq!(
            block_bash_policy::evaluate_bash_command(&command, &managed_root),
            None,
            "observe-only classification must preserve the exact existing HookOutput for {command}"
        );
    }
}

#[test]
fn managed_git_ref_history_and_path_restore_are_protected_observations_only() {
    let commands = [
        "git reset",
        "git reset HEAD~1",
        "git reset --soft HEAD~1",
        "git reset --mixed HEAD~1",
        "git reset --merge HEAD~1",
        "git reset --keep HEAD~1",
        "git update-ref refs/heads/main HEAD~1",
        "git rebase main",
        "git commit --amend --no-edit",
        "git tag -d v1.0.0",
        "git tag --delete v1.0.0",
        "git tag -f v1.0.0 HEAD~1",
        "git tag --force v1.0.0 HEAD~1",
        "git checkout -- src/lib.rs",
        "git checkout HEAD~1 -- src/lib.rs",
        "git checkout --ours src/lib.rs",
        "git checkout --theirs src/lib.rs",
        "git checkout --patch src/lib.rs",
    ];

    for command in commands {
        let observation = classify(command);
        let expected_decision = if command == "git checkout --patch src/lib.rs" {
            ExistingHookDecision::BranchSwitchBlock
        } else {
            ExistingHookDecision::Allow
        };

        assert_eq!(
            observation.target,
            RepositoryTarget::ManagedCurrent,
            "{command}"
        );
        assert_eq!(observation.effect, GovernanceEffect::Protected, "{command}");
        assert_eq!(
            block_bash_policy::evaluate_bash_command(command, &root()),
            expected_existing_hook_output(expected_decision, command),
            "observation must preserve the exact existing HookOutput for {command}"
        );
    }

    for command in [
        "git commit -m local-change",
        "git merge main",
        "git cherry-pick HEAD~1",
        "git revert HEAD",
        "git am patch.mbox",
        "git tag v1.0.0",
    ] {
        assert_eq!(
            classify(command).effect,
            GovernanceEffect::Reversible,
            "normal local history work must remain reversible: {command}"
        );
        assert_eq!(
            block_bash_policy::evaluate_bash_command(command, &root()),
            None,
            "normal local work must preserve its exact existing HookOutput for {command}"
        );
    }

    for command in [
        "git branch --show-current",
        "git branch --list",
        "git tag --list",
    ] {
        assert_eq!(
            classify(command).effect,
            GovernanceEffect::Observe,
            "{command}"
        );
        assert_eq!(
            block_bash_policy::evaluate_bash_command(command, &root()),
            None,
            "read-only Git queries must preserve their exact existing HookOutput for {command}"
        );
    }
}

#[test]
fn external_branch_navigation_stays_reversible_but_path_restore_is_protected() {
    let scratch = root()
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("semantic-classifier-scratch");

    for subcommand in ["checkout main", "switch main"] {
        let command = format!("git -C {} {subcommand}", scratch.display());
        assert_eq!(
            classify(&command).effect,
            GovernanceEffect::Reversible,
            "{command}"
        );
        assert_eq!(
            block_bash_policy::evaluate_bash_command(&command, &root()),
            expected_existing_hook_output(ExistingHookDecision::BranchSwitchBlock, &command),
            "external branch navigation must preserve its exact existing HookOutput for {command}"
        );
    }

    for (subcommand, expected_decision) in [
        ("checkout -- src/lib.rs", ExistingHookDecision::Allow),
        ("checkout HEAD~1 -- src/lib.rs", ExistingHookDecision::Allow),
        ("checkout --ours src/lib.rs", ExistingHookDecision::Allow),
        ("checkout --theirs src/lib.rs", ExistingHookDecision::Allow),
        (
            "checkout --patch src/lib.rs",
            ExistingHookDecision::BranchSwitchBlock,
        ),
        ("checkout -f main", ExistingHookDecision::BranchSwitchBlock),
        (
            "switch --discard-changes main",
            ExistingHookDecision::BranchSwitchBlock,
        ),
        (
            "checkout -B rewritten",
            ExistingHookDecision::BranchSwitchBlock,
        ),
        (
            "switch -C rewritten",
            ExistingHookDecision::BranchSwitchBlock,
        ),
    ] {
        let command = format!("git -C {} {subcommand}", scratch.display());
        let observation = classify(&command);

        assert_eq!(
            observation.target,
            RepositoryTarget::ExternalPath,
            "{command}"
        );
        assert_eq!(observation.effect, GovernanceEffect::Protected, "{command}");
        assert_eq!(
            block_bash_policy::evaluate_bash_command(&command, &root()),
            expected_existing_hook_output(expected_decision, &command),
            "observation must preserve the exact existing HookOutput for {command}"
        );
    }
}

#[test]
fn blocks_branch_policy_commands() {
    block("git rebase -i origin/main");
    block("git checkout main");
}

#[test]
fn blocks_cd_outside_worktree() {
    block(&format!("cd {}", outside_root().display()));
}

#[test]
fn blocks_file_ops_outside_worktree() {
    block("rm -rf /");
    block(&format!(
        "cp {}/foo.txt {}/foo.txt",
        root().display(),
        outside_root().display()
    ));
}

#[test]
fn blocks_git_dir_override_env_vars() {
    block("GIT_DIR=/other/.git git status");
    block("export GIT_WORK_TREE=/somewhere");
}

#[test]
fn blocks_workflow_focused_github_cli_commands() {
    block("gh issue view 1942");
    block("gh issue create --title \"fix: issue\" --body \"details\"");
    block("gh issue comment 1942 --body \"done\"");
    block("gh pr view 1949");
    block("gh pr create --base main --head feature/x --title test --body body");
    block("gh pr ready 1949");
    block("gh pr draft 1949");
    block("gh pr checks 1949");
    block("gh run view 123456789");
    block("env GH_TOKEN=test gh issue view 1942");
    block("gh api repos/akiojin/gwt/issues/1942");
    block("gh api /repos/akiojin/gwt/issues/1942/comments");
    block("gh api repos/akiojin/gwt/pulls/1949");
    block("gh api repos/akiojin/gwt/actions/runs/123456789");
    block("gh api graphql -f query='query { repository(owner:\"akiojin\", name:\"gwt\") { issue(number:1942) { id } } }'");
    block("gh api graphql -f query='query { repository(owner:\"akiojin\", name:\"gwt\") { pullRequest(number:1949) { id } } }'");
}

#[test]
fn blocks_long_sleep_pr_ci_polling_commands() {
    block("sleep 280 && gwtd pr view 1949");
    block("gwtd pr checks 1949; sleep 280");
    block("while true; do gwtd pr checks 1949; sleep 2m; done");
    block("sleep 280 && /Applications/GWT.app/Contents/MacOS/gwtd pr checks 1949");
    block("sleep 280 && gh run view 123456789");
    block("gh run view 123456789; sleep 0.5h");
}

#[test]
fn allows_bounded_or_non_pr_sleep_commands() {
    allow("sleep 30 && gwtd pr checks 1949");
    allow("sleep 280 && echo done");
}

#[test]
fn github_workflow_block_message_points_to_canonical_gwt_surfaces() {
    // `permissionDecisionReason` is the single field PreToolUse actually
    // surfaces, so the canonical alternatives and the blocked command
    // must all land inside it — otherwise the LLM/user only sees the
    // short rule name and has no recovery path.
    let decision = block_bash_policy::evaluate_bash_command("gh pr view 1949", &root())
        .expect("workflow gh command must block");
    let visible = decision.permission_decision_reason();

    for required in [
        "GitHub workflow CLI",
        "issue.view",
        "pr.view",
        "pr.ready",
        "pr.draft",
        "actions.logs",
        "gwt-search",
        "Blocked command: gh pr view 1949",
    ] {
        assert!(
            visible.contains(required),
            "{required:?} missing from permission_decision_reason: {visible}"
        );
    }
}

#[test]
fn long_sleep_pr_ci_block_message_points_to_board_handoff() {
    let command = "sleep 280 && gwtd pr checks 1949";
    let decision = block_bash_policy::evaluate_bash_command(command, &root())
        .expect("long PR polling sleep must block");
    let visible = decision.permission_decision_reason();

    for required in [
        "Long PR/CI polling sleeps are not allowed",
        "JSON operation `pr.checks`",
        "JSON operation `board.post`",
        "instead of sleeping indefinitely",
        command,
    ] {
        assert!(
            visible.contains(required),
            "{required:?} missing from permission_decision_reason: {visible}"
        );
    }
}

#[test]
fn allows_read_only_and_in_worktree_commands() {
    allow("git branch --list");
    allow("git checkout HEAD -- foo.rs");
    allow(&format!("mkdir {}/new-dir", root().display()));
}

#[test]
fn allows_non_workflow_github_cli_commands() {
    allow("gh auth status");
    allow("gh repo view");
    allow("gh release list");
    allow("gh api user");
    allow("gh api graphql -f query='query { viewer { login } }'");
    // The gwt-manage-pr Deliver flow depends on `gh pr merge` staying allowed:
    // it is the documented transport exception with no JSON operation.
    allow("gh pr merge --auto 1949");
    allow("gh pr merge --disable-auto 1949");
    allow("gh pr merge 1949");
}

#[test]
fn allows_search_patterns_that_mention_blocked_github_commands() {
    allow(r#"rg -n "gh pr checks|gh run view|gh api graphql" .codex"#);
}
