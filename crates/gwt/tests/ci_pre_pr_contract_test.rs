//! Coverage and lint must run independently under separate required CI checks.

use serde_yaml::Value;

fn workflow(name: &str) -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    serde_yaml::from_str(
        &std::fs::read_to_string(root.join(".github/workflows").join(name)).unwrap(),
    )
    .unwrap()
}

#[test]
fn pr_coverage_and_lint_run_independently_under_stable_check_names() {
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
    assert!(lint["jobs"]["coverage"].get("name").is_none());
    assert!(lint["jobs"]["coverage"].get("needs").is_none());
    assert!(lint["jobs"]["coverage"].get("if").is_none());
    assert!(lint["jobs"]["coverage"].get("continue-on-error").is_none());
    assert_eq!(coverage["jobs"]["rust-coverage"]["name"], "Rust Coverage");
    assert!(coverage["jobs"]["rust-coverage"].get("if").is_none());
    assert!(coverage["jobs"]["rust-coverage"]
        .get("continue-on-error")
        .is_none());
    assert!(lint["jobs"]["lint"].get("needs").is_none());
    assert!(lint["jobs"]["lint"].get("if").is_none());
    assert_eq!(lint["jobs"]["lint"]["name"], "Clippy & Rustfmt");
    let steps = lint["jobs"]["lint"]["steps"].as_sequence().unwrap();
    assert!(!steps.iter().any(|step| step["run"]
        .as_str()
        .is_some_and(|run| run.contains("COVERAGE_RESULT"))));
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

#[test]
fn rust_ci_uses_the_exact_repository_toolchain() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pin = std::fs::read_to_string(root.join("rust-toolchain.toml"))
        .expect("pin the repository Rust version in rust-toolchain.toml");
    let channel = pin
        .lines()
        .find_map(|line| line.trim().strip_prefix("channel = "))
        .and_then(|value| value.strip_prefix('"'))
        .and_then(|value| value.strip_suffix('"'))
        .expect("declare a quoted channel in rust-toolchain.toml");
    assert_eq!(channel.split('.').count(), 3, "pin an exact Rust release");
    assert!(channel
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())));
    let expected = format!("dtolnay/rust-toolchain@{channel}");
    let mut toolchain_steps = 0;
    for entry in std::fs::read_dir(root.join(".github/workflows")).unwrap() {
        let path = entry.unwrap().path();
        if !matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        for line in raw.lines() {
            let line = line.trim();
            if let Some(action) = line.strip_prefix("- uses: dtolnay/rust-toolchain@") {
                toolchain_steps += 1;
                assert_eq!(
                    format!("dtolnay/rust-toolchain@{action}"),
                    expected,
                    "{} must match rust-toolchain.toml",
                    path.display()
                );
            }
            assert!(
                !line.contains("rustup run stable") && !line.contains("cargo +stable"),
                "{} must not bypass the repository pin: {line}",
                path.display()
            );
        }
    }
    assert!(toolchain_steps > 0, "expected Rust CI toolchain steps");
}

#[test]
fn coverage_caches_instrumented_builds_and_runs_the_full_inventory_with_nextest() {
    let coverage = workflow("coverage.yml");
    let steps = coverage["jobs"]["rust-coverage"]["steps"]
        .as_sequence()
        .unwrap();
    let toolchain = steps
        .iter()
        .find(|s| {
            s["uses"]
                .as_str()
                .is_some_and(|action| action.starts_with("dtolnay/rust-toolchain@"))
        })
        .unwrap();
    // Verification admission tests execute the Light `cargo fmt --version` command.
    assert!(toolchain["with"]["components"]
        .as_str()
        .unwrap()
        .split(',')
        .any(|component| component.trim() == "rustfmt"));
    let cache = steps
        .iter()
        .find(|s| {
            s["uses"]
                .as_str()
                .is_some_and(|s| s.starts_with("Swatinem/rust-cache@"))
        })
        .unwrap();
    assert_eq!(cache["with"]["workspaces"], ". -> target/llvm-cov-target");
    assert_eq!(
        cache["with"]["cache-workspace-crates"].as_bool(),
        Some(true)
    );
    assert_eq!(cache["with"]["key"], "coverage-instrumented-workspace-v1");
    let saves = cache["with"]["save-if"].as_str().unwrap();
    assert!(saves.contains("github.ref == 'refs/heads/develop'"));
    assert!(saves.contains("github.event_name == 'schedule'"));
    assert!(saves.contains("github.event_name == 'workflow_dispatch'"));
    assert!(steps
        .iter()
        .any(|s| s["with"]["tool"].as_str() == Some("cargo-nextest@0.9.146")));
    let summary = steps.iter().find(|s| s["run"].as_str() == Some("node scripts/coverage-summary.mjs --output-path target/coverage-summary.json --nextest -- --workspace --all-features")).unwrap();
    assert_eq!(summary["env"]["LLVM_PROFILE_FILE_NAME"], "gwt-%8m.profraw");
    assert_eq!(summary["env"]["NEXTEST_PROFILE"], "coverage");
    let nextest: toml::Value =
        toml::from_str(include_str!("../../../.config/nextest.toml")).unwrap();
    assert_eq!(
        nextest["profile"]["coverage"]["test-threads"].as_integer(),
        Some(16)
    );
}
