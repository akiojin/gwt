//! SPEC-1921 Phase 75: authenticated Windows official-provider smoke contract.

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
