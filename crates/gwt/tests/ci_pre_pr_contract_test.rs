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
    assert!(lint["jobs"].get("coverage").is_none());
    let caller = workflow("coverage-ci.yml");
    for event in ["push", "pull_request", "merge_group"] {
        assert!(caller["on"].get(event).is_some());
    }
    assert_eq!(
        caller["jobs"]["coverage"]["uses"],
        "./.github/workflows/coverage.yml"
    );
    assert!(caller["jobs"]["coverage"].get("name").is_none());
    assert!(caller["jobs"]["coverage"].get("needs").is_none());
    assert!(caller["jobs"]["coverage"].get("if").is_none());
    assert!(caller["jobs"]["coverage"]
        .get("continue-on-error")
        .is_none());
    assert_eq!(coverage["jobs"]["rust-coverage"]["name"], "Rust Coverage");
    assert_eq!(coverage["jobs"]["rust-coverage"]["needs"], "coverage-shard");
    assert_eq!(coverage["jobs"]["rust-coverage"]["if"], "${{ always() }}");
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
    let download = steps
        .iter()
        .find(|s| s["name"] == "Download pre-shard head summary")
        .unwrap();
    assert_eq!(download["with"]["name"], "workspace-tests-1");
    assert_eq!(
        download["with"]["path"],
        "target/coverage-shards/workspace-tests-1"
    );
    assert!(steps
        .iter()
        .any(|s| s["run"].as_str() == Some("test \"$COVERAGE_SHARDS_RESULT\" = success")));
    assert!(steps.iter().any(|s| s["run"].as_str()
        == Some("python3 scripts/ci_workspace_tests.py target/coverage-shards --shards 6")));
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
fn coverage_keeps_pre_shard_pr_heads_on_their_supported_full_workspace_transport() {
    let coverage = workflow("coverage.yml");
    for job in ["coverage-shard", "rust-coverage"] {
        let steps = coverage["jobs"][job]["steps"].as_sequence().unwrap();
        let mode = steps.iter().find(|s| s["id"] == "transport").unwrap();
        let run = mode["run"].as_str().unwrap();
        assert!(run.contains("git show HEAD:.github/workflows/coverage.yml"));
        assert!(run.contains("grep -q '^  coverage-shard:'"));
        assert!(run.contains("sharded=true") && run.contains("sharded=false"));
        assert!(!run.contains("|| true"));
    }
    let steps = coverage["jobs"]["coverage-shard"]["steps"]
        .as_sequence()
        .unwrap();
    let legacy = steps
        .iter()
        .find(|s| s["name"] == "Run pre-shard head coverage")
        .unwrap();
    assert_eq!(
        legacy["if"],
        "${{ steps.transport.outputs.sharded == 'false' && matrix.shard == 1 }}"
    );
    let run = legacy["run"].as_str().unwrap();
    assert!(run.contains("node scripts/coverage-summary.mjs --output-path target/coverage-export/coverage-summary.json --nextest -- --workspace --all-features"));
    assert!(!run.contains("--partition") && !run.contains("--export-dir"));
    assert_eq!(legacy["env"]["NEXTEST_PROFILE"], "coverage");
    let legacy_lcov = steps
        .iter()
        .find(|s| s["name"] == "Generate pre-shard head lcov")
        .unwrap();
    let run = legacy_lcov["run"].as_str().unwrap();
    assert!(run.contains("coverage-summary.json.profiles.json"));
    assert!(run.contains("--failure-mode all"));
    assert!(run.contains("--lcov --output-path target/coverage-export/lcov.info"));
    let steps = coverage["jobs"]["rust-coverage"]["steps"]
        .as_sequence()
        .unwrap();
    for name in [
        "Verify the complete workspace test union",
        "Generate workspace summary",
    ] {
        let step = steps.iter().find(|s| s["name"] == name).unwrap();
        assert_eq!(
            step["if"],
            "${{ steps.transport.outputs.sharded == 'true' }}"
        );
    }
    let legacy = steps
        .iter()
        .find(|s| s["name"] == "Restore pre-shard head summary")
        .unwrap();
    assert_eq!(
        legacy["if"],
        "${{ steps.transport.outputs.sharded == 'false' }}"
    );
    assert_eq!(legacy["run"], "cp target/coverage-shards/workspace-tests-1/coverage-summary.json target/coverage-summary.json");
    for step in steps.iter().filter(|s| {
        s["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("Enforce coverage"))
    }) {
        assert!(
            step.get("if").is_none(),
            "both transports must enforce the same thresholds"
        );
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
    let shard = &coverage["jobs"]["coverage-shard"];
    assert_eq!(shard["strategy"]["fail-fast"], false);
    assert_eq!(
        shard["strategy"]["matrix"]["shard"],
        serde_yaml::from_str::<Value>("[1, 2, 3, 4, 5, 6]").unwrap()
    );
    assert_eq!(coverage["env"]["CARGO_PROFILE_DEV_DEBUG"], "0");
    assert_eq!(coverage["env"]["CARGO_PROFILE_TEST_DEBUG"], "0");
    let steps = shard["steps"].as_sequence().unwrap();
    assert!(
        steps.iter().any(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|action| action.starts_with("dtolnay/rust-toolchain@"))
                && step["with"]["components"]
                    .as_str()
                    .is_some_and(|components| {
                        components.split(',').any(|part| part.trim() == "rustfmt")
                    })
        }),
        "coverage workspace tests invoke cargo fmt --version"
    );
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
    assert_eq!(cache["with"]["key"], "coverage-instrumented-workspace-v2");
    let saves = cache["with"]["save-if"].as_str().unwrap();
    assert!(saves.contains("github.ref == 'refs/heads/develop'"));
    assert!(saves.contains("github.event_name == 'schedule'"));
    assert!(saves.contains("github.event_name == 'workflow_dispatch'"));
    assert!(saves.contains("matrix.shard == 1"));
    assert!(steps
        .iter()
        .any(|s| s["with"]["tool"].as_str() == Some("cargo-nextest@0.9.146")));
    let summary = steps.iter().find(|s| s["run"].as_str() == Some("node scripts/coverage-summary.mjs --nextest --partition slice:${{ matrix.shard }}/6 --export-dir target/coverage-export -- --workspace --all-features")).unwrap();
    assert_eq!(summary["env"]["LLVM_PROFILE_FILE_NAME"], "gwt-%8m.profraw");
    assert_eq!(summary["env"]["NEXTEST_PROFILE"], "coverage");
    assert!(steps
        .iter()
        .any(|s| s["with"]["name"].as_str() == Some("workspace-tests-${{ matrix.shard }}")));
    let nextest: toml::Value =
        toml::from_str(include_str!("../../../.config/nextest.toml")).unwrap();
    assert_eq!(
        nextest["profile"]["coverage"]["test-threads"].as_integer(),
        Some(16)
    );
    assert_eq!(
        nextest["profile"]["coverage"]["junit"]["path"].as_str(),
        Some("junit.xml")
    );
}

#[test]
fn unused_dependency_check_blocks_the_required_lint_job() {
    let lint = workflow("lint.yml");
    let job = &lint["jobs"]["lint"];
    assert_eq!(job["name"], "Clippy & Rustfmt");
    let steps = job["steps"].as_sequence().unwrap();
    let install = steps
        .iter()
        .find(|step| step["with"]["tool"].as_str() == Some("cargo-machete@0.9.2"))
        .expect("install a pinned cargo-machete in the required lint job");
    assert_eq!(install["uses"], "taiki-e/install-action@v2");
    let check = steps
        .iter()
        .find(|step| step["run"].as_str() == Some("cargo machete --with-metadata crates"))
        .expect("check unused dependencies with Cargo metadata");
    for entry in [job, install, check] {
        assert!(
            entry.get("if").is_none(),
            "the dependency gate must always run"
        );
        assert!(
            entry.get("continue-on-error").is_none(),
            "unused dependencies must fail the required lint job"
        );
    }
}
