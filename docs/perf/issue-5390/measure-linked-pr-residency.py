#!/usr/bin/env python3
"""Measure queued payloads using the extracted LF7 Rust reader, not an RSS model.

Use an existing native serde_json rlib; this script builds no Cargo target and
downloads nothing. Example (baseline is a preserved pre-change issue.rs):
  python measure-linked-pr-residency.py --baseline OLD --current NEW \
      --deps-dir target/debug/deps --output linked-pr-residency.json
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def fixture():
    responses = []
    for continuation in (False, True):
        for start in (1, 26):
            repository = {}
            for issue in range(start, start + 25):
                nodes = []
                for event in range(10 if continuation else 100):
                    pr = issue * 1000 + event % 10
                    prefix = f"Closes #{issue}\n"
                    nodes.append({
                        "__typename": "CrossReferencedEvent",
                        "willCloseTarget": False,
                        "source": {
                            "__typename": "PullRequest", "number": pr,
                            "title": f"{'later' if continuation else 'first'}-{pr}",
                            "state": "MERGED" if continuation else "OPEN",
                            "url": f"https://example.test/pull/{pr}",
                            "body": "" if continuation else prefix + "x" * (4096 - len(prefix)),
                            "mergedAt": "2026-11-04T00:00:00Z" if continuation else None,
                        },
                    })
                repository[f"i{issue}"] = {"timelineItems": {
                    "nodes": nodes, "pageInfo": {
                        "hasNextPage": not continuation,
                        "endCursor": None if continuation else f"next-{issue}",
                    },
                }}
            responses.append(json.dumps({"data": {"repository": repository}}, separators=(",", ":")))
    return encoded(responses)


PRELUDE = r'''
#![allow(dead_code)]
use std::{cell::RefCell, collections::{BTreeMap, BTreeSet, VecDeque}, io};
use serde_json::{json, Value};
#[derive(Clone, Debug)]
struct LinkedPrSummary {
    number: u64, title: String, state: String, url: String,
    will_close_target: bool, merged_at: Option<String>,
}
// Probe-only closed stubs: fake transport has no real deadline or budget ledger.
mod gwt_core {
    pub mod operation_deadline {
        pub fn ensure_remaining(_: &str) -> std::io::Result<()> { Ok(()) }
    }
    pub mod github_budget {
        pub fn parse_graphql_rate_limit(_: &serde_json::Value) -> Option<()> { None }
        pub fn spawn_source(_: &[&str]) {}
        pub struct BudgetLedger;
        impl BudgetLedger {
            pub fn global() -> Self { Self }
            pub fn record_graphql_response(&self, _: &(), _: &(), _: ()) {}
        }
    }
}
mod chrono { pub struct Utc; impl Utc { pub fn now() {} } }
thread_local! { static SAMPLES: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) }; }
#[derive(Default)]
struct Payload {
    raw_nodes: usize, raw_body_bytes: usize, raw_string_bytes: usize,
    summaries: usize, summary_string_bytes: usize, summary_index_entries: usize,
}
trait Retained { fn count(&self, count: &mut Payload); }
fn strings(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Array(a) => a.iter().map(strings).sum(),
        Value::Object(o) => o.values().map(strings).sum(),
        _ => 0,
    }
}
impl Retained for Vec<Value> {
    fn count(&self, count: &mut Payload) {
        count.raw_nodes += self.len();
        for node in self {
            count.raw_string_bytes += strings(node);
            count.raw_body_bytes += node.get("source").or_else(|| node.get("subject"))
                .and_then(|pr| pr.get("body")).and_then(Value::as_str).map_or(0, str::len);
        }
    }
}
fn record_queue<T: Retained>(
    queue: &VecDeque<(Vec<u64>, BTreeMap<u64, String>, BTreeSet<(u64, String)>, BTreeMap<u64, T>)>
) {
    let mut count = Payload::default();
    let mut issues = 0;
    for (_, _, _, payloads) in queue {
        issues += payloads.len();
        for payload in payloads.values() { payload.count(&mut count); }
    }
    SAMPLES.with(|s| s.borrow_mut().push(json!({
        "queued_issues": issues,
        "raw_nodes": count.raw_nodes, "raw_body_bytes": count.raw_body_bytes,
        "raw_string_value_bytes": count.raw_string_bytes,
        "summaries": count.summaries, "summary_string_bytes": count.summary_string_bytes,
        "summary_index_entries": count.summary_index_entries,
    })));
}
fn summary(pr: &LinkedPrSummary) -> Value {
    json!({"number": pr.number, "title": pr.title, "state": pr.state, "url": pr.url,
           "will_close_target": pr.will_close_target, "merged_at": pr.merged_at})
}
'''

COMPACT_COUNTER = r'''
impl Retained for LinkedPrAccumulator {
    fn count(&self, count: &mut Payload) {
        count.summaries += self.prs.len();
        count.summary_index_entries += self.index.len();
        count.summary_string_bytes += self.prs.iter().map(|pr|
            pr.title.len() + pr.state.len() + pr.url.len() + pr.merged_at.as_ref().map_or(0, String::len)
        ).sum::<usize>();
    }
}
'''

MAIN = r'''
fn main() {
    let path = std::env::args().nth(1).expect("fixture path");
    let responses: Vec<String> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut queries = Vec::new();
    let mut observed = Vec::new();
    let query_number = std::cell::Cell::new(0);
    let linked = fetch_linked_prs_bulk_via_gh_with_observer(
        &(1..=50).collect::<Vec<_>>(),
        |query| {
            queries.push(query.to_string());
            query_number.set(queries.len());
            Ok(responses[queries.len() - 1].clone())
        },
        |issue, prs| {
            assert_eq!(query_number.get(), if issue <= 25 { 3 } else { 4 }, "complete-only publication");
            observed.push(json!([issue, prs.iter().map(summary).collect::<Vec<_>>()]));
        },
    ).expect("complete fixture");
    assert_eq!(queries.len(), 4);
    for (position, query) in queries.iter().enumerate() {
        let start = if position % 2 == 0 { 1 } else { 26 };
        for issue in 1..=50 {
            assert_eq!(query.contains(&format!("i{issue}: issue(")), (start..start + 25).contains(&issue));
            if position >= 2 && (start..start + 25).contains(&issue) {
                assert!(query.contains(&format!("after: \"next-{issue}\"")));
            }
        }
        assert_eq!(query.contains("after:"), position >= 2);
    }
    let returned: Vec<_> = linked.iter().map(|(issue, prs)|
        json!([issue, prs.iter().map(summary).collect::<Vec<_>>()])
    ).collect();
    assert_eq!(observed, returned);
    assert_eq!(returned.len(), 50);
    for (issue, prs) in &linked {
        assert_eq!(prs.len(), 10);
        for (offset, pr) in prs.iter().enumerate() {
            assert_eq!(pr.number, issue * 1000 + offset as u64);
            assert_eq!(pr.title, format!("first-{}", pr.number));
            assert_eq!(pr.state, "OPEN");
            assert!(pr.will_close_target);
            assert_eq!(pr.merged_at.as_deref(), Some("2026-11-04T00:00:00Z"));
        }
    }
    SAMPLES.with(|samples| println!("{}", json!({
        "samples": *samples.borrow(), "queries": queries, "observed": observed, "returned": returned,
    })));
}
'''


def extract(source):
    chunk = re.search(r"pub const LINKED_PR_BULK_CHUNK: usize = \d+;", source).group()
    query = source[source.index("const LINKED_PR_TIMELINE_SELECTION:"):
                   source.index("/// Linked PRs for every issue")]
    core = source[source.index("fn fetch_linked_prs_bulk_via_gh_with_observer("):]
    core = core[:core.index("\n#[cfg(test)]\nmod tests {")]
    original = chunk + "\n" + query + core
    pattern = r"(?m)^        if !next\.is_empty\(\) \{\n            chunks\.push_back\((?s:.*?)\);\n        \}"
    core, replacements = re.subn(pattern, lambda match: match.group() + "\n        record_queue(&chunks);", core)
    assert replacements == 1, "expected exactly one queue-tail instrumentation point"
    compact = "struct LinkedPrAccumulator" in core
    instrumented = PRELUDE + chunk + "\n" + query + core + (COMPACT_COUNTER if compact else "") + MAIN
    return original, instrumented


def run_probe(label, source_path, deps, rlib, work, fixture_path):
    source_bytes = source_path.read_bytes()
    original, probe = extract(source_bytes.decode("utf-8").replace("\r\n", "\n"))
    probe_path = work / f"{label}.rs"
    executable = work / (label + (".exe" if os.name == "nt" else ""))
    probe_path.write_text(probe, encoding="utf-8", newline="\n")
    command = ["rustc", "--edition=2021", "--crate-name", "linked_pr_residency_probe", str(probe_path),
               "-L", f"dependency={deps}", "--extern", f"serde_json={rlib}", "-o", str(executable)]
    subprocess.run(command, check=True)
    result = subprocess.run([str(executable), str(fixture_path)], check=True, stdout=subprocess.PIPE, text=True)
    (work / f"{label}.stdout.json").write_text(result.stdout, encoding="utf-8")
    raw = json.loads(result.stdout)
    return {
        "source_path": str(source_path), "source_sha256": digest(source_bytes),
        "extracted_source_sha256": digest(original.encode()), "probe_source_sha256": digest(probe.encode()),
        "zero_queued_raw_body_acceptance": max(sample["raw_body_bytes"] for sample in raw["samples"]) == 0,
        "samples_after_each_query": raw["samples"],
        "peak": {key: max(sample[key] for sample in raw["samples"]) for key in raw["samples"][0]},
        "queries": len(raw["queries"]), "queries_sha256": digest(encoded(raw["queries"])),
        "complete_only_publication_asserted": True,
        "observed_issues": len(raw["observed"]), "observer_order": [row[0] for row in raw["observed"]],
        "observer_sha256": digest(encoded(raw["observed"])), "returned_sha256": digest(encoded(raw["returned"])),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--current", required=True, type=Path)
    parser.add_argument("--deps-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument("--baseline-only", action="store_true")
    args = parser.parse_args()
    work = args.work_dir or Path(tempfile.mkdtemp(prefix="gwt-linked-pr-residency-"))
    work.mkdir(parents=True, exist_ok=True)
    deps = args.deps_dir.resolve()
    rlib = max(deps.glob("libserde_json-*.rlib"), key=lambda path: path.stat().st_mtime)
    fixture_bytes = fixture()
    fixture_path = work / "fixture.json"
    fixture_path.write_bytes(fixture_bytes)
    report = {
        "schema_version": 1, "metric": "queued payload UTF-8 string lengths and entry counts; not allocations or RSS",
        "fixture": {"issues": 50, "chunks": 2, "initial_events_per_issue": 100,
                    "unique_prs_per_issue": 10, "initial_body_bytes_per_event": 4096,
                    "continuation_events_per_issue": 10, "queries": 4, "sha256": digest(fixture_bytes)},
        "extraction": ["LINKED_PR_BULK_CHUNK constant", "timeline selection and query functions",
                       "bulk observer core through parser/accumulator and body_closes_issue before test module"],
        "probe_changes": ["one record_queue(&chunks) call after each existing queue-tail block",
                          "closed no-op gwt_core deadline/budget and chrono stubs; transport returns finite fixture responses",
                          "local LinkedPrSummary field-equivalent declaration; queue counters and result assertions"],
        "excluded": ["active stdout and parsed response JSON", "fixture/harness storage", "JSON object key strings",
                     "cursor/seen-cursor storage", "numeric values and capacities", "allocator overhead", "RSS", "elapsed time"],
        "compiler": subprocess.run(["rustc", "--version"], check=True, capture_output=True, text=True).stdout.strip(),
        "serde_json_rlib": {"path": str(rlib), "sha256": digest(rlib.read_bytes())},
    }
    report["baseline"] = run_probe("baseline", args.baseline, deps, rlib, work, fixture_path)
    assert report["baseline"]["peak"]["raw_body_bytes"] > 0, "baseline must witness queued body retention"
    if not args.baseline_only:
        report["current"] = run_probe("current", args.current, deps, rlib, work, fixture_path)
        assert report["current"]["peak"]["raw_body_bytes"] == 0, "compact reader must retain no queued raw body"
        assert report["current"]["peak"]["summaries"] == 500
        for key in ("queries", "queries_sha256", "observed_issues", "observer_order", "observer_sha256", "returned_sha256"):
            assert report["baseline"][key] == report["current"][key], key
        report["comparison"] = "PASS: same queries, returned summaries and complete observer order; queued raw bodies eliminated"
    else:
        report["comparison"] = "BASELINE ONLY: queued raw-body witness established; current reader not measured"
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"comparison": report["comparison"], "baseline_peak": report["baseline"]["peak"],
                      "current_peak": report.get("current", {}).get("peak"), "work_dir": str(work)}))


if __name__ == "__main__":
    main()
