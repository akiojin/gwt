---
name: release
description: "Update the version on develop and create a Release PR to main. Use when the user says '/release', 'リリース', 'release PR', or wants to create a new version release."
---

# Release

Update the version and changelog on `develop`, then create or update the Release PR to `main`.

## Instructions

Follow the release flow defined in `.claude/commands/release.md`. Treat that command file as the canonical procedure for branch checks, version classification, approval, file mutation, issue reference collection, and PR creation.

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
6. Update `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md`.
7. Create `chore(release): v{VERSION}` on `develop` and push it.
8. Collect Closing Issues with `scripts/release_issue_refs.py`; keep `gwt-spec` Issues reference-only.
9. Create or update the `develop -> main` Release PR and report the result.
