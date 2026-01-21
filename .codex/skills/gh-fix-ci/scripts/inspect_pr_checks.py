#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Iterable, Sequence

FAILURE_CONCLUSIONS = {
    "failure",
    "cancelled",
    "timed_out",
    "action_required",
}

FAILURE_STATES = {
    "failure",
    "error",
    "cancelled",
    "timed_out",
    "action_required",
}

FAILURE_BUCKETS = {"fail"}

FAILURE_MARKERS = (
    "error",
    "fail",
    "failed",
    "traceback",
    "exception",
    "assert",
    "panic",
    "fatal",
    "timeout",
    "segmentation fault",
)

DEFAULT_MAX_LINES = 160
DEFAULT_CONTEXT_LINES = 30
DEFAULT_MAX_REVIEW_COMMENTS = 50
DEFAULT_MAX_COMMENT_BODY_CHARS = 400
PENDING_LOG_MARKERS = (
    "still in progress",
    "log will be available when it is complete",
)


class GhResult:
    def __init__(self, returncode: int, stdout: str, stderr: str):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr


def run_gh_command(args: Sequence[str], cwd: Path) -> GhResult:
    process = subprocess.run(
        ["gh", *args],
        cwd=cwd,
        text=True,
        capture_output=True,
    )
    return GhResult(process.returncode, process.stdout, process.stderr)


def run_gh_command_raw(args: Sequence[str], cwd: Path) -> tuple[int, bytes, str]:
    process = subprocess.run(
        ["gh", *args],
        cwd=cwd,
        capture_output=True,
    )
    stderr = process.stderr.decode(errors="replace")
    return process.returncode, process.stdout, stderr


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Inspect mergeability, reviewer feedback, and failing GitHub PR checks. "
            "Fetch GitHub Actions logs and extract a failure snippet."
        ),
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument("--repo", default=".", help="Path inside the target Git repository.")
    parser.add_argument(
        "--pr", default=None, help="PR number or URL (defaults to current branch PR)."
    )
    parser.add_argument("--max-lines", type=int, default=DEFAULT_MAX_LINES)
    parser.add_argument("--context", type=int, default=DEFAULT_CONTEXT_LINES)
    parser.add_argument(
        "--max-review-comments",
        type=int,
        default=DEFAULT_MAX_REVIEW_COMMENTS,
        help="Maximum number of review comments to list in text output.",
    )
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text output.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo_root = find_git_root(Path(args.repo))
    if repo_root is None:
        print("Error: not inside a Git repository.", file=sys.stderr)
        return 1

    if not ensure_gh_available(repo_root):
        return 1

    pr_value = resolve_pr(args.pr, repo_root)
    if pr_value is None:
        return 1

    merge_status = fetch_merge_status(pr_value, repo_root)
    merge_conflict = build_merge_conflict_result(merge_status)
    update_required = build_update_branch_result(merge_status)

    checks = fetch_checks(pr_value, repo_root)
    if checks is None:
        return 1

    failing = [c for c in checks if is_failing(c)]

    results = []
    has_blocking = False
    if merge_conflict:
        results.append(merge_conflict)
        has_blocking = True
    if update_required:
        results.append(update_required)
        has_blocking = True

    review_result, review_requires_response = analyze_reviews(
        pr_value,
        repo_root=repo_root,
        max_review_comments=max(1, args.max_review_comments),
    )
    if review_result:
        results.append(review_result)
        if review_requires_response:
            has_blocking = True

    for check in failing:
        has_blocking = True
        results.append(
            analyze_check(
                check,
                repo_root=repo_root,
                max_lines=max(1, args.max_lines),
                context=max(1, args.context),
            )
        )

    if args.json:
        print(
            json.dumps(
                {"pr": pr_value, "mergeStatus": merge_status, "results": results},
                indent=2,
            )
        )
    else:
        render_results(pr_value, results)

    return 1 if has_blocking else 0


def find_git_root(start: Path) -> Path | None:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=start,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        return None
    return Path(result.stdout.strip())


