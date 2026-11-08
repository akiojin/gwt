#!/usr/bin/env python3
"""Measure CI throughput and queue delays using read-only GitHub data or saved snapshots."""
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
    step_durations = defaultdict(lambda: defaultdict(list))
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
                for step in job.get("steps", []):
                    if step.get("conclusion") == "success" and step.get("started_at") and step.get("completed_at"):
                        step_durations[job["name"]][step["name"]].append(
                            minutes(step["started_at"], step["completed_at"]))
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
                        "step_duration_minutes": {step: distribution(times)
                                                  for step, times in sorted(step_durations[name].items())},
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


def collect_queue(repo, limit, before, workflow):
    endpoint = f"actions/workflows/{workflow}/runs" if workflow else "actions/runs"
    runs, page = [], 1
    while len(runs) < limit:
        rows = api(repo, endpoint, status="completed", created=f"<={before}",
                   per_page=min(limit, 100), page=page)["workflow_runs"]
        runs.extend(row for row in rows if row["status"] == "completed"
                    and timestamp(row["created_at"]) <= timestamp(before))
        if len(rows) < min(limit, 100):
            break
        page += 1
    runs.sort(key=lambda row: (timestamp(row["created_at"]), row["id"]), reverse=True)
    if len(runs) < limit:
        raise ValueError(f"requested {limit} completed runs but only found {len(runs)}")
    for run in runs[:limit]:
        run["jobs"] = paged(repo, f"actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs", "jobs")
    return {"schema_version": 1, "repo": repo, "before": before,
            "collected_at": datetime.now(timezone.utc).isoformat(), "runs": runs[:limit]}


def summarize_queue(snapshot):
    if snapshot["schema_version"] != 1 or not snapshot["runs"]:
        raise ValueError("expected a schema_version 1 snapshot with completed runs")
    queues, scheduling, unavailable, rows = [], [], [], []
    for run in snapshot["runs"]:
        jobs, waits, delays = [], [], []
        for job in run["jobs"]:
            if job["conclusion"] in ("skipped", "cancelled") or not job.get("completed_at"):
                continue
            created, started = job.get("created_at"), job.get("started_at")
            if not created or not started or not (
                    timestamp(run["created_at"]) <= timestamp(created) <= timestamp(started)
                    and timestamp(run["run_started_at"]) <= timestamp(started)):
                unavailable.append(job["id"])
                continue
            waits.append(minutes(created, started))
            if timestamp(created) >= timestamp(run["run_started_at"]):
                delays.append(minutes(run["run_started_at"], created))
            jobs.append({"name": job["name"], "duration_minutes": minutes(started, job["completed_at"])})
        queues.extend(waits)
        scheduling.extend(delays)
        rows.append({"id": run["id"], "workflow": run["name"], "head_sha": run["head_sha"],
                     "run_attempt": run["run_attempt"], "created_at": run["created_at"],
                     "conclusion": run["conclusion"],
                     "run_queue_minutes": (minutes(run["created_at"], run["run_started_at"])
                                           if run["run_attempt"] == 1 else None),
                     "run_execution_minutes": minutes(run["run_started_at"], run["updated_at"]),
                     "job_queue_minutes": distribution(waits),
                     "dependency_scheduling_minutes": distribution(delays),
                     "longest_job": max(jobs, key=lambda job: job["duration_minutes"]) if jobs else None})
    return {"repo": snapshot["repo"], "before": snapshot["before"], "run_count": len(rows),
            "method": {"selection": "Latest completed runs by created_at; all conclusions retained",
                       "run_queue": "run created_at -> run_started_at; unavailable for reruns",
                       "job_queue": "job created_at -> started_at; includes job concurrency admission",
                       "dependency_scheduling": "run_started_at -> job created_at; includes needs and scheduling",
                       "exclusions": "Skipped/cancelled jobs and unavailable/reused attempt timestamps",
                       "limitation": "API timings alone cannot distinguish hosted capacity from concurrency waiting",
                       "units": "minutes; p50 median, p90 nearest rank"},
            "run_queue_minutes": distribution(row["run_queue_minutes"] for row in rows
                                               if row["run_queue_minutes"] is not None),
            "job_queue_minutes": distribution(queues),
            "dependency_scheduling_minutes": distribution(scheduling),
            "unavailable_job_ids": unavailable, "runs": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--input", type=Path, help="Replay saved JSON without GitHub or Git")
    source.add_argument("--repo", help="GitHub owner/repo; requires authenticated gh (merge mode also needs Git history)")
    parser.add_argument("--base", default="develop")
    parser.add_argument("--limit", type=int, help="Sample size (default: 25 merged PRs or 50 queue runs)")
    parser.add_argument("--queue", action="store_true", help="Measure latest completed workflow runs, without Git history")
    parser.add_argument("--before", default=datetime.now(timezone.utc).isoformat(), help="Inclusive timezone-aware cutoff")
    parser.add_argument("--workflow", help="Workflow filename (default: test.yml; all workflows with --queue)")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--save", type=Path, help="Save collected input JSON for reproducible offline replay")
    args = parser.parse_args()
    limit = args.limit if args.limit is not None else (50 if args.queue else 25)
    if limit < 1:
        parser.error("--limit must be positive")
    if args.input and args.save:
        parser.error("--save applies to collection, not replay")
    try:
        if args.input:
            data = json.loads(args.input.read_text(encoding="utf-8"))
        elif args.queue:
            data = collect_queue(args.repo, limit, args.before, args.workflow)
        else:
            data = collect(args.repo, args.base, limit, args.before, args.root, args.workflow or "test.yml")
        if args.save:
            args.save.parent.mkdir(parents=True, exist_ok=True)
            args.save.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        report = summarize_queue(data) if "runs" in data else summarize(data)
        print(json.dumps(report, indent=2, ensure_ascii=False))
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"ci_throughput: {error}\n")


if __name__ == "__main__":
    main()
