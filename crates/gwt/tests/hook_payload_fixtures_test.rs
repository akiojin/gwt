//! Issue #4779 (parent #4771): real provider hook payloads must keep passing
//! the hook parser entry, so a provider-side contract drift fails CI instead
//! of every hook call at runtime.
//!
//! Layout: `tests/fixtures/hook_payloads/<provider>/<provider version>/<Event>.json`.
//! Each file is the raw stdin a real session delivered to a hook, with only
//! secret values (session / prompt ids, absolute paths) replaced. Never add a
//! hand-written payload here.

use std::path::{Path, PathBuf};

use gwt::cli::hook::HookEvent;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hook_payloads")
}

/// Every fixture as (provider, version, event, raw payload).
fn fixtures() -> Vec<(String, String, String, String)> {
    let mut out = Vec::new();
    for provider in std::fs::read_dir(fixture_root()).unwrap() {
        let provider = provider.unwrap().path();
        if !provider.is_dir() {
            continue;
        }
        for version in std::fs::read_dir(&provider).unwrap() {
            let version = version.unwrap().path();
            for file in std::fs::read_dir(&version).unwrap() {
                let file = file.unwrap().path();
                assert_eq!(file.extension().and_then(|e| e.to_str()), Some("json"));
                let name = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
                out.push((
                    name(&provider),
                    name(&version),
                    file.file_stem().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&file).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

/// Secret-scrubbing lint: ids are placeholders, absolute paths live under
/// `/redacted/`, and nothing looks like a credential.
fn assert_scrubbed(label: &str, value: &serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            for marker in [
                "/Users/",
                "/home/",
                "/private/",
                "/tmp/",
                ":\\",
                "Bearer ",
                "xai-",
                "sk-",
            ] {
                assert!(
                    !s.contains(marker),
                    "{label}: unscrubbed `{marker}` in {s:?}"
                );
            }
            if s.starts_with('/') {
                assert!(
                    s.starts_with("/redacted/"),
                    "{label}: unscrubbed path {s:?}"
                );
            }
            let is_uuid = s.len() == 36 && s.matches('-').count() == 4;
            if is_uuid {
                assert!(
                    s.starts_with("00000000-0000-0000-0000-"),
                    "{label}: real id {s:?}"
                );
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| assert_scrubbed(label, v)),
        serde_json::Value::Object(map) => map.values().for_each(|v| assert_scrubbed(label, v)),
        _ => {}
    }
}

#[test]
fn issue_4779_real_grok_build_prompt_and_stop_payloads_are_present() {
    let events: Vec<_> = fixtures()
        .into_iter()
        .filter(|(provider, ..)| provider == "grok-build")
        .map(|(_, _, event, _)| event)
        .collect();
    for required in ["Stop", "UserPromptSubmit"] {
        assert!(
            events.iter().any(|e| e == required),
            "missing grok-build {required}"
        );
    }
}

#[test]
fn issue_4779_real_hook_payloads_pass_the_parser_entry() {
    let all = fixtures();
    assert!(!all.is_empty());
    for (provider, version, event, raw) in all {
        let label = format!("{provider} {version} {event}");
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_scrubbed(&label, &value);
        let parsed = HookEvent::read_from_str(&raw)
            .unwrap_or_else(|err| panic!("{label}: parser rejected real payload: {err}"))
            .unwrap_or_else(|| panic!("{label}: empty payload"));
        assert_eq!(parsed.cwd.as_deref(), value["cwd"].as_str(), "{label}");
    }
}