def ensure_gh_available(repo_root: Path) -> bool:
    result = run_gh_command(["auth", "status"], cwd=repo_root)
    if result.returncode == 0:
        return True
    message = (result.stderr or result.stdout or "").strip()
    print(message or "Error: gh not authenticated.", file=sys.stderr)
    return False


def extract_pr_number(pr_value: str) -> str | None:
    if pr_value.isdigit():
        return pr_value
    match = re.search(r"/pull/(\d+)", pr_value)
    if match:
        return match.group(1)
    return None


def resolve_pr(pr_value: str | None, repo_root: Path) -> str | None:
    if pr_value:
        extracted = extract_pr_number(pr_value)
        if extracted:
            return extracted
        result = run_gh_command(["pr", "view", pr_value, "--json", "number"], cwd=repo_root)
        if result.returncode != 0:
            message = (result.stderr or result.stdout or "").strip()
            print(message or "Error: unable to resolve PR.", file=sys.stderr)
            return None
        try:
            data = json.loads(result.stdout or "{}")
        except json.JSONDecodeError:
            print("Error: unable to parse PR JSON.", file=sys.stderr)
            return None
        number = data.get("number")
        if not number:
            print("Error: no PR number found.", file=sys.stderr)
            return None
        return str(number)
    result = run_gh_command(["pr", "view", "--json", "number"], cwd=repo_root)
    if result.returncode != 0:
        message = (result.stderr or result.stdout or "").strip()
        print(message or "Error: unable to resolve PR.", file=sys.stderr)
        return None
    try:
        data = json.loads(result.stdout or "{}")
    except json.JSONDecodeError:
        print("Error: unable to parse PR JSON.", file=sys.stderr)
        return None
    number = data.get("number")
    if not number:
        print("Error: no PR number found.", file=sys.stderr)
        return None
    return str(number)


def fetch_merge_status(pr_value: str, repo_root: Path) -> dict[str, Any] | None:
    primary_fields = ["mergeable", "mergeStateStatus", "url", "baseRefName", "headRefName"]
    result = run_gh_command(
        ["pr", "view", pr_value, "--json", ",".join(primary_fields)],
        cwd=repo_root,
    )
    if result.returncode != 0:
        fallback_fields = ["mergeable", "mergeStateStatus", "url"]
        result = run_gh_command(
            ["pr", "view", pr_value, "--json", ",".join(fallback_fields)],
            cwd=repo_root,
        )
        if result.returncode != 0:
            return None
    try:
        data = json.loads(result.stdout or "{}")
    except json.JSONDecodeError:
        return None
    if not isinstance(data, dict):
        return None
    return data


def build_merge_conflict_result(
    merge_status: dict[str, Any] | None,
) -> dict[str, Any] | None:
    if not merge_status:
        return None
    mergeable = normalize_field(merge_status.get("mergeable"))
    merge_state = normalize_field(merge_status.get("mergeStateStatus"))
    if mergeable != "conflicting" and merge_state != "dirty":
        return None
    return {
        "name": "Merge conflicts",
        "status": "conflict",
        "detailsUrl": merge_status.get("url", ""),
        "mergeable": merge_status.get("mergeable"),
        "mergeStateStatus": merge_status.get("mergeStateStatus"),
        "baseRefName": merge_status.get("baseRefName"),
        "headRefName": merge_status.get("headRefName"),
        "note": "PR has merge conflicts and cannot be merged as-is.",
    }


def build_update_branch_result(
    merge_status: dict[str, Any] | None,
) -> dict[str, Any] | None:
    if not merge_status:
        return None
    merge_state = normalize_field(merge_status.get("mergeStateStatus"))
    if merge_state != "behind":
        return None
    base_ref = merge_status.get("baseRefName")
    head_ref = merge_status.get("headRefName")
    base_label = f" '{base_ref}'" if base_ref else ""
    head_label = f" '{head_ref}'" if head_ref else ""
    note = (
        f"PR branch{head_label} is behind base{base_label}; update the branch before re-running checks."
    )
    return {
        "name": "Update branch required",
        "status": "behind",
        "detailsUrl": merge_status.get("url", ""),
        "mergeable": merge_status.get("mergeable"),
        "mergeStateStatus": merge_status.get("mergeStateStatus"),
        "baseRefName": base_ref,
        "headRefName": head_ref,
        "note": note,
    }


