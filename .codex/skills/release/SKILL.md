---
name: release
description: "Prepare a frozen develop snapshot Release PR to main. Use when the user says '/release', 'リリース', 'release PR', or wants to create a new version release."
---

# Release

Update the version and changelog on `develop`, then release its frozen `release/vX.Y.Z` snapshot to `main`.

## Instructions

Follow `.claude/commands/release.md` as the canonical procedure for workflow dispatch, version classification, approval, recovery, and publication checks.

### Recommended: Prepare Release workflow (any branch, zero friction)

From every worktree, trigger the
GitHub Actions **Prepare Release** workflow (Actions → `Prepare Release` →
`Run workflow`). It checks out `develop` in CI and performs the version bump
(`scripts/compute_release_version.py` latest-tag-relative calc, `cargo
set-version`, `cargo update -w`, git-cliff), the `chore(release): vX.Y.Z`
commit, a create-only `release/vX.Y.Z` ref at that commit, and its Release PR to `main`. Inputs: `bump`
= `auto` (default; a breaking marker caps at minor and is only listed in the
Release PR body, Issue #4373) / `patch` / `minor` / `major`. A major release
happens only when the user chooses `major` explicitly. Approval happens by
reviewing and merging the generated Release PR.
Never create or advance the snapshot branch locally. The workflow owns its materialization.

## Quick Reference

### Flow

```
develop (version and changelog) → release/vX.Y.Z (frozen) → main (PR)
                                                            ↓
                                                   GitHub Release assets
```

### Preconditions

- The repository's Prepare Release workflow is available.
- Release credentials are configured in GitHub Actions.
- At least one unreleased commit exists after the latest `v*` tag.

### Main Steps

1. Use Prepare Release from the current worktree; do not switch branches.
1a. **Arm the release bypass (required, Issue #3267).** A release is ownerless
    chore work, and the workflow-policy owner guard blocks its mutating steps
    (`git fetch`/`git commit`/`cargo update`/`Cargo.toml` edits/`gh run rerun`)
    when no owner Issue/SPEC is linked. Arm a session-scoped bypass with a
    single heredoc using the **literal `gwtd` command name** (variable-style
    invocations may not be recognized by older hooks):
    `{"schema_version":1,"operation":"workflow.bypass","params":{"mode":"release"}}`.
    The bypass auto-expires after 6 hours; re-arm if a long transient-recovery
    loop outlives it. **Always disarm with `{"mode":"off"}` at completion and
    on every abort path.** If the operation is unknown, the local gwtd is too
    old — build `./target/debug/gwtd` or see Issue #3267.
2. Read remote `main`, `develop`, and tags without pulling develop into the current worktree. CI performs the checkout and source mutations.
3. Identify the latest `v*` tag and confirm there are unreleased commits.
4. Classify the next version from commits after the latest tag: `feat` or a breaking marker -> minor, `fix` -> patch, otherwise patch. Never derive major from commits; major only when the user explicitly instructs it (Issue #4373). List breaking-marker commits for the user. Do not use `git-cliff --bumped-version`.
5. Present the computed version, changelog preview, and commit list to the user, then wait for explicit approval.
5a. **Arm a release-completion goal (required, both runtimes).** Right after approval, before mutating files, arm a goal so the flow does not stop at PR creation. Codex: call `create_goal` with the completion condition. Claude Code: inject `/goal <condition>` into your own pane via JSON operation `pane.send` (it cannot self-invoke `/goal`). Condition = "release.yml all jobs success AND GitHub Release v{VERSION} published (draft=false) with all platform assets; rerun transient build failures, report non-transient failures, cap at 60 min / 30 turns." If the goal cannot be armed, print the `/goal` line for the user and continue — step 11 monitoring still runs.
6. Let Prepare Release update `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md`.
7. The workflow pushes `chore(release): v{VERSION}` to `develop`, then atomically creates `release/v{VERSION}`. An existing snapshot is never overwritten.
8. Collect delivered / reference-only Issues with `scripts/release_issue_refs.py` and render the body with `--format pr-body`. The Release PR body is **reference-only**: never write a closing keyword (`Closes` / `Fixes` / `Resolves` + `#N`) in it — `main` is the default branch, so it would close Issues with open acceptance criteria (Issue #3545). Issues are settled by the Issue Monitor on develop merge (Issue #3917).
9. Use the workflow's `release/v{VERSION} -> main` PR and report its frozen head. For an interrupted run with an existing snapshot, use `release.status` with `release_branch:"release/v{VERSION}", base_branch:"main", ensure_release_pr:true`. This dedicated reconcile finds or creates only that snapshot PR; generic current-branch PR creation may fail the verified HEAD gate. Never recreate a moving `develop -> main` PR. **A release is NOT complete at PR creation.**
10. After the PR merges, poll `pr.view` until `[MERGED]`, then find the `release.yml` run (`gh run list --workflow release.yml --branch main`) and poll until it completes.
11. **Monitor, detect errors, and confirm publication (required).** On `release.yml` failure, fetch logs via JSON operation `actions.logs` / `actions.job_logs` (available after the run completes) and classify: transient/infra failures (crates.io download, curl, HTTP2 framing, registry update, runner provisioning) → `actions.rerun` with `failed_only:true` (max 3 retries); non-transient failures (compile/test/clippy/signing) → report and stop. Confirm completion with `gh release view v{VERSION} --json isDraft,assets,publishedAt`: only report "release complete" once `isDraft=false` with all platform assets attached. See `.claude/commands/release.md` step 13 for the full procedure.
12. Disarm the release bypass right after the completion report (and on abort): `{"schema_version":1,"operation":"workflow.bypass","params":{"mode":"off"}}`.
