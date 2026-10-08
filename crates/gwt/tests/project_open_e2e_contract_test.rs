//! Issue #3754 — project-open browser coverage must exercise the real
//! rendered entry surfaces instead of hiding them in the shared harness.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn live_harness_requires_explicit_project_surface_suppression() {
    let helper = read("crates/gwt/playwright/tests/_helpers/live-gwt.ts");

    assert!(
        helper.contains("suppressProjectSurfaces?: boolean"),
        "the shared live helper must expose an explicit opt-in for tests that intentionally hide project entry surfaces"
    );
    assert!(
        helper.contains("if (options.suppressProjectSurfaces)"),
        "picker/onboarding suppression must be conditional instead of part of the default goto path"
    );
    assert!(
        helper.contains("suppressProjectSurfaces: Boolean(options.suppressProjectSurfaces)"),
        "the DOM hidden-state mutation must receive the same explicit option as the CSS suppression"
    );
}

#[test]
fn project_open_surfaces_have_ci_runnable_browser_coverage() {
    let spec = read("crates/gwt/playwright/tests/project-open-surfaces.spec.ts");

    assert!(
        spec.contains("gotoLiveGwt(page, APP_URL"),
        "the project-open spec must exercise the shared live harness against the rendered frontend"
    );
    assert!(
        !spec.contains("test.skip("),
        "the deterministic project-open spec must run in the existing CI browser job"
    );
    for contract in [
        "#project-picker",
        "#picker-open-project",
        "#picker-clone-project",
        "open_project_dialog",
        "non_repo",
        "#project-onboarding",
        "elementFromPoint",
        "pageerror",
        "message.type() === \"error\"",
    ] {
        assert!(
            spec.contains(contract),
            "project-open browser coverage must include `{contract}`"
        );
    }
}

#[test]
fn playwright_config_does_not_claim_unused_visual_snapshots() {
    let config = read("crates/gwt/playwright/playwright.config.ts");

    for dead_contract in ["snapshotDir", "snapshotPathTemplate", "toHaveScreenshot"] {
        assert!(
            !config.contains(dead_contract),
            "unused visual-regression setting `{dead_contract}` must not remain in the Playwright config"
        );
    }
}

#[test]
fn frontend_ci_runs_the_project_open_browser_spec() {
    let workflow = read(".github/workflows/test.yml");
    let runner = read("scripts/run-visual-tests.sh");
    let frontend_job = workflow
        .split("\n  test-frontend:")
        .nth(1)
        .expect("test.yml must keep the test-frontend job")
        .split("\n  test-")
        .next()
        .expect("split always yields a first segment");

    assert!(
        frontend_job.contains("bash scripts/run-visual-tests.sh"),
        "the frontend CI job must execute every non-skipped Playwright spec"
    );
    assert!(
        !runner.contains("playwright/snapshots"),
        "the Playwright runner must not keep dead snapshot-directory plumbing"
    );
}