def analyze_reviews(
    pr_value: str,
    repo_root: Path,
    max_review_comments: int,
) -> tuple[dict[str, Any] | None, bool]:
    repo_slug = fetch_repo_slug(repo_root)
    if not repo_slug:
        return None, False

    pr_data, pr_error = gh_api_get_json(f"/repos/{repo_slug}/pulls/{pr_value}", repo_root)
    if pr_error or not isinstance(pr_data, dict):
        return (
            {
                "name": "Reviewer comments",
                "status": "review_unavailable",
                "detailsUrl": "",
                "error": pr_error or "Error: unable to fetch PR metadata.",
            },
            False,
        )

    author_login = str(pr_data.get("user", {}).get("login") or "")
    pr_url = pr_data.get("html_url") or pr_data.get("url") or ""

    errors: list[str] = []
    reviews_data, reviews_error = gh_api_get_json(
        f"/repos/{repo_slug}/pulls/{pr_value}/reviews?per_page=100",
        repo_root,
    )
    if reviews_error:
        errors.append(f"Reviews: {reviews_error}")
        reviews_data = []
    if not isinstance(reviews_data, list):
        errors.append("Reviews: unexpected response.")
        reviews_data = []

    review_comments_data, review_comments_error = gh_api_get_json(
        f"/repos/{repo_slug}/pulls/{pr_value}/comments?per_page=100",
        repo_root,
    )
    if review_comments_error:
        errors.append(f"Review comments: {review_comments_error}")
        review_comments_data = []
    if not isinstance(review_comments_data, list):
        errors.append("Review comments: unexpected response.")
        review_comments_data = []

    issue_comments_data, issue_comments_error = gh_api_get_json(
        f"/repos/{repo_slug}/issues/{pr_value}/comments?per_page=100",
        repo_root,
    )
    if issue_comments_error:
        errors.append(f"Issue comments: {issue_comments_error}")
        issue_comments_data = []
    if not isinstance(issue_comments_data, list):
        errors.append("Issue comments: unexpected response.")
        issue_comments_data = []

    author_key = author_login.lower()

    review_items: list[dict[str, Any]] = []
    for review in reviews_data:
        if not isinstance(review, dict):
            continue
        user_login = str(review.get("user", {}).get("login") or "")
        if not user_login or user_login.lower() == author_key:
            continue
        state = normalize_field(review.get("state"))
        if not state or state == "pending":
            continue
        review_items.append(
            {
                "author": user_login,
                "state": state.upper(),
                "submittedAt": review.get("submitted_at") or "",
                "url": review.get("html_url") or review.get("url") or "",
                "body": compact_text(review.get("body") or "", DEFAULT_MAX_COMMENT_BODY_CHARS),
            }
        )

    latest_by_reviewer: dict[str, dict[str, Any]] = {}
    for item in review_items:
        reviewer_key = str(item.get("author", "")).lower()
        if not reviewer_key:
            continue
        current = latest_by_reviewer.get(reviewer_key)
        if current is None or str(item.get("submittedAt") or "") > str(current.get("submittedAt") or ""):
            latest_by_reviewer[reviewer_key] = item

    latest_reviews = sorted(
        latest_by_reviewer.values(),
        key=lambda entry: str(entry.get("submittedAt") or ""),
        reverse=True,
    )

    review_comments: list[dict[str, Any]] = []
    for comment in review_comments_data:
        if not isinstance(comment, dict):
            continue
        user_login = str(comment.get("user", {}).get("login") or "")
        if not user_login or user_login.lower() == author_key:
            continue
        review_comments.append(
            {
                "author": user_login,
                "path": comment.get("path") or "",
                "line": comment.get("line") or comment.get("original_line") or comment.get("position"),
                "createdAt": comment.get("created_at") or "",
                "url": comment.get("html_url") or comment.get("url") or "",
                "body": compact_text(comment.get("body") or "", DEFAULT_MAX_COMMENT_BODY_CHARS),
            }
        )

    issue_comments: list[dict[str, Any]] = []
    for comment in issue_comments_data:
        if not isinstance(comment, dict):
            continue
        user_login = str(comment.get("user", {}).get("login") or "")
        if not user_login or user_login.lower() == author_key:
            continue
        issue_comments.append(
            {
                "author": user_login,
                "createdAt": comment.get("created_at") or "",
                "url": comment.get("html_url") or comment.get("url") or "",
                "body": compact_text(comment.get("body") or "", DEFAULT_MAX_COMMENT_BODY_CHARS),
            }
        )

    decision = "PENDING"
    states = [normalize_field(item.get("state")) for item in latest_reviews]
    if any(state == "changes_requested" for state in states):
        decision = "CHANGES_REQUESTED"
    elif any(state == "approved" for state in states):
        decision = "APPROVED"
    elif any(state == "commented" for state in states):
        decision = "COMMENTED"
    elif latest_reviews:
        decision = "REVIEWED"

    summary_comments = any(
        normalize_field(item.get("state")) == "commented" and item.get("body")
        for item in latest_reviews
    )
    action_required = bool(
        decision == "CHANGES_REQUESTED"
        or summary_comments
        or review_comments
        or issue_comments
    )

    review_result: dict[str, Any] = {
        "name": "Reviewer comments",
        "status": "review",
        "detailsUrl": pr_url,
        "reviewDecision": decision,
        "reviewActionRequired": action_required,
        "reviewCounts": {
            "reviews": len(review_items),
            "reviewers": len(latest_reviews),
            "reviewComments": len(review_comments),
            "issueComments": len(issue_comments),
        },
        "latestReviews": latest_reviews[:max_review_comments],
        "reviewComments": review_comments[:max_review_comments],
        "issueComments": issue_comments[:max_review_comments],
    }

    if errors:
        review_result["error"] = "; ".join(errors)

    truncated_notes = []
    if len(latest_reviews) > max_review_comments:
        truncated_notes.append(
            f"Showing {max_review_comments} of {len(latest_reviews)} latest reviews."
        )
    if len(review_comments) > max_review_comments:
        truncated_notes.append(
            f"Showing {max_review_comments} of {len(review_comments)} inline review comments."
        )
    if len(issue_comments) > max_review_comments:
        truncated_notes.append(
            f"Showing {max_review_comments} of {len(issue_comments)} issue comments."
        )
    if truncated_notes:
        review_result["note"] = " ".join(truncated_notes)

    return review_result, action_required


