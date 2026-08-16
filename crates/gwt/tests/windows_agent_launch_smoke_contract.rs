//! Issue #3566: authenticated Windows official-provider smoke contract.
//!
//! SPEC-1921 Phase 75 remains the reference design for the existing E2E.

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
        "@openai/codex",
        "@anthropic-ai/claude-code",
        "npm.cmd",
        "npx.cmd",
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
        "requested_selector",
        "resolved_exact_version",
        "session_fingerprint_sha256",
        "ConvertTo-Json",
    ] {
        assert!(
            source.contains(required),
            "official-provider smoke must contain {required:?}"
        );
    }

    for provider in ["codex", "claude"] {
        for selector in ["latest", "exact"] {
            let case = format!("{provider}/{selector}");
            assert!(
                source.contains(&case),
                "official-provider smoke must name the {case} case explicitly"
            );
        }
    }

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
fn windows_launch_e2e_preflights_the_loopback_registry_with_bounded_diagnostics() {
    let root = repo_root();
    let e2e_path = root.join("crates/gwt/tests/windows_agent_launch_e2e.rs");
    let e2e = fs::read_to_string(&e2e_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", e2e_path.display()));
    let fixture_path = root.join("crates/gwt-core/src/test_support.rs");
    let fixture = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));
    let workflow_path = root.join(".github/workflows/test.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", workflow_path.display()));

    for required in [
        "gwt_agent::prepare::probe_host_runner_with_timeout",
        "HostRunnerProbeKind::Runner",
        "npm.cmd",
        "config",
        "get",
        "registry",
        "view",
        "version",
        "metadata preflight",
        "--json",
        "loopback registry preflight failure",
        "accepted_connection_count",
        "header_complete_request_count",
        "accepted_connection_delta",
        "header_complete_request_delta",
        "request_snapshot",
        "registry_request_diagnostic_snapshot",
        "probe_registry_health",
        "tracing_subscriber",
        "with_test_writer",
        "prepare_agent_launch",
    ] {
        assert!(
            e2e.contains(required),
            "Windows launch E2E must contain {required:?}"
        );
    }
    assert!(
        e2e.lines().any(|line| {
            line.trim() == "const TEST_REGISTRY_HEALTH_TIMEOUT: Duration = Duration::from_secs(5);"
        }),
        "the registry healthcheck deadline must stay explicitly bounded at its constant definition"
    );
    let preflight = e2e
        .split("fn assert_loopback_registry_preflight(")
        .nth(1)
        .and_then(|tail| tail.split("\nfn route_capture_path(").next())
        .expect("loopback registry preflight source");
    assert!(
        e2e.lines().any(|line| {
            line.trim() == "const TEST_PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(60);"
        }) && preflight.matches("TEST_PREFLIGHT_TIMEOUT,").count() >= 2,
        "the npm config and metadata preflights must share the explicit 60-second bound"
    );
    for required in [
        "if requested_selector == \"latest\"",
        "provider.package()",
        "format!(\"{}@latest\", provider.package())",
        "serde_json::from_str::<String>",
        "fixture.exact_version",
        "metadata_request_count_before",
        "metadata_outcome",
        "metadata preflight process failed",
        "reached_packument",
        ".get(metadata_request_count_before..)",
        "metadata preflight did not reach the loopback packument",
    ] {
        assert!(
            preflight.contains(required),
            "latest-selector metadata preflight must contain {required:?}"
        );
    }
    assert!(
        !preflight.contains("ToolRuntimeProvenance"),
        "the test-only metadata preflight must not inject resolved provenance"
    );
    let metadata_success_guard_offset = preflight
        .find("if !metadata_outcome.success")
        .expect("metadata preflight process failure guard");
    let metadata_parse_offset = preflight
        .find("serde_json::from_str::<String>")
        .expect("metadata preflight JSON parse");
    assert!(
        metadata_success_guard_offset < metadata_parse_offset,
        "metadata process failures must retain their primary outcome before JSON parsing"
    );
    let clone_offset = preflight
        .find("let mut preflight_env = launch_env.clone();")
        .expect("preflight must clone the production-equivalent launch environment");
    let registry_remove_offset = preflight
        .find("preflight_env.remove(\"NPM_CONFIG_REGISTRY\")")
        .expect("preflight must remove the expected URL from probe redaction inputs");
    let probe_offset = preflight
        .find("gwt_agent::prepare::probe_host_runner_with_timeout")
        .expect("preflight must call the public bounded probe seam");
    assert!(
        clone_offset < registry_remove_offset && registry_remove_offset < probe_offset,
        "preflight must remove NPM_CONFIG_REGISTRY before calling the bounded probe"
    );
    assert!(
        preflight[probe_offset..].contains("&preflight_env,"),
        "the bounded probe must receive the redaction-safe preflight environment"
    );
    assert!(
        preflight.contains("HostRunnerProbeKind::Runner")
            && !preflight.contains("HostRunnerProbeKind::Metadata"),
        "the config preflight must use a non-Metadata trace label"
    );
    let strict_compare_offset = preflight
        .find("observed_registry != fixture.registry_url")
        .expect("preflight must strictly compare the configured registry URL");
    let health_probe_offset = preflight
        .find("fixture.probe_registry_health")
        .expect("preflight must run the bounded loopback HTTP healthcheck");
    assert!(
        strict_compare_offset < health_probe_offset,
        "the npm config result must match before the fixture healthcheck runs"
    );

    let diagnostic_snapshot = e2e
        .split("fn registry_request_diagnostic_snapshot(")
        .nth(1)
        .and_then(|tail| {
            tail.split("\nfn assert_loopback_registry_preflight(")
                .next()
        })
        .expect("redacted registry request diagnostic snapshot helper");
    for required in ["request.method", "request.path"] {
        assert!(
            diagnostic_snapshot.contains(required),
            "diagnostic snapshot helper must retain {required}"
        );
    }
    assert!(
        !diagnostic_snapshot.contains("headers")
            && !e2e.contains("let request_snapshot = fixture.requests();"),
        "failure diagnostics must not Debug-print raw registry request headers"
    );

    for required in [
        "AtomicUsize",
        "accepted_connection_count",
        "probe_registry_health",
        "GET /-/gwt-health HTTP/1.1",
    ] {
        assert!(
            fixture.contains(required),
            "loopback registry fixture must expose {required:?}"
        );
    }
    let healthcheck = fixture
        .split("pub fn probe_registry_health(")
        .nth(1)
        .and_then(|tail| tail.split("\n}\n\n#[cfg(windows)]\nimpl Drop").next())
        .expect("loopback registry healthcheck source");
    for required in [
        "let deadline = Instant::now()",
        ".checked_add(timeout)",
        "remaining_timeout",
        "TcpStream::connect_timeout",
        "set_read_timeout",
        "set_write_timeout",
        "let mut written = 0;",
        "while written < request.len()",
        "written += count;",
        "Duration::from_millis(1)",
        "ErrorKind::TimedOut",
        "ErrorKind::WouldBlock",
    ] {
        assert!(
            healthcheck.contains(required),
            "loopback registry healthcheck must contain {required:?}"
        );
    }
    assert_eq!(
        healthcheck.matches("let deadline = Instant::now()").count(),
        1,
        "registry healthcheck must create exactly one end-to-end deadline"
    );
    assert!(
        healthcheck.matches("remaining_timeout()?").count() >= 4,
        "connect, write, each read, and final success must recompute remaining deadline time"
    );
    assert!(
        !healthcheck.contains("connect_timeout(&self.address, timeout)")
            && !healthcheck.contains("Some(timeout)"),
        "registry healthcheck must not restart the full timeout for individual I/O operations"
    );
    let request_handler = fixture
        .split("fn serve_npm_request(")
        .nth(1)
        .and_then(|tail| tail.split("\nfn percent_decode_registry_path(").next())
        .expect("loopback registry request handler source");
    for required in [
        "let mut header_complete = false;",
        "header_complete = true;",
        "raw.len() > 64 * 1024",
        "if !header_complete",
        "UnexpectedEof",
        "InvalidData",
        "\"/-/gwt-health\"",
        "200 OK",
    ] {
        assert!(
            request_handler.contains(required),
            "loopback registry request handler must contain {required:?}"
        );
    }
    let complete_guard_offset = request_handler
        .find("if !header_complete")
        .expect("partial header guard");
    let request_record_offset = request_handler
        .find(".push(NpmRegistryRequest")
        .expect("completed request recording");
    assert!(
        complete_guard_offset < request_record_offset,
        "only header-complete requests may enter the fixture request log"
    );

    assert!(
        workflow.contains(
            "cargo test -p gwt --test windows_agent_launch_e2e -- --ignored --test-threads=1 --nocapture"
        ),
        "Windows launch E2E must retain tracing output in CI with --nocapture"
    );
}
