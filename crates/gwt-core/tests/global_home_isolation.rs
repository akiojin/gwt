//! Cross-crate regression for Issue #5339: one scoped home owns every global path.

use gwt_config::Settings;
use gwt_core::{paths::gwt_home, test_support::ScopedGwtHome};

#[test]
fn scoped_home_also_isolates_global_settings() {
    let home = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(home.path());

    assert_eq!(gwt_home(), home.path().join(".gwt"));
    assert_eq!(
        Settings::global_config_path(),
        Some(home.path().join(".gwt/config.toml"))
    );
    Settings::default().save_global().unwrap();
    Settings::update_global(|settings| {
        settings.debug = true;
        Ok(())
    })
    .unwrap();
    assert!(Settings::load().unwrap().debug);
    assert!(home.path().join(".gwt/config.toml").is_file());
}

#[test]
fn config_and_core_writers_panic_before_unisolated_io() {
    let path = std::path::Path::new("/__gwt_protected_test_home__/.gwt/config.toml");
    assert!(!path.parent().unwrap().exists());
    assert!(std::panic::catch_unwind(|| {
        let _ = gwt_config::atomic::write_atomic(path, "must not be written");
    })
    .is_err());
    assert!(std::panic::catch_unwind(|| {
        let _ = gwt_core::atomic_file::write_atomic(path, b"must not be written");
    })
    .is_err());
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn pending_update_guard_runs_before_creating_the_directory_or_lock() {
    let home = tempfile::tempdir().unwrap();
    let missing = home.path().join("must-not-be-created");
    let pending = missing.join("../pending-update");
    assert!(std::panic::catch_unwind(|| {
        let _ = gwt_core::update::pending_update_manifest_lock(&pending);
    })
    .is_err());
    assert!(!missing.exists());
    assert!(!home.path().join("pending-update").exists());
}