def fetch_checks(pr_value: str, repo_root: Path) -> list[dict[str, Any]] | None:
    primary_fields = ["name", "state", "conclusion", "detailsUrl", "startedAt", "completedAt"]
    result = run_gh_command(
        ["pr", "checks", pr_value, "--json", ",".join(primary_fields)],
        cwd=repo_root,
    )
    if result.returncode != 0:
        message = "\n".join(filter(None, [result.stderr, result.stdout])).strip()
        available_fields = parse_available_fields(message)
        if available_fields:
            fallback_fields = [
                "name",
                "state",
                "bucket",
                "link",
                "startedAt",
                "completedAt",
                "workflow",
            ]
            selected_fields = [field for field in fallback_fields if field in available_fields]
            if not selected_fields:
                print("Error: no usable fields available for gh pr checks.", file=sys.stderr)
                return None
            result = run_gh_command(
                ["pr", "checks", pr_value, "--json", ",".join(selected_fields)],
                cwd=repo_root,
            )
            if result.returncode != 0:
                message = (result.stderr or result.stdout or "").strip()
                print(message or "Error: gh pr checks failed.", file=sys.stderr)
                return None
        else:
            print(message or "Error: gh pr checks failed.", file=sys.stderr)
            return None
    try:
        data = json.loads(result.stdout or "[]")
    except json.JSONDecodeError:
        print("Error: unable to parse checks JSON.", file=sys.stderr)
        return None
    if not isinstance(data, list):
        print("Error: unexpected checks JSON shape.", file=sys.stderr)
        return None
    return data


