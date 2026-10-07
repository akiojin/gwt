//! Coverage delegated by pre-PR verification must block the existing CI gate.

use serde_yaml::Value;

fn workflow(name: &str) -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    serde_yaml::from_str(
        &std::fs::read_to_string(root.join(".github/workflows").join(name)).unwrap(),
    )
    .unwrap()
}

#[test]
fn pr_coverage_failure_blocks_the_existing_required_lint_check() {
    let coverage = workflow("coverage.yml");
    assert!(
        coverage["on"].get("workflow_call").is_some(),
        "coverage must be callable by PR CI"
    );
    let lint = workflow("lint.yml");
    assert_eq!(
        lint["jobs"]["coverage"]["uses"],
        "./.github/workflows/coverage.yml"
    );
    assert_eq!(lint["jobs"]["lint"]["needs"], "coverage");
    assert_eq!(lint["jobs"]["lint"]["if"], "${{ !cancelled() }}");
    assert_eq!(lint["jobs"]["lint"]["name"], "Clippy & Rustfmt");
    let steps = lint["jobs"]["lint"]["steps"].as_sequence().unwrap();
    assert_eq!(
        steps[0]["env"]["COVERAGE_RESULT"],
        "${{ needs.coverage.result }}"
    );
    assert_eq!(steps[0]["run"], "test \"$COVERAGE_RESULT\" = success");
}

#[test]
fn coverage_measures_the_requested_source_without_cancelling_its_caller() {
    let coverage = workflow("coverage.yml");
    let group = coverage["concurrency"]["group"].as_str().unwrap();
    assert!(group.starts_with("coverage-"));
    assert!(group.contains("github.event.pull_request.number || github.ref"));
    let steps = coverage["jobs"]["rust-coverage"]["steps"]
        .as_sequence()
        .unwrap();
    let checkout = steps
        .iter()
        .find(|s| {
            s["uses"]
                .as_str()
                .is_some_and(|s| s.starts_with("actions/checkout@"))
        })
        .unwrap();
    let source = checkout["with"]["ref"].as_str().unwrap();
    assert!(source.contains("github.event.pull_request.head.sha"));
    assert!(source.contains("github.sha"));
    assert_ne!(source, "develop");
    for (threshold, scope) in [("90", "--scope"), ("80", "--scope-exclude")] {
        assert!(steps.iter().any(|s| s["run"].as_str().is_some_and(|run| run
            .contains("node scripts/check-coverage-threshold.mjs")
            && run.contains(threshold)
            && run.contains(scope))));
    }
}
