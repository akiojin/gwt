---
name: release
description: "Update the version on develop and create a Release PR to main. Use when the user says '/release', 'リリース', 'release PR', or wants to create a new version release."
---

# Release

Update the version and changelog on `develop`, then create or update the Release PR to `main`.

## Instructions

Follow the release flow defined in `.claude/commands/release.md`. Treat that command file as the canonical procedure for branch checks, version classification, approval, file mutation, issue reference collection, and PR creation.

### Recommended: Prepare Release workflow (any branch, zero friction)

When you cannot or do not want to switch to `develop` (e.g. running inside a
work worktree), do NOT run the manual steps locally. Instead trigger the
GitHub Actions **Prepare Release** workflow (Actions → `Prepare Release` →
`Run workflow`). It checks out `develop` in CI and performs the version bump
(`scripts/compute_release_version.py` latest-tag-relative calc, `cargo
set-version`, `cargo update -w`, git-cliff), the `chore(release): vX.Y.Z`
commit, and the `develop -> main` Release PR. Inputs: `bump`
= `auto` (default; fails on breaking so major is explicit) / `patch` / `minor`
/ `major`. Approval happens by reviewing and merging the generated Release PR.
The manual steps below are a fallback for interactive runs on `develop`.

## Quick Reference

### Flow

```
develop (バージョン更新・CHANGELOG更新) → main (PR)
                                            ↓
                                  GitHub Release assets (自動)
```

### Preconditions

- Work is running on the `develop` branch.
- `git-cliff` is installed.
- GitHub authentication is available.
- At least one unreleased commit exists after the latest `v*` tag.

### Main Steps

1. Confirm the current branch is `develop`.
2. Fetch `origin/main`, `origin/develop`, and tags, then pull `origin/develop`.
3. Identify the latest `v*` tag and confirm there are unreleased commits.
4. Classify the next version from commits after the latest tag: breaking change -> major, `feat` -> minor, `fix` -> patch, otherwise patch. Do not use `git-cliff --bumped-version`.
5. Present the computed version, changelog preview, and commit list to the user, then wait for explicit approval.
5a. **Arm a release-completion goal (required, both runtimes).** Right after approval, before mutating files, arm a goal so the flow does not stop at PR creation. Codex: call `create_goal` with the completion condition. Claude Code: inject `/goal <condition>` into your own pane via JSON operation `pane.send` (it cannot self-invoke `/goal`). Condition = "release.yml all jobs success AND GitHub Release v{VERSION} published (draft=false) with all platform assets; rerun transient build failures, report non-transient failures, cap at 60 min / 30 turns." If the goal cannot be armed, print the `/goal` line for the user and continue — step 11 monitoring still runs.
6. Update `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md`.
7. Create `chore(release): v{VERSION}` on `develop` and push it.
8. Collect Closing Issues with `scripts/release_issue_refs.py`; keep `gwt-spec` Issues reference-only.
9. Create or update the `develop -> main` Release PR and report the result. **A release is NOT complete at PR creation.**
10. After the PR merges, poll `pr.view` until `[MERGED]`, then find the `release.yml` run (`gh run list --workflow release.yml --branch main`) and poll until it completes.
11. **Monitor, detect errors, and confirm publication (required).** On `release.yml` failure, fetch logs via JSON operation `actions.logs` / `actions.job_logs` (available after the run completes) and classify: transient/infra failures (crates.io download, curl, HTTP2 framing, registry update, runner provisioning) → `gh run rerun <run-id> --failed` (max 3 retries); non-transient failures (compile/test/clippy/signing) → report and stop. Confirm completion with `gh release view v{VERSION} --json isDraft,assets,publishedAt` (`gh release` is not blocked): only report "release complete" once `isDraft=false` with all platform assets attached. See `.claude/commands/release.md` step 13 for the full procedure.