def is_failing(check: dict[str, Any]) -> bool:
    conclusion = normalize_field(check.get("conclusion"))
    if conclusion in FAILURE_CONCLUSIONS:
        return True
    state = normalize_field(check.get("state") or check.get("status"))
    if state in FAILURE_STATES:
        return True
    bucket = normalize_field(check.get("bucket"))
    return bucket in FAILURE_BUCKETS


def analyze_check(
    check: dict[str, Any],
    repo_root: Path,
    max_lines: int,
    context: int,
) -> dict[str, Any]:
    url = check.get("detailsUrl") or check.get("link") or ""
    run_id = extract_run_id(url)
    job_id = extract_job_id(url)
    base: dict[str, Any] = {
        "name": check.get("name", ""),
        "detailsUrl": url,
        "runId": run_id,
        "jobId": job_id,
    }

    if run_id is None:
        base["status"] = "external"
        base["note"] = "No GitHub Actions run id detected in detailsUrl."
        return base

    metadata = fetch_run_metadata(run_id, repo_root)
    log_text, log_error, log_status = fetch_check_log(
        run_id=run_id,
        job_id=job_id,
        repo_root=repo_root,
    )

    if log_status == "pending":
        base["status"] = "log_pending"
        base["note"] = log_error or "Logs are not available yet."
        if metadata:
            base["run"] = metadata
        return base

    if log_error:
        base["status"] = "log_unavailable"
        base["error"] = log_error
        if metadata:
            base["run"] = metadata
        return base

    snippet = extract_failure_snippet(log_text, max_lines=max_lines, context=context)
    base["status"] = "ok"
    base["run"] = metadata or {}
    base["logSnippet"] = snippet
    base["logTail"] = tail_lines(log_text, max_lines)
    return base


def extract_run_id(url: str) -> str | None:
    if not url:
        return None
    for pattern in (r"/actions/runs/(\d+)", r"/runs/(\d+)"):
        match = re.search(pattern, url)
        if match:
            return match.group(1)
    return None


def extract_job_id(url: str) -> str | None:
    if not url:
        return None
    match = re.search(r"/actions/runs/\d+/job/(\d+)", url)
    if match:
        return match.group(1)
    match = re.search(r"/job/(\d+)", url)
    if match:
        return match.group(1)
    return None


def fetch_run_metadata(run_id: str, repo_root: Path) -> dict[str, Any] | None:
    fields = [
        "conclusion",
        "status",
        "workflowName",
        "name",
        "event",
        "headBranch",
        "headSha",
        "url",
    ]
    result = run_gh_command(["run", "view", run_id, "--json", ",".join(fields)], cwd=repo_root)
    if result.returncode != 0:
        return None
    try:
        data = json.loads(result.stdout or "{}")
    except json.JSONDecodeError:
        return None
    if not isinstance(data, dict):
        return None
    return data


def fetch_check_log(
    run_id: str,
    job_id: str | None,
    repo_root: Path,
) -> tuple[str, str, str]:
    log_text, log_error = fetch_run_log(run_id, repo_root)
    if not log_error:
        return log_text, "", "ok"

    if is_log_pending_message(log_error) and job_id:
        job_log, job_error = fetch_job_log(job_id, repo_root)
        if job_log:
            return job_log, "", "ok"
        if job_error and is_log_pending_message(job_error):
            return "", job_error, "pending"
        if job_error:
            return "", job_error, "error"
        return "", log_error, "pending"

    if is_log_pending_message(log_error):
        return "", log_error, "pending"

    return "", log_error, "error"


