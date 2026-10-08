//! T-030 (SPEC #1942) — block-git-branch-ops golden tests.
//!
//! Translated from the retired branch-policy parity matrix. Each case is a
//! (command, expected_decision) tuple. `Block` means the hook MUST return
//! a deny `HookOutput`; `Allow` means it MUST return `None`.

use gwt::cli::hook::block_git_branch_ops;

#[derive(Debug)]
enum Expected {
    Block,
    Allow,
}

fn case(command: &str, expected: Expected) {
    let result = block_git_branch_ops::evaluate_bash_command(command);
    match (result, expected) {
        (Some(_), Expected::Block) => {}
        (None, Expected::Allow) => {}
        (Some(decision), Expected::Allow) => {
            panic!("expected ALLOW for {command:?}, got block: {decision:?}")
        }
        (None, Expected::Block) => panic!("expected BLOCK for {command:?}, got allow"),
    }
}

#[test]
fn interactive_rebase_against_origin_main_is_blocked() {
    case("git rebase -i origin/main", Expected::Block);
    case("git rebase --interactive origin/main", Expected::Block);
}

#[test]
fn interactive_rebase_against_other_ref_is_allowed() {
    case("git rebase -i HEAD~3", Expected::Allow);
    case("git rebase -i feature/foo", Expected::Allow);
}

#[test]
fn non_interactive_rebase_is_allowed() {
    // The Node helper only blocks the `-i` + `origin/main` combo, not
    // `git rebase origin/main` on its own.
    case("git rebase origin/main", Expected::Allow);
}

#[test]
fn branch_switching_via_checkout_is_blocked() {
    case("git checkout main", Expected::Block);
    case("git checkout -b new-feature", Expected::Block);
    case("git switch main", Expected::Block);
}

#[test]
fn file_level_checkout_with_explicit_separator_is_allowed() {
    // `git checkout <ref> -- <file>` is a file-level operation, not a
    // branch switch. Likewise `--theirs` / `--ours` during merge.
    case("git checkout HEAD -- foo.rs", Expected::Allow);
    case("git checkout --theirs -- foo.rs", Expected::Allow);
    case("git checkout --ours foo.rs", Expected::Allow);
}

#[test]
fn checkout_with_broad_target_is_still_blocked() {
    // `git checkout -- .` and `git checkout -- *` are broad file
    // operations that nuke the working tree — block them even though
    // the explicit separator is present.
    case("git checkout -- .", Expected::Block);
    case("git checkout -- *", Expected::Block);
}

#[test]
fn destructive_branch_subcommand_is_blocked() {
    case("git branch -d old-branch", Expected::Block);
    case("git branch -D old-branch", Expected::Block);
    case("git branch -m new-name", Expected::Block);
    case("git branch feature/foo", Expected::Block);
}

#[test]
fn read_only_branch_subcommand_is_allowed() {
    case("git branch", Expected::Allow); // list
    case("git branch --list", Expected::Allow);
    case("git branch -a", Expected::Allow);
    case("git branch --all", Expected::Allow);
    case("git branch --show-current", Expected::Allow);
    case("git branch -v", Expected::Allow);
    case("git branch --merged", Expected::Allow);
}

#[test]
fn worktree_subcommand_is_blocked() {
    case("git worktree add ../foo main", Expected::Block);
    case("git worktree remove ../foo", Expected::Block);
}

#[test]
fn non_git_command_is_allowed() {
    case("echo hi", Expected::Allow);
    case("ls -la", Expected::Allow);
    case("grep branch foo.txt", Expected::Allow);
}

#[test]
fn adversarial_prefix_does_not_smuggle_blocked_rebase() {
    // The whole point of segmentation: a benign prefix joined with
    // `&&` to a blocked command must not bypass the hook.
    case("echo hello && git rebase -i origin/main", Expected::Block);
    case("git status; git rebase -i origin/main", Expected::Block);
    case("git pull || git branch -D scratch", Expected::Block);
}

/// Issue #4888: the rule keys off the git subcommand, so `checkout` or
/// `switch` appearing anywhere else in the command is not a branch operation.
///
/// The repository's own `crates/gwt/playwright/tests/kill-switch.spec.ts`
/// made every read-only `git` command naming it read as a branch switch — a
/// hyphen is not a word character, so `\bswitch\b` matched inside the
/// filename. Agents could not inspect their own change to that file.
#[test]
fn read_only_git_commands_naming_a_switch_path_are_allowed() {
    case(
        "git diff --stat -- crates/gwt/playwright/tests/kill-switch.spec.ts",
        Expected::Allow,
    );
    case(
        "git log -1 --oneline -- crates/gwt/playwright/tests/kill-switch.spec.ts",
        Expected::Allow,
    );
    case("git show HEAD:src/checkout-flow.ts", Expected::Allow);
    case("git blame src/kill-switch.ts", Expected::Allow);
    case("git status -- switch/", Expected::Allow);
}

/// The same subcommand lookup must still find the real thing behind the global
/// options agents actually use, and must not mistake an option's value for it.
#[test]
fn the_subcommand_is_found_past_global_options() {
    case("git -C /tmp/repo checkout main", Expected::Block);
    case("git -C /tmp/repo switch main", Expected::Block);
    case("git --git-dir /tmp/r/.git checkout main", Expected::Block);
    case("git -c core.pager=cat switch main", Expected::Block);
    // The value of a global option is not the subcommand.
    case("git -C /tmp/switch diff", Expected::Allow);
    case("git -C /tmp/checkout log -1", Expected::Allow);
    // A bare `git`, or one carrying no subcommand, blocks nothing.
    case("git", Expected::Allow);
    case("git --version", Expected::Allow);
}
