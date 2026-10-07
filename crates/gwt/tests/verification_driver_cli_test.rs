//! Issue #4970: the real Windows verifier and its watchdog must not lock Cargo's artifact.
#![cfg(windows)]

use std::{fs, io::Write, process::Stdio};

use gwt_core::process::hidden_command;
use sha2::{Digest, Sha256};

#[test]
fn canonical_driver_frees_its_artifact_and_records_build_provenance() {
    assert_driver_relink("target");
}

#[test]
fn configured_output_driver_frees_its_artifact_and_records_build_provenance() {
    // The configured target must not also contain the fixed driver's cache.
    assert_driver_relink(".gwt");
}

fn assert_driver_relink(target_dir: &str) {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='driver-fixture'\nversion='0.1.0'\n[lib]\npath='lib.rs'\n",
    )
    .unwrap();
    fs::write(root.join("lib.rs"), "").unwrap();
    fs::create_dir(root.join(".cargo")).unwrap();
    fs::write(
        root.join(".cargo/config.toml"),
        format!("[build]\ntarget-dir='{target_dir}'\n"),
    )
    .unwrap();
    assert!(hidden_command("git")
        .args(["init", "-q"])
        .current_dir(root)
        .status()
        .unwrap()
        .success());
    let artifact = root.join(target_dir).join("debug/gwtd.exe");
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_gwtd"), &artifact).unwrap();
    let expected_hash = format!("{:x}", Sha256::digest(fs::read(&artifact).unwrap()));
    // The failure code also proves that relocation preserves the caller's OS
    // exit code, in addition to freeing the runner and watchdog's original path.
    let command = format!(
        r#"powershell.exe -NoProfile -NonInteractive -Command "Remove-Item -LiteralPath '{target_dir}/debug/gwtd.exe' -ErrorAction Stop; [System.IO.File]::WriteAllBytes('{target_dir}/debug/gwtd.exe', [byte[]](1,2,3)); exit 7""#
    );
    let mut driver = hidden_command(&artifact);
    for key in [
        "GWT_BIN_PATH",
        "GWT_HOOK_BIN",
        "GWT_HOOK_FORWARD_TOKEN",
        "GWT_HOOK_FORWARD_URL",
        "GWT_PROJECT_ROOT",
        "GWT_REPO_HASH",
        "GWT_SESSION_RUNTIME_PATH",
        "GWT_WORKTREE_HASH",
        "CARGO_TARGET_DIR",
    ] {
        driver.env_remove(key);
    }
    let mut child = driver
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GWT_SESSION_ID", "driver-regression")
        .env("GWT_VERIFY_SPAWN_HOST", "inherit")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({
                "schema_version": 1, "operation": "verify.run",
                "params": {"commands": [command], "max_wait_secs": 0}
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        fs::metadata(&artifact).unwrap().len(),
        3,
        "the artifact remained locked"
    );
    assert_eq!(fs::read(&artifact).unwrap(), [1, 2, 3]);
    let record: gwt::cli::verification_record::VerificationRunRecord =
        serde_json::from_slice(&fs::read(root.join(".gwt/tmp/verify-run.json")).unwrap()).unwrap();
    assert!(gwt::cli::verification_record::integrity_ok(&record));
    assert_eq!(record.commands[0].exit_code, 7);
    let value = serde_json::to_value(&record).unwrap();
    let provenance = &value["driver"];
    assert_eq!(provenance["sha256"], expected_hash);
    assert_eq!(provenance["source_head"], env!("GWT_BUILD_COMMIT"));
    let fixed = std::path::PathBuf::from(provenance["fixed_path"].as_str().unwrap());
    assert!(!fixed.starts_with(root.join(target_dir)));
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(&fixed).unwrap())),
        expected_hash
    );
}