def fetch_run_log(run_id: str, repo_root: Path) -> tuple[str, str]:
    result = run_gh_command(["run", "view", run_id, "--log"], cwd=repo_root)
    if result.returncode != 0:
        error = (result.stderr or result.stdout or "").strip()
        return "", error or "gh run view failed"
    return result.stdout, ""


def fetch_job_log(job_id: str, repo_root: Path) -> tuple[str, str]:
    repo_slug = fetch_repo_slug(repo_root)
    if not repo_slug:
        return "", "Error: unable to resolve repository name for job logs."
    endpoint = f"/repos/{repo_slug}/actions/jobs/{job_id}/logs"
    returncode, stdout_bytes, stderr = run_gh_command_raw(["api", endpoint], cwd=repo_root)
    if returncode != 0:
        message = (stderr or stdout_bytes.decode(errors="replace")).strip()
        return "", message or "gh api job logs failed"
    if is_zip_payload(stdout_bytes):
        return "", "Job logs returned a zip archive; unable to parse."
    return stdout_bytes.decode(errors="replace"), ""


def fetch_repo_slug(repo_root: Path) -> str | None:
    result = run_gh_command(["repo", "view", "--json", "nameWithOwner"], cwd=repo_root)
    if result.returncode != 0:
        return None
    try:
        data = json.loads(result.stdout or "{}")
    except json.JSONDecodeError:
        return None
    name_with_owner = data.get("nameWithOwner")
    if not name_with_owner:
        return None
    return str(name_with_owner)


def gh_api_get_json(endpoint: str, repo_root: Path) -> tuple[Any, str]:
    result = run_gh_command(["api", endpoint], cwd=repo_root)
    if result.returncode != 0:
        message = (result.stderr or result.stdout or "").strip()
        return None, message or f"gh api {endpoint} failed"
    try:
        data = json.loads(result.stdout or "null")
    except json.JSONDecodeError:
        return None, "Error: unable to parse gh api JSON."
    return data, ""


def compact_text(text: str, max_chars: int) -> str:
    if not text:
        return ""
    cleaned = " ".join(str(text).split())
    if len(cleaned) <= max_chars:
        return cleaned
    trimmed = cleaned[: max(0, max_chars - 3)].rstrip()
    return f"{trimmed}..."


def normalize_field(value: Any) -> str:
    if value is None:
        return ""
    return str(value).strip().lower()


def parse_available_fields(message: str) -> list[str]:
    if "Available fields:" not in message:
        return []
    fields: list[str] = []
    collecting = False
    for line in message.splitlines():
        if "Available fields:" in line:
            collecting = True
            continue
        if not collecting:
            continue
        field = line.strip()
        if not field:
            continue
        fields.append(field)
    return fields


def is_log_pending_message(message: str) -> bool:
    lowered = message.lower()
    return any(marker in lowered for marker in PENDING_LOG_MARKERS)


def is_zip_payload(payload: bytes) -> bool:
    return payload.startswith(b"PK")


def extract_failure_snippet(log_text: str, max_lines: int, context: int) -> str:
    lines = log_text.splitlines()
    if not lines:
        return ""

    marker_index = find_failure_index(lines)
    if marker_index is None:
        return "\n".join(lines[-max_lines:])

    start = max(0, marker_index - context)
    end = min(len(lines), marker_index + context)
    window = lines[start:end]
    if len(window) > max_lines:
        window = window[-max_lines:]
    return "\n".join(window)


def find_failure_index(lines: Sequence[str]) -> int | None:
    for idx in range(len(lines) - 1, -1, -1):
        lowered = lines[idx].lower()
        if any(marker in lowered for marker in FAILURE_MARKERS):
            return idx
    return None


def tail_lines(text: str, max_lines: int) -> str:
    if max_lines <= 0:
        return ""
    lines = text.splitlines()
    return "\n".join(lines[-max_lines:])


