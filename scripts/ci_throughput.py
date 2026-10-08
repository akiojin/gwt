#!/usr/bin/env python3
"""Measure merged-PR CI throughput using read-only GitHub data or a saved snapshot."""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import statistics
import subprocess
import sys
from urllib.parse import urlencode


def timestamp(value):
    result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if result.tzinfo is None:
        raise ValueError("timestamps must include a timezone")
    return result


def minutes(start, end):
    elapsed = (timestamp(end) - timestamp(start)).total_seconds() / 60
    if elapsed < 0:
        raise ValueError(f"negative duration: {start} -> {end}")
    return elapsed


def distribution(values):
    ordered = sorted(values)
    return {"count": len(ordered),
            "p50": statistics.median(ordered) if ordered else None,
            "p90": ordered[math.ceil(len(ordered) * .9) - 1] if ordered else None,
            "max": max(ordered) if ordered else None}


def summarize(snapshot):
    if snapshot["schema_version"] != 1:
        raise ValueError("unsupported snapshot schema_version")
    prs = snapshot["pull_requests"]
    if not prs:
        raise ValueError("no merged PRs in snapshot")
    cycles, waits, required_waits = [], [], []
    durations, job_waits = defaultdict(list), defaultdict(list)
    excluded, missing, run_ids, unavailable_waits = Counter(), [], [], []
    required = set(snapshot.get("required_check_names", []))
    for pr in prs:
        successful = []
        for run in pr["runs"]:
            if run["status"] == "completed" and run["conclusion"] == "success":
                successful.append(run)
            else:
                excluded[run.get("conclusion") or run["status"]] += 1
        if not successful:
            missing.append(pr["number"])
            continue
        run = max(successful, key=lambda item: (timestamp(item["run_started_at"]), item["id"]))
        cycles.append(minutes(run["run_started_at"], run["updated_at"]))
        run_ids.append({"pr": pr["number"], "run": run["id"], "attempt": run["run_attempt"]})
        for job in run["jobs"]:
            if job["conclusion"] in ("skipped", "cancelled") or not job.get("completed_at"):
                continue
            if job.get("started_at"):
                durations[job["name"]].append(minutes(job["started_at"], job["completed_at"]))
                if job.get("created_at") and timestamp(job["created_at"]) <= timestamp(job["started_at"]):
                    wait = minutes(job["created_at"], job["started_at"])
                    waits.append(wait)
                    job_waits[job["name"]].append(wait)
                    if job["name"] in required:
                        required_waits.append(wait)
                else:
                    # GitHub reruns can refresh created_at on reused predecessor jobs
                    # while retaining their earlier started_at/completed_at timestamps.
                    unavailable_waits.append(job["id"])
    return {
        "repo": snapshot["repo"], "base": snapshot["base"], "before": snapshot["before"],
        "workflow": snapshot.get("workflow", "test.yml"), "pr_count": len(prs),
        "pr_numbers": [pr["number"] for pr in prs], "selected_runs": run_ids,
        "method": {"pr_selection": "Latest N merged_at values at or before the UTC cutoff",
                   "cycle": "Latest successful final-head workflow attempt: run_started_at -> updated_at",
                   "job": "started_at -> completed_at; skipped/cancelled jobs excluded",
                   "runner_wait": "job created_at -> started_at (after dependency scheduling)",
                   "base_sync": "First-parent PR merges whose second parent is on the base's first-parent history",
                   "historical_sync": "Same base syncs with subjects starting 'Merge ' and containing the base name",
                   "percentiles": "p50 median; p90 nearest rank; all durations in minutes"},
        "pr_created_to_merged_minutes": distribution(minutes(pr["created_at"], pr["merged_at"]) for pr in prs),
        "workflow_cycle_minutes": distribution(cycles),
        "base_syncs_per_merge": (sum(len(pr["sync_commits"]) for pr in prs) / len(prs)
                                 if all(pr.get("sync_commits") is not None for pr in prs) else None),
        "historical_message_filter_syncs_per_merge": (
            sum(len(pr["conventional_sync_commits"]) for pr in prs) / len(prs)
            if all(pr.get("conventional_sync_commits") is not None for pr in prs) else None),
        "runner_wait_minutes": distribution(waits),
        "required_runner_wait_minutes": distribution(required_waits),
        "unavailable_runner_wait_jobs": unavailable_waits,
        "jobs": {name: {"duration_minutes": distribution(values),
                        "runner_wait_minutes": distribution(job_waits[name])}
                 for name, values in sorted(durations.items())},
        "excluded_runs": dict(sorted(excluded.items())), "missing_workflow_prs": missing,
    }


def api(repo, endpoint, **params):
    query = "?" + urlencode(params) if params else ""
    output = subprocess.check_output(
        ["gh", "api", "--method", "GET", f"repos/{repo}/{endpoint}{query}"],
        text=True, encoding="utf-8")
    return json.loads(output)


