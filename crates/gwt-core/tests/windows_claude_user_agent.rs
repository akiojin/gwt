//! Issue #4750: PATH-changing live probes own a separate test process.
//! Both tests share the environment lock; unrelated library tests never see
//! their PATH, PATHEXT, or USERPROFILE overrides.
#![cfg(windows)]

use gwt_core::usage::claude::claude_user_agent;

#[test]
fn live_user_agent_probe_resolves_real_bun_global_placeholder_fixture() {
    let _env = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
        .expect("create real Windows Bun fixture");
    let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
    let _path_ext = gwt_core::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
    let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);

    assert_eq!(claude_user_agent(), Ok("claude-code/2.1.210".to_string()));
}

#[test]
fn live_user_agent_probe_rejects_real_bun_global_placeholder_fixture_without_safe_target() {
    let _env = gwt_core::test_support::env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
        .expect("create real Windows Bun fixture");
    fixture
        .remove_safe_targets()
        .expect("remove safe redirect targets");
    let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
    let _path_ext = gwt_core::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
    let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);

    let error = claude_user_agent().expect_err("unsafe placeholder must be rejected");
    assert!(error.contains("native-binary placeholder"));
}
