---
name: gh-fix-ci
description: Inspect GitHub PR checks with gh, pull failing GitHub Actions logs, summarize failure context, then create a fix plan and implement after user approval. Use when a user asks to debug or fix failing PR CI/CD checks on GitHub Actions and wants a plan + code changes; for external checks (e.g., Buildkite), only report the details URL and mark them out of scope.
metadata:
  short-description: Fix failing Github CI actions
---

# Gh Pr Checks Plan Fix

## Overview

Use gh to locate reviewer change requests and failing PR checks, fetch GitHub Actions logs for actionable failures, summarize the failure snippet, then propose a fix plan and implement after explicit approval.
- Depends on the `plan` skill for drafting and approving the fix plan.

Prereq: ensure `gh` is authenticated (for example, run `gh auth login` once), then run `gh auth status` with escalated permissions (include workflow/repo scopes) so `gh` commands succeed. If sandboxing blocks `gh auth status`, rerun it with `sandbox_permissions=require_escalated`.

## Inputs

- `repo`: path inside the repo (default `.`)
- `pr`: PR number or URL (optional; defaults to current branch PR)
- `gh` authentication for the repo host

## Quick start

- `python "<path-to-skill>/scripts/inspect_pr_checks.py" --repo "." --pr "<number-or-url>"`
- Add `--json` if you want machine-friendly output for summarization.

## Workflow

1. Verify gh authentication.
   - Run `gh auth status` in the repo with escalated scopes (workflow/repo) after running `gh auth login`.
   - If sandboxed auth status fails, rerun the command with `sandbox_permissions=require_escalated` to allow network/keyring access.
   - If unauthenticated, ask the user to log in before proceeding.
2. Resolve the PR.
   - Prefer the current branch PR: `gh pr view --json number,url`.
   - If the user provides a PR number or URL, use that directly.
3. Check review status (Change Requests).
   - `gh pr view <pr> --json reviewDecision,reviews`
   - If `reviewDecision` is `CHANGES_REQUESTED`, capture the reviewers and their review bodies for the summary.
   - Useful filter for reviewers who requested changes:
     - `gh pr view <pr> --json reviews --jq '.reviews[] | select(.state=="CHANGES_REQUESTED") | {author: .author.login, submittedAt: .submittedAt, body: .body}'`
   - If JSON fields are rejected or incomplete, fall back to the API:
     - `gh api "/repos/<owner>/<repo>/pulls/<pr>/reviews"`
     - `gh api "/repos/<owner>/<repo>/pulls/<pr>/comments"` (inline review comments)
4. Inspect failing checks (GitHub Actions only).
   - Preferred: run the bundled script (handles gh field drift and job-log fallbacks):
     - `python "<path-to-skill>/scripts/inspect_pr_checks.py" --repo "." --pr "<number-or-url>"`
     - Add `--json` for machine-friendly output.
   - Manual fallback:
     - `gh pr checks <pr> --json name,state,bucket,link,startedAt,completedAt,workflow`
       - If a field is rejected, rerun with the available fields reported by `gh`.
     - For each failing check, extract the run id from `detailsUrl` and run:
       - `gh run view <run_id> --json name,workflowName,conclusion,status,url,event,headBranch,headSha`
       - `gh run view <run_id> --log`
     - If the run log says it is still in progress, fetch job logs directly:
       - `gh api "/repos/<owner>/<repo>/actions/jobs/<job_id>/logs" > "<path>"`
5. Scope non-GitHub Actions checks.
   - If `detailsUrl` is not a GitHub Actions run, label it as external and only report the URL.
   - Do not attempt Buildkite or other providers; keep the workflow lean.
6. Summarize failures for the user.
   - Provide change request status (if any), reviewers requesting changes, and key review snippets.
   - Provide the failing check name, run URL (if any), and a concise log snippet.
   - Call out missing logs explicitly.
7. Create a plan.
   - Use the `plan` skill to draft a concise plan and request approval.
8. Implement after approval.
   - Apply the approved plan, summarize diffs/tests, and ask about opening a PR.
9. Recheck status.
   - After changes, suggest re-running the relevant tests, `gh pr checks`, and re-checking review status to confirm.

## Bundled Resources

### scripts/inspect_pr_checks.py

Fetch failing PR checks, pull GitHub Actions logs, and extract a failure snippet. Exits non-zero when failures remain so it can be used in automation.

Usage examples:
- `python "<path-to-skill>/scripts/inspect_pr_checks.py" --repo "." --pr "123"`
- `python "<path-to-skill>/scripts/inspect_pr_checks.py" --repo "." --pr "https://github.com/org/repo/pull/123" --json`
- `python "<path-to-skill>/scripts/inspect_pr_checks.py" --repo "." --max-lines 200 --context 40`
