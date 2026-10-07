"""Classify CI synchronization without treating base commits as PR changes.

The Git proof mirrors gwt's pr/head_check.rs (#4979). This is a cancellation
diagnostic, never an authorization to make canonical verification evidence fresh.
"""

import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import subprocess


def git(repo, *args):
    return subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, check=True
    ).stdout


def text(repo, *args):
    return git(repo, *args).decode().strip()


def product_paths(repo, before, after):
    return [path.decode(errors="replace") for path in git(
        repo, "diff", "--name-only", "--no-renames", "-z", before, after,
        "--", ".", ":(exclude).gwt",
    ).split(b"\0") if path]


def projected_tree(repo, first, second):
    result = subprocess.run(
        ["git", "merge-tree", "--write-tree", "--name-only", "--no-messages",
         "-z", first, second], cwd=repo, capture_output=True,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError(result.stderr.decode(errors="replace").strip())
    fields = result.stdout.split(b"\0")
    tree = fields[0].decode()
    if len(tree) not in (40, 64) or any(c not in "0123456789abcdef" for c in tree):
        raise RuntimeError("Git did not return a projected merge tree")
    conflicts = []
    if result.returncode == 1:
        for path in fields[1:]:
            if not path:
                break
            if path != b".gwt" and not path.startswith(b".gwt/"):
                conflicts.append(path.decode(errors="replace"))
    return tree, conflicts


def classify(repo, before, head, base):
    result = {
        "before": before, "head": head, "base": base,
        "classification": "unprovable", "base_only": None,
        "product_commits": [], "product_files": [],
        "reuse_evidence": False,
        "reuse_reason": "Paired evidence measures the full tested source tree; "
                        "base changes can alter test binaries, so rerun on the new tree.",
    }
    try:
        for key in ("before", "head", "base"):
            result[key] = text(repo, "rev-parse", "--verify", "--end-of-options",
                               f"{result[key]}^{{commit}}")
        before, head, base = (result[key] for key in ("before", "head", "base"))
        if before == head:
            result.update(classification="matched", base_only=True)
            return result
        ancestry = subprocess.run(
            ["git", "merge-base", "--is-ancestor", before, head], cwd=repo,
            capture_output=True,
        )
        if ancestry.returncode == 1:
            result.update(classification="divergent_or_behind", base_only=False)
            return result
        ancestry.check_returncode()
        files = set()
        for commit in text(repo, "rev-list", "--reverse", head,
                           f"^{before}", f"^{base}").splitlines():
            parents = text(repo, "rev-list", "--parents", "-n", "1", commit).split()[1:]
            if len(parents) == 1:
                paths = product_paths(repo, parents[0], commit)
            elif len(parents) == 2:
                tree, conflicts = projected_tree(repo, *parents)
                paths = conflicts or product_paths(repo, tree, commit)
            else:
                raise RuntimeError("Cannot prove a multi-parent merge is base-only")
            if paths:
                result["product_commits"].append(commit)
                files.update(paths)
        if not files:
            bases = text(repo, "merge-base", "--all", head, base).splitlines()
            if len(bases) != 1:
                raise RuntimeError("Cannot prove a unique base synchronization")
            tree, conflicts = projected_tree(repo, before, bases[0])
            if conflicts:
                raise RuntimeError(f"Product conflicts in base projection: {conflicts}")
            paths = product_paths(repo, tree, head)
            if paths:
                result["product_commits"].append(head)
                files.update(paths)
        result["product_files"] = sorted(files)
        result.update(
            classification="unverified_product" if files else "bookkeeping_or_base_sync",
            base_only=not files,
        )
    except (subprocess.CalledProcessError, RuntimeError) as error:
        diagnostic = getattr(error, "stderr", None)
        result["diagnostic"] = (
            diagnostic.decode(errors="replace").strip() if diagnostic else str(error)
        )
    return result


def event_relation(repo, event):
    pr = event.get("pull_request", {})
    group = event.get("merge_group", {})
    head = pr.get("head", {}).get("sha") or group.get("head_sha")
    base = pr.get("base", {}).get("sha") or group.get("base_sha")
    before = event.get("before")
    if event.get("action") == "synchronize":
        if before:
            return classify(repo, before, head, base)
        result = classify(repo, head, head, base)
        result.update(classification="unprovable", base_only=None,
                      diagnostic="Synchronize event has no previous HEAD to compare")
        return result
    # Open/reopen and merge-group events have no earlier PR source to prove.
    # They start normally; merge groups have distinct refs/concurrency groups.
    result = classify(repo, head, head, base)
    if result["classification"] == "matched":
        result.update(classification="initial", base_only=False)
    return result


def stamp(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def run_order(run):
    # Actions timestamps have only second precision. The per-workflow run
    # number disambiguates simultaneous runs; absent numbers cannot prove a tie.
    return stamp(run["created_at"]), run.get("run_number", 0)


def runs_for_pr(runs, pr):
    """GitHub sometimes omits PR associations; label the narrower inference."""
    selected = []
    counts = {"explicit_pr_association": 0,
              "inferred_head_repo_branch_lifetime": 0, "excluded": 0}
    for run in runs:
        associations = run.get("pull_requests", [])
        if associations:
            membership = "explicit_pr_association" if any(
                item.get("number") == pr["number"] for item in associations
            ) else None
        else:
            membership = "inferred_head_repo_branch_lifetime" if (
                run.get("head_branch") == pr["head"]["ref"]
                and (run.get("head_repository") or {}).get("id") == (pr["head"].get("repo") or {}).get("id")
                and (pr["head"].get("repo") or {}).get("id") is not None
                and stamp(run["created_at"]) >= stamp(pr["created_at"])
                and (not pr.get("closed_at")
                     or stamp(run["created_at"]) <= stamp(pr["closed_at"]))
            ) else None
        counts[membership or "excluded"] += 1
        if membership:
            selected.append(dict(run, pr_membership=membership))
    return selected, counts


def cancellation_decision(repo, event, runs, current_run_id):
    result = event_relation(repo, event)
    current = next((run for run in runs if run["id"] == current_run_id), None)
    comparisons = []
    for run in runs:
        if run["id"] == current_run_id or run.get("status") == "completed":
            continue
        # Never let a delayed classifier compare against a newer execution.
        if current and run_order(run) >= run_order(current):
            continue
        proof = classify(repo, run["head_sha"], result["head"], result["base"])
        if not current and proof["classification"] == "divergent_or_behind":
            # API propagation may omit this run. An ancestor HEAD still proves
            # older source; a divergent/newer HEAD cannot prove run ordering.
            proof.update(classification="unprovable", base_only=None,
                         diagnostic="Cannot prove ordering against the new run")
        comparisons.append({"run_id": run["id"], "source_relation": proof})
    result["active_run_comparisons"] = comparisons
    proofs = [row["source_relation"] for row in comparisons]
    product = [proof for proof in proofs if proof["base_only"] is False]
    if product:
        result["event_classification"] = result["classification"]
        result.update(classification="unverified_product", base_only=False)
        for key in ("product_commits", "product_files"):
            result[key] = sorted(set(result[key]).union(
                *(set(proof[key]) for proof in product)
            ))
    elif result["base_only"] is True and any(proof["base_only"] is None for proof in proofs):
        result.update(classification="unprovable", base_only=None,
                      diagnostic="Cannot compare the source of an active predecessor")
    return result


def cancellation_audit(repo, runs, base):
    """Count source-related successors, not an actor GitHub never supplies."""
    ordered = sorted({run["id"]: run for run in runs}.values(), key=run_order)
    report = {
        "attribution": "inferred_from_successor_source_relation",
        "base_sync_successor_cancelled_runs": 0,
        "product_change_successor_cancelled_runs": 0,
        "unknown_cancelled_runs": 0,
        "runs": [],
    }
    for index, run in enumerate(ordered):
        if run.get("conclusion") != "cancelled":
            continue
        row = {"run_id": run["id"], "attribution": "unknown"}
        if "pr_membership" in run:
            row["pr_membership"] = run["pr_membership"]
        successors = [later for later in ordered[index + 1:]
                      if run_order(later) > run_order(run)
                      and stamp(later["created_at"]) <= stamp(run["updated_at"])]
        comparisons = []
        for successor in successors:
            comparison = {"successor_run_id": successor["id"], "attribution": "unknown"}
            try:
                head = successor["head_sha"]
                comparison_base = base
                parents = text(repo, "rev-list", "--parents", "-n", "1", head).split()[1:]
                # A historical merge's target parent is a stable base snapshot.
                # Using today's base indiscriminately would hide PR changes
                # that have since landed there.
                if len(parents) == 2 and subprocess.run(
                    ["git", "merge-base", "--is-ancestor", parents[1], base],
                    cwd=repo, capture_output=True,
                ).returncode == 0:
                    comparison_base = parents[1]
                elif subprocess.run(
                    ["git", "merge-base", "--is-ancestor", head, base],
                    cwd=repo, capture_output=True,
                ).returncode == 0:
                    raise RuntimeError("Historical HEAD is already in base; event base is unknown")
                proof = classify(repo, run["head_sha"], head, comparison_base)
                comparison["source_relation"] = proof
                if proof["base_only"] is True and proof["before"] != proof["head"]:
                    comparison["attribution"] = "base_sync_successor"
                elif proof["base_only"] is False:
                    comparison["attribution"] = "product_change_successor"
            except (subprocess.CalledProcessError, RuntimeError) as error:
                comparison["diagnostic"] = str(error)
            comparisons.append(comparison)
        if comparisons:
            row["successor_comparisons"] = comparisons
            if len({comparison["attribution"] for comparison in comparisons}) == 1:
                row.update(comparisons[0])
            else:
                row["diagnostic"] = "Overlapping successors have mixed or unknown source relations"
        key = {"base_sync_successor": "base_sync_successor_cancelled_runs",
               "product_change_successor": "product_change_successor_cancelled_runs",
               "unknown": "unknown_cancelled_runs"}[row["attribution"]]
        report[key] += 1
        report["runs"].append(row)
    return report


def run_pages(data):
    if isinstance(data, dict):
        return data["workflow_runs"]
    return [run for page in data for run in page["workflow_runs"]]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--event-file", type=Path)
    parser.add_argument("--runs-file", type=Path)
    parser.add_argument("--audit-pr", type=int)
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY"))
    parser.add_argument("--before")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--base", default="origin/develop")
    args = parser.parse_args()
    if args.audit_pr:
        if not args.repository:
            parser.error("--audit-pr requires --repository OWNER/REPO")
        pr = json.loads(subprocess.check_output(
            ["gh", "api", f"repos/{args.repository}/pulls/{args.audit_pr}"], cwd=args.repo,
        ))
        pages = json.loads(subprocess.check_output(
            ["gh", "api", "--method", "GET",
             f"repos/{args.repository}/actions/workflows/test.yml/runs",
             "-f", f"branch={pr['head']['ref']}", "-f", "event=pull_request",
             "-f", "per_page=100", "--paginate", "--slurp"], cwd=args.repo,
        ))
        runs, membership = runs_for_pr(run_pages(pages), pr)
        result = cancellation_audit(args.repo, runs, args.base)
        result["run_membership"] = membership
        result.update(pr=args.audit_pr, branch=pr["head"]["ref"])
        print(json.dumps(result, sort_keys=True))
        return
    if args.event_file:
        event = json.loads(args.event_file.read_text())
        result = event_relation(args.repo, event)
        if args.runs_file and event.get("pull_request"):
            try:
                runs, membership = runs_for_pr(
                    run_pages(json.loads(args.runs_file.read_text())), event["pull_request"],
                )
                result = cancellation_decision(
                    args.repo, event, runs, int(os.environ.get("GITHUB_RUN_ID", "0")),
                )
                result["run_membership"] = membership
                result["cancellations"] = cancellation_audit(
                    args.repo, runs, result["base"],
                )
            except (OSError, ValueError, KeyError) as error:
                result["cancellations"] = {"status": "unavailable", "diagnostic": str(error)}
    else:
        if not args.before:
            parser.error("--before or --event-file is required")
        result = classify(args.repo, args.before, args.head, args.base)
    print(json.dumps(result, sort_keys=True))
    value = {True: "true", False: "false", None: "unknown"}[result["base_only"]]
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a") as stream:
            stream.write(f"base_only={value}\n")
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a") as stream:
            stream.write("## CI synchronization (#5059)\n\n")
            stream.write(f"Classification: `{result['classification']}`; "
                         f"cancel running jobs: `{value == 'false'}`.\n\n")
            stream.write(f"Paired evidence reuse: no. {result['reuse_reason']}\n\n")
            stream.write(f"```json\n{json.dumps(result, indent=2)}\n```\n")


if __name__ == "__main__":
    main()