def merged_prs(repo, base, limit, before, page_size=100):
    selected, page = [], 1
    cutoff = timestamp(before)
    while True:
        rows = api(repo, "pulls", state="closed", base=base, sort="updated",
                   direction="desc", per_page=page_size, page=page)
        selected.extend(row for row in rows if row.get("merged_at") and timestamp(row["merged_at"]) <= cutoff)
        selected.sort(key=lambda row: (timestamp(row["merged_at"]), row["number"]), reverse=True)
        selected = selected[:limit]
        # updated_at >= merged_at, so older pages cannot contain a later merge.
        if len(rows) < page_size or (len(selected) == limit and
                                    timestamp(rows[-1]["updated_at"]) < timestamp(selected[-1]["merged_at"])):
            break
        page += 1
    if len(selected) < limit:
        raise ValueError(f"requested {limit} merged PRs but only found {len(selected)}")
    return selected


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root, text=True, encoding="utf-8").strip()


def base_sync_commits(root, base, head):
    branch_range = f"{base}..{head}"
    shallow = Path(git(root, "rev-parse", "--git-path", "shallow"))
    if not shallow.is_absolute():
        shallow = root / shallow
    if shallow.exists() and set(shallow.read_text().splitlines()) & set(git(root, "rev-list", branch_range).splitlines()):
        raise ValueError("incomplete PR history; fetch the missing Git history before collecting")
    base_history = set(git(root, "rev-list", "--first-parent", base).splitlines())
    merges = git(root, "log", "--first-parent", "--min-parents=2", "--format=%H%x00%P%x00%s", branch_range)
    evidence = []
    for line in merges.splitlines():
        sha, parents, subject = line.split("\0", 2)
        parents = parents.split()
        evidence.append({"sha": sha, "parents": parents, "subject": subject,
                         "is_base_sync": len(parents) == 2 and parents[1] in base_history})
    return evidence


def paged(repo, endpoint, key, **params):
    result, page = [], 1
    while True:
        rows = api(repo, endpoint, per_page=100, page=page, **params)[key]
        result.extend(rows)
        if len(rows) < 100:
            return result
        page += 1


def collect(repo, base, limit, before, root, workflow):
    snapshot = {"schema_version": 1, "repo": repo, "base": base, "limit": limit,
                "before": before, "workflow": workflow,
                "collected_at": datetime.now(timezone.utc).isoformat(), "pull_requests": []}
    protection = api(repo, f"branches/{base}/protection")
    checks = protection["required_status_checks"] or {}
    snapshot["required_check_names"] = sorted(set(checks.get("contexts", [])) |
                                              {check["context"] for check in checks.get("checks", [])})
    for pr in merged_prs(repo, base, limit, before):
        print(f"Collecting PR #{pr['number']}", file=sys.stderr, flush=True)
        merge = pr["merge_commit_sha"]
        parents = git(root, "show", "-s", "--format=%P", merge).split()
        if len(parents) != 2:
            raise ValueError(f"PR #{pr['number']} requires a two-parent merge commit and its Git history")
        head = parents[1]
        evidence = base_sync_commits(root, parents[0], head)
        syncs = [row for row in evidence if row["is_base_sync"]]
        runs = paged(repo, f"actions/workflows/{workflow}/runs", "workflow_runs",
                     event="pull_request", head_sha=head)
        runs = [run for run in runs if run["head_sha"] == head and
                timestamp(pr["created_at"]) <= timestamp(run["created_at"])
                and timestamp(run["updated_at"]) <= timestamp(pr["merged_at"])]
        successful = [run for run in runs if run["status"] == "completed" and run["conclusion"] == "success"]
        if successful:
            selected = max(successful, key=lambda run: (timestamp(run["run_started_at"]), run["id"]))
            selected["jobs"] = paged(repo, f"actions/runs/{selected['id']}/attempts/{selected['run_attempt']}/jobs", "jobs")
        snapshot["pull_requests"].append({
            "number": pr["number"], "html_url": pr["html_url"], "head_ref": pr["head"]["ref"],
            "merge_commit_sha": merge, "head_sha": head,
            "created_at": pr["created_at"], "merged_at": pr["merged_at"],
            "sync_commits": [row["sha"] for row in syncs],
            "conventional_sync_commits": [row["sha"] for row in syncs
                                          if row["subject"].startswith("Merge ") and base in row["subject"]],
            "sync_commit_evidence": evidence, "runs": runs})
    return snapshot


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--input", type=Path, help="Replay saved JSON without GitHub or Git")
    source.add_argument("--repo", help="GitHub owner/repo; requires authenticated gh and local merge history")
    parser.add_argument("--base", default="develop")
    parser.add_argument("--limit", type=int, default=25)
    parser.add_argument("--before", default=datetime.now(timezone.utc).isoformat(), help="Inclusive timezone-aware cutoff")
    parser.add_argument("--workflow", default="test.yml")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--save", type=Path, help="Save collected input JSON for reproducible offline replay")
    args = parser.parse_args()
    if args.limit < 1:
        parser.error("--limit must be positive")
    if args.input and args.save:
        parser.error("--save applies to collection, not replay")
    try:
        data = json.loads(args.input.read_text(encoding="utf-8")) if args.input else collect(
            args.repo, args.base, args.limit, args.before, args.root, args.workflow)
        if args.save:
            args.save.parent.mkdir(parents=True, exist_ok=True)
            args.save.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(json.dumps(summarize(data), indent=2, ensure_ascii=False))
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"ci_throughput: {error}\n")


if __name__ == "__main__":
    main()
