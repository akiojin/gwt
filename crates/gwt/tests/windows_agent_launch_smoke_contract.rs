//! Issue #3566: authenticated Windows official-provider smoke contract.
//!
//! SPEC-1921 Phase L1b: installed runner launch coverage.

use std::{fs, path::PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn windows_official_provider_smoke_is_explicit_sanitized_and_checkout_local() {
    let script_path = repo_root().join("scripts/windows-agent-launch-smoke.ps1");
    let source = fs::read_to_string(&script_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", script_path.display()));

    for required in [
        "codex/installed",
        "claude/installed",
        "observed_version",
        "runner_kind = \"installed\"",
        "cargo",
        "build",
        "--bin",
        "gwt",
        "gwtd",
        "target/debug/gwt.exe",
        "target/debug/gwtd.exe",
        "GWT_SMOKE_FRESH_OK",
        "GWT_SMOKE_RESUME_OK",
        "SessionStart",
        "authenticated_session_start",
        "same_provider_session_resume",
        "session_fingerprint_sha256",
        "ConvertTo-Json",
    ] {
        assert!(
            source.contains(required),
            "official-provider smoke must contain {required:?}"
        );
    }

    assert!(
        !source.contains("npx.cmd") && !source.contains("Resolve-LatestExactVersion"),
        "installed-provider smoke must never select or dispatch a package runner"
    );

    assert!(
        source.contains("credential") && source.contains("throw"),
        "missing provider credentials must fail explicitly rather than skip"
    );
    assert!(
        source.contains("function Resolve-ExecutablePath")
            && source.contains("-CommandType Application")
            && source.contains("$startInfo.FileName = Resolve-ExecutablePath -FilePath $FilePath"),
        "bare .cmd launchers must be resolved to absolute application paths before ProcessStartInfo starts them"
    );
    assert!(
        source.contains("[IO.Path]::GetExtension($startInfo.FileName)")
            && source.contains("'/d /v:off /s /c \"'")
            && source.contains("-FilePath \"cmd.exe\""),
        "installed .cmd runners must execute through cmd.exe with delayed expansion disabled"
    );
    assert!(
        source.contains("[AllowEmptyString()][string[]]$Arguments"),
        "the process helper must preserve Claude Code's required empty --tools argument"
    );
    assert!(
        source.contains("$startInfo.Environment[$name] = $EnvironmentOverrides[$name]")
            && source.contains("Join-Path $codexHome \"hooks.json\"")
            && source.contains("CODEX_HOME = $codexHome")
            && source.contains("Copy-Item -LiteralPath $authSource"),
        "Codex smoke must use an isolated CODEX_HOME with user-level hooks and only copy the credential artifact"
    );
    assert!(
        source.contains("$assistantMarkerObserved")
            && source.contains("$itemType -eq \"agent_message\"")
            && source.contains("$eventType -eq \"assistant\" -and $blockType -eq \"text\"")
            && !source.contains("$serialized.Contains($ExpectedMarker"),
        "provider markers must be accepted only from assistant output, never an echoed user prompt"
    );
    let protect_offset = source
        .find("Protect-CurrentUserDirectory -Path $temporaryRoot")
        .expect("temporary credential directory must receive a protected ACL");
    let official_case_call_offset = protect_offset
        + source[protect_offset..]
            .find("Invoke-OfficialCase")
            .expect("protected temporary root must be used by the official smoke cases");
    assert!(
        source.contains("SetAccessRuleProtection($true, $false)")
            && source
                .contains("Temporary credential directory grants access outside the current user")
            && protect_offset < official_case_call_offset,
        "the current-user-only ACL must be applied and verified before credentials are copied"
    );
    assert!(
        !source.contains("Join-Path $CaseRoot \".codex\"")
            && !source.contains("Join-Path $codexDir \"hooks.json\""),
        "Codex smoke must not depend on project-local hook discovery or ambient project trust"
    );
    assert!(
        source.contains("Remove-Item -LiteralPath $temporaryRoot -Recurse -Force"),
        "raw provider output must always be discarded"
    );
    assert!(
        !source.contains("Get-ChildItem Env:") && !source.contains("ConvertTo-Json $env:"),
        "the smoke must never serialize the ambient environment"
    );
}

#[test]
fn windows_launch_e2e_bounds_marker_waits_with_saturation_aware_diagnosable_budgets() {
    let root = repo_root();
    let e2e_path = root.join("crates/gwt/tests/windows_agent_launch_e2e.rs");
    let e2e = fs::read_to_string(&e2e_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", e2e_path.display()));
    let fixture_path = root.join("crates/gwt-core/src/test_support.rs");
    let fixture = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));

    // Issue #3656 AC-2: the agent-ready marker wait must carry a named,
    // saturation-aware budget instead of an inline 60-second literal. The
    // budget lives in the same constants block as the preflight budgets so
    // every wait surface of this E2E is declared in one place (AC-4).
    assert!(
        e2e.lines().any(|line| {
            line.trim() == "const TEST_AGENT_READY_TIMEOUT: Duration = Duration::from_secs(180);"
        }),
        "the agent-ready marker budget must be a named 180-second constant"
    );
    assert!(
        e2e.lines().any(|line| {
            line.trim() == "const TEST_PROVIDER_EXIT_TIMEOUT: Duration = Duration::from_secs(30);"
        }),
        "the post-marker provider exit budget must be a named 30-second constant"
    );
    assert!(
        e2e.contains(
            "read_pty_until_marker(pty, \"phase75-agent-ready\", TEST_AGENT_READY_TIMEOUT)"
        ),
        "the route marker wait must consume the named agent-ready budget"
    );
    assert!(
        !e2e.contains("\"phase75-agent-ready\", Duration::from_secs(60)"),
        "the route marker wait must not hardcode a 60-second budget inline"
    );

    // Issue #3656 AC-2: elapsed-to-marker must be logged so CI keeps a
    // measurable latency distribution for regression detection (the workflow
    // retains --nocapture).
    assert!(
        e2e.contains("phase75 marker timing:")
            && e2e.contains("elapsed_ms=")
            && e2e.contains("budget_ms="),
        "marker arrival latency must be logged with elapsed and budget fields"
    );

    let wait_fn = e2e
        .split("fn read_pty_until_marker(")
        .nth(1)
        .and_then(|tail| tail.split("\nfn launch_env_with_real_gwtd(").next())
        .expect("read_pty_until_marker source");
    // Issue #3656 AC-1: every timeout path must name the marker, report the
    // waited/budget durations, and summarize the received output through the
    // shared sanitizer instead of Debug-printing raw escape sequences.
    assert!(
        wait_fn.contains("timed out waiting for PTY marker"),
        "the marker timeout must still name the awaited marker"
    );
    assert!(
        wait_fn.contains("waited=") && wait_fn.contains("budget="),
        "the marker timeout must report how long it actually waited and its budget"
    );
    assert!(
        wait_fn
            .matches("summarize_pty_output_for_diagnostics(")
            .count()
            >= 3,
        "timeout, EOF, and exit-stall diagnostics must all use the shared output sanitizer"
    );
    assert!(
        !wait_fn.contains("output={:?}"),
        "marker wait diagnostics must not Debug-print raw un-sanitized PTY output"
    );
    assert!(
        wait_fn.contains("Instant::now() + TEST_PROVIDER_EXIT_TIMEOUT")
            && !wait_fn.contains("Duration::from_secs(10)"),
        "the post-marker provider exit wait must consume the named exit budget"
    );

    // Issue #3656 AC-1/AC-4: the sanitizer is shared cross-platform machinery
    // in gwt-core test_support with its own unit coverage, so the diagnostics
    // stay testable on every host.
    assert!(
        fixture.contains("pub fn summarize_pty_output_for_diagnostics("),
        "gwt-core test_support must expose the shared PTY output sanitizer"
    );
    assert!(
        fixture.contains("fn summarize_pty_output_for_diagnostics_strips_escape_sequences")
            && fixture
                .contains("fn summarize_pty_output_for_diagnostics_bounds_long_output_to_a_tail"),
        "the shared PTY output sanitizer must keep cross-platform unit coverage"
    );
}

#[test]
fn windows_launch_e2e_installs_fixture_before_direct_launch_without_registry_access() {
    let root = repo_root();
    let e2e = fs::read_to_string(root.join("crates/gwt/tests/windows_agent_launch_e2e.rs"))
        .expect("read Windows E2E source");
    let workflow =
        fs::read_to_string(root.join(".github/workflows/test.yml")).expect("read test workflow");
    for required in [
        "windows_official_provider_launch_uses_installed_direct_runner",
        "install_fixture_provider",
        "spawn_logged_blocking_with_deadline",
        "TEST_FIXTURE_INSTALL_TIMEOUT",
        "npm.cmd",
        "--prefix",
        "--ignore-scripts",
        "node_modules",
        "installed_bin",
        "prepare_agent_launch",
        "persisted.tool_runtime_provenance.is_none()",
        "registry_requests_after_install",
        "installed launches must not access the registry",
    ] {
        assert!(
            e2e.contains(required),
            "installed Windows E2E must contain {required:?}"
        );
    }
    assert!(
        e2e.split_whitespace()
            .collect::<String>()
            .contains("fixture.requests().len(),registry_requests_after_install,"),
        "registry requests must remain unchanged after installed launches"
    );
    for removed in [
        ".version(requested_selector)",
        "HostRunnerProbeKind::Runner",
        "ToolRuntimeProvenance {",
        "SELECTOR_ENV",
    ] {
        assert!(
            !e2e.contains(removed),
            "Windows launch must not retain {removed:?}"
        );
    }
    assert!(
        workflow.contains("name: Test (Windows agent launch)")
            && workflow.contains("for provider in codex claude; do")
            && !workflow.contains("GWT_WINDOWS_AGENT_SELECTOR"),
        "required Windows job must run both installed providers without selector shards"
    );
    assert!(
        workflow.contains(
            "cargo test -p gwt --test windows_agent_launch_e2e -- --ignored --test-threads=1 --nocapture"
        ),
        "Windows launch E2E must retain native execution and tracing output in CI"
    );
}