def render_results(pr_number: str, results: Iterable[dict[str, Any]]) -> None:
    results_list = list(results)
    print(f"PR #{pr_number}: {len(results_list)} items analyzed.")
    for result in results_list:
        print("-" * 60)
        print(f"Check: {result.get('name', '')}")
        if result.get("detailsUrl"):
            print(f"Details: {result['detailsUrl']}")
        run_id = result.get("runId")
        if run_id:
            print(f"Run ID: {run_id}")
        job_id = result.get("jobId")
        if job_id:
            print(f"Job ID: {job_id}")
        status = result.get("status", "unknown")
        print(f"Status: {status}")

        if str(status).startswith("review"):
            render_review_result(result)
            continue

        run_meta = result.get("run", {})
        if run_meta:
            branch = run_meta.get("headBranch", "")
            sha = (run_meta.get("headSha") or "")[:12]
            workflow = run_meta.get("workflowName") or run_meta.get("name") or ""
            conclusion = run_meta.get("conclusion") or run_meta.get("status") or ""
            print(f"Workflow: {workflow} ({conclusion})")
            if branch or sha:
                print(f"Branch/SHA: {branch} {sha}")
            if run_meta.get("url"):
                print(f"Run URL: {run_meta['url']}")

        if result.get("note"):
            print(f"Note: {result['note']}")

        if result.get("error"):
            print(f"Error fetching logs: {result['error']}")
            continue

        snippet = result.get("logSnippet") or ""
        if snippet:
            print("Failure snippet:")
            print(indent_block(snippet, prefix="  "))
        else:
            print("No snippet available.")
    print("-" * 60)


def render_review_result(result: dict[str, Any]) -> None:
    decision = result.get("reviewDecision", "")
    action_required = result.get("reviewActionRequired")
    if decision:
        print(f"Review decision: {decision}")
    if action_required is not None:
        print(f"Action required: {'yes' if action_required else 'no'}")

    counts = result.get("reviewCounts", {})
    if counts:
        print(
            "Counts: "
            f"reviews {counts.get('reviews', 0)}, "
            f"reviewers {counts.get('reviewers', 0)}, "
            f"review comments {counts.get('reviewComments', 0)}, "
            f"issue comments {counts.get('issueComments', 0)}"
        )

    if result.get("note"):
        print(f"Note: {result['note']}")

    if result.get("error"):
        print(f"Error: {result['error']}")

    latest_reviews = result.get("latestReviews") or []
    if latest_reviews:
        print("Latest reviews:")
        for review in latest_reviews:
            print(f"  - {format_review_line(review)}")

    review_comments = result.get("reviewComments") or []
    if review_comments:
        print("Inline review comments:")
        for comment in review_comments:
            print(f"  - {format_comment_line(comment, include_path=True)}")

    issue_comments = result.get("issueComments") or []
    if issue_comments:
        print("Issue comments:")
        for comment in issue_comments:
            print(f"  - {format_comment_line(comment, include_path=False)}")


def format_review_line(review: dict[str, Any]) -> str:
    author = review.get("author", "")
    state = review.get("state", "")
    submitted = review.get("submittedAt", "")
    url = review.get("url", "")
    body = review.get("body", "")
    parts = [part for part in [author, state, submitted, url] if part]
    line = " ".join(str(part) for part in parts)
    if body:
        line = f"{line} - {body}" if line else body
    return line


def format_comment_line(comment: dict[str, Any], include_path: bool) -> str:
    author = comment.get("author", "")
    created = comment.get("createdAt", "")
    url = comment.get("url", "")
    body = comment.get("body", "")
    location = ""
    if include_path:
        path = comment.get("path") or ""
        line = comment.get("line")
        if path or line:
            line_value = "" if line is None else str(line)
            location = f"{path}:{line_value}" if line_value else path
    parts = [part for part in [author, location, created, url] if part]
    line_text = " ".join(str(part) for part in parts)
    if body:
        line_text = f"{line_text} - {body}" if line_text else body
    return line_text


def indent_block(text: str, prefix: str = "  ") -> str:
    return "\n".join(f"{prefix}{line}" for line in text.splitlines())


if __name__ == "__main__":
    raise SystemExit(main())
