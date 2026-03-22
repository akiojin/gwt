---
description: Create or update GitHub PRs with the gh CLI using the gwt-pr skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GitHub PR Command

Use this command to draft or update a GitHub PR with the gh CLI.

## Usage

```
/gwt:gwt-pr [optional context]
```

## Steps

1. Load `skills/gwt-pr/SKILL.md` and follow the workflow.
2. Ensure `gh auth status` succeeds before running PR commands.
3. Run the local working tree preflight from the skill (`git status --porcelain`); if changes exist, confirm with the user before push/PR operations.
4. Run the branch sync preflight from the skill (`git rev-list --left-right --count "HEAD...origin/$base"`); if the branch is behind, merge `origin/$base` into the current branch and push before PR creation.
5. If that merge produces conflicts, inspect the conflict carefully and only ask the user when it cannot be resolved with high confidence.
6. Use REST list/view endpoints as the primary transport: `gh api repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>&per_page=100` for lookup and `gh api repos/<owner>/<repo>/pulls/<number>` for the final URL.
7. When all PRs for the head are merged, validate merge commit ancestry before counting post-merge commits.
8. If the merge commit is missing or not an ancestor of `HEAD`, compare `origin/<head>..HEAD` first and then `origin/<base>..HEAD` before concluding `NO ACTION`.
9. If any post-merge count is greater than `0`, verify `git diff --quiet origin/<base>...HEAD --` before creating a PR.
10. If the base compare is empty, return `NO ACTION`; merged base commits alone do not justify a new PR.
11. If both upstream and base comparisons fail, stop with `MANUAL CHECK`; do not create a PR by guesswork.
12. Generate or update the PR body using the provided templates.
13. Create or update PRs through the REST pull-request endpoint first; use `gh pr create` / `gh pr edit` only as fallback.

## Examples

```
/gwt:gwt-pr create draft for current branch
```

```
/gwt:gwt-pr update PR body only
```
