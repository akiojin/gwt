#!/usr/bin/env python3
"""Paired Windows gwt-lib scheduling measurements for #4822 (not dev-host AC-4)."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time
import xml.etree.ElementTree as ET

SELECTION = ["-p", "gwt", "--lib", "--all-features"]


def inventory(data):
    result = {}
    for binary, suite in data["rust-suites"].items():
        for name, test in suite["testcases"].items():
            ignored = test["ignored"]
            selected = test["filter-match"]["status"] == "matches"
            if not ignored and not selected:
                raise ValueError(f"inventory excludes non-ignored test: {binary}::{name}")
            result[(binary, name)] = ignored
    if not result:
        raise ValueError("empty test inventory")
    return result


def durations(xml, phase):
    result = {}
    for suite in ET.fromstring(xml).iter("testsuite"):
        for test in suite.findall("testcase"):
            key = (test.get("classname", suite.attrib["name"]), test.attrib["name"])
            if key in result:
                raise ValueError(f"duplicate JUnit test: {key}")
            if any(test.find(tag) is not None for tag in ("failure", "error", "skipped", "rerun", "flakyFailure", "flakyError")):
                raise ValueError(f"{phase}: JUnit contains failed, retried or skipped test: {key}")
            seconds = float(test.attrib["time"])
            if not math.isfinite(seconds) or seconds < 0:
                raise ValueError(f"invalid JUnit duration: {key}")
            result[key] = seconds
    if not result:
        raise ValueError("empty JUnit report")
    return result


def summarize(values):
    ordered = sorted(values.values())
    distribution = {"count": len(ordered), "sum": sum(ordered), "p50": statistics.median(ordered),
                    "p95": ordered[math.ceil(len(ordered) * .95) - 1], "max": max(ordered),
                    "histogram_seconds": {label: sum(low <= value < high for value in ordered)
                                          for label, low, high in (("0-1", 0, 1), ("1-10", 1, 10),
                                                                  ("10-60", 10, 60), ("60-120", 60, 120),
                                                                  ("120+", 120, math.inf))}}
    top = [{"binary": key[0], "name": key[1], "seconds": seconds}
           for key, seconds in sorted(values.items(), key=lambda pair: (-pair[1], pair[0]))[:20]]
    return {"distribution": distribution, "top20": top}


def compare(before_list, after_list, before_xml, after_xml):
    before_inventory, after_inventory = inventory(before_list), inventory(after_list)
    if before_inventory != after_inventory:
        raise ValueError("before/after inventory or ignored status differs")
    expected = {key for key, ignored in before_inventory.items() if not ignored}
    before, after = durations(before_xml, "before"), durations(after_xml, "after")
    if set(before) != expected or set(after) != expected:
        raise ValueError("JUnit executed set differs from non-ignored inventory")
    return {
        "inventory": {"total": len(before_inventory), "executed": len(expected),
                      "ignored": len(before_inventory) - len(expected)},
        "before": summarize(before), "after": summarize(after),
        "deltas": [{"binary": key[0], "name": key[1], "before_seconds": before[key],
                    "after_seconds": after[key], "delta_seconds": after[key] - before[key]}
                   for key in sorted(before, key=lambda key: (-before[key], key))],
        "workspace": {phase: summarize(subset) if subset else None
                      for phase, data in (("before", before), ("after", after))
                      for subset in [{key: seconds for key, seconds in data.items()
                                      if key[1].startswith("cli::workspace::tests::")}]},
    }


def summarize_trace(events):
    sessions, children, result = {}, {}, {}
    for event in events:
        sid = event.get("sid")
        if event.get("event") == "def_param" and event.get("param") == "NEXTEST_TEST_NAME":
            sessions[sid] = event["value"]
        elif event.get("event") == "child_start":
            children[(sid, event["child_id"])] = " ".join(event.get("argv", [])).lower()
        name = sessions.get(sid)
        if not name:
            continue
        row = result.setdefault(name, {"git_processes": 0, "git_seconds": 0.0,
                                      "credential_children": 0, "credential_seconds": 0.0})
        # Top-level git elapsed only: nested Git sessions would double-count it.
        # Trace2 starts inside git, so this excludes OS spawn time before main.
        if event.get("event") == "exit" and "/" not in sid:
            row["git_processes"] += 1
            row["git_seconds"] += event["t_abs"]
        elif event.get("event") == "child_exit":
            command = children.get((sid, event["child_id"]), "")
            if "credential" in command or "askpass" in command:
                row["credential_children"] += 1
                row["credential_seconds"] += event["t_rel"]
    return result


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def sha256(path):
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def measure(root, output):
    output.mkdir(parents=True, exist_ok=False)  # Never accept stale evidence.
    source = (root / ".config/nextest.toml").read_text(encoding="utf-8")
    config = output / "nextest.toml"
    config.write_text(source + '\n[store]\ndir = ' + json.dumps(str(output / "store")) + '''
[profile.gwt-before]
retries = 0
test-threads = 1
[profile.gwt-before.junit]
path = "junit.xml"
''', encoding="utf-8")
    metadata = {"platform": platform.platform(), "config_sha256": hashlib.sha256(source.encode()).hexdigest(),
                "comparison": "Same test binaries: serial nextest baseline vs gwt-verify scheduling. "
                              "Both use process-per-test; this does not measure cargo-test harness migration "
                              "or the Windows dev-host 30-minute target (#4877).",
                "runs": {}}
    for command in (["git", "rev-parse", "HEAD"], ["cargo", "nextest", "--version"], ["rustc", "-Vv"]):
        metadata[" ".join(command)] = subprocess.check_output(command, cwd=root, text=True).strip()
    base_sha = os.environ.get("GWT_TIMING_BASE_SHA")
    if base_sha:
        metadata["source_diff_base"] = base_sha
        with (output / "source-diff.patch").open("w", encoding="utf-8") as stream:
            subprocess.run(["git", "diff", f"{base_sha}...HEAD", "--", "crates/gwt/src", "crates/gwt/tests"],
                           cwd=root, stdout=stream, check=True)
    write_json(output / "metadata.json", metadata)
    for phase, profile in (("before", "gwt-before"), ("after", "gwt-verify")):
        common = SELECTION + ["--config-file", str(config), "--profile", profile, "--color=never"]
        list_command = ["cargo", "nextest", "list"] + common + ["--message-format", "json"]
        with (output / f"{phase}-list.json").open("w", encoding="utf-8") as stream:
            subprocess.run(list_command, cwd=root, stdout=stream, check=True)
        listed = json.loads((output / f"{phase}-list.json").read_text(encoding="utf-8"))
        binaries = {binary: sha256(Path(suite["binary-path"])) for binary, suite in listed["rust-suites"].items()}
        command = ["cargo", "nextest", "run"] + common + ["--retries", "0"]
        env = dict(os.environ, GIT_TRACE2_EVENT=str(output / f"{phase}-git-trace.jsonl"),
                   GIT_TRACE2_ENV_VARS="NEXTEST_TEST_NAME")
        started = time.monotonic()
        with (output / f"{phase}-run.log").open("w", encoding="utf-8") as stream:
            completed = subprocess.run(command, cwd=root, env=env, stdout=stream, stderr=subprocess.STDOUT)
        metadata["runs"][phase] = {"command": command, "list_command": list_command,
                                    "exit_code": completed.returncode, "wall_seconds": time.monotonic() - started,
                                    "binary_sha256": binaries}
        write_json(output / "metadata.json", metadata)
        print(f"{phase}: exit={completed.returncode}, wall={metadata['runs'][phase]['wall_seconds']:.3f}s", flush=True)
    try:
        reports = {}
        for phase, profile in (("before", "gwt-before"), ("after", "gwt-verify")):
            reports[phase] = (output / "store" / profile / "junit.xml").read_text(encoding="utf-8")
            (output / f"{phase}-junit.xml").write_text(reports[phase], encoding="utf-8")
        report = compare(*(json.loads((output / f"{phase}-list.json").read_text(encoding="utf-8"))
                           for phase in ("before", "after")), reports["before"], reports["after"])
        if metadata["runs"]["before"]["binary_sha256"] != metadata["runs"]["after"]["binary_sha256"]:
            raise ValueError("test binaries changed between measurements")
        if any(run["exit_code"] != 0 for run in metadata["runs"].values()):
            raise ValueError("one or both nextest runs failed")
        report["git_trace"] = {}
        for phase in ("before", "after"):
            with (output / f"{phase}-git-trace.jsonl").open(encoding="utf-8") as stream:
                report["git_trace"][phase] = summarize_trace(json.loads(line) for line in stream if line.strip())
            if not any(name.startswith("cli::workspace::tests::") for name in report["git_trace"][phase]):
                raise ValueError(f"{phase}: no workspace Git trace attributed to nextest test names")
        report["limitations"] = [metadata["comparison"],
            "Git elapsed excludes pre-main OS spawn cost. Residual test time is not proof of internal timeout.",
            "Credential child duration includes spawn/wait; no credential event does not rule out agent waits outside Git.",
            "Only current CI conditions are observed; historical ~120s/test must not be inferred if not reproduced.",
            "One ordered pair is measured; runner load and cache warming may affect the distributions.",
            "Same-binary parity checks selection. source-diff.patch separately supports auditing removed tests or newly ignored source tests."]
        write_json(output / "comparison.json", report)
        lines = ["# Windows paired scheduling evidence", "", metadata["comparison"], "",
                 f"Identical inventory: {report['inventory']}", ""]
        for phase in ("before", "after"):
            lines += [f"## {phase}", "", json.dumps(report[phase]["distribution"]), "",
                      "| Test | Seconds |", "| --- | ---: |"]
            lines += [f"| {row['binary']}::{row['name']} | {row['seconds']:.6f} |" for row in report[phase]["top20"]]
            lines += [""]
        top_keys = {(row["binary"], row["name"]) for phase in ("before", "after")
                    for row in report[phase]["top20"]}
        lines += ["## Top-20 changes (union of both runs)", "",
                  "| Test | Before seconds | After seconds | Delta seconds |", "| --- | ---: | ---: | ---: |"]
        lines += [f"| {row['binary']}::{row['name']} | {row['before_seconds']:.6f} | {row['after_seconds']:.6f} | {row['delta_seconds']:+.6f} |"
                  for row in report["deltas"] if (row["binary"], row["name"]) in top_keys]
        lines += ["", "## Workspace cause evidence", "",
                  "Git Trace2 is attributed by NEXTEST_TEST_NAME. Compare its elapsed time and helper waits with JUnit; residual time is unclassified.", ""]
        for phase in ("before", "after"):
            rows = {name: row for name, row in report["git_trace"][phase].items()
                    if name.startswith("cli::workspace::tests::")}
            stats = report["workspace"][phase]
            lines += [f"### {phase}", "", f"Duration distribution: {json.dumps(stats['distribution'])}",
                      f"Tests with Git trace: {len(rows)}", "",
                      "| Test | Test seconds | Git seconds (excludes spawn) | Git processes | Credential children | Credential seconds |",
                      "| --- | ---: | ---: | ---: | ---: | ---: |"]
            for test in stats["top20"]:
                trace = rows.get(test["name"])
                if trace:
                    lines.append(f"| {test['name']} | {test['seconds']:.6f} | {trace['git_seconds']:.6f} | {trace['git_processes']} | {trace['credential_children']} | {trace['credential_seconds']:.6f} |")
                else:
                    lines.append(f"| {test['name']} | {test['seconds']:.6f} | unavailable | | | |")
            lines += [""]
        lines += ["## Limits", ""] + [f"- {line}" for line in report["limitations"]]
        (output / "comparison.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    except (ValueError, KeyError, OSError, ET.ParseError) as error:
        write_json(output / "comparison-error.json", {"error": str(error)})
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    measure(args.root.resolve(), args.output.resolve())
