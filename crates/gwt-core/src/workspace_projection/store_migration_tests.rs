use super::*;

#[test]
fn transaction_temp_cleanup_retains_live_uncertain_and_nonempty_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let _home = crate::test_support::ScopedGwtHome::set(temp.path().join("home"));
    assert_eq!(
        cleanup_abandoned_workspace_transaction_temps(|_| Some(false)).unwrap(),
        0
    );
    let directory = crate::paths::gwt_home().join(WORKSPACE_STATE_TRANSACTION_COORDINATOR_DIR);
    fs::create_dir_all(&directory).unwrap();
    let residue = |pid| {
        directory.join(format!(
            ".{}.json.tmp-{pid}-{}",
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        ))
    };
    let dead = residue(11);
    let live = residue(22);
    let uncertain = residue(33);
    let nonempty = residue(11);
    let directory_residue = residue(11);
    let unrelated = directory.join("unrelated.tmp");
    for path in [&dead, &live, &uncertain, &unrelated] {
        fs::write(path, []).unwrap();
    }
    fs::write(&nonempty, b"preserved transaction data").unwrap();
    fs::create_dir(&directory_residue).unwrap();
    #[cfg(unix)]
    let symlink = {
        let path = residue(11);
        std::os::unix::fs::symlink(&live, &path).unwrap();
        path
    };

    let removed = cleanup_abandoned_workspace_transaction_temps(|pid| match pid {
        11 => Some(false),
        22 => Some(true),
        _ => None,
    })
    .unwrap();

    assert_eq!(removed, 1);
    assert!(!dead.exists());
    for retained in [&live, &uncertain, &nonempty, &directory_residue, &unrelated] {
        assert!(retained.exists(), "{} must remain", retained.display());
    }
    #[cfg(unix)]
    assert!(fs::symlink_metadata(symlink)
        .unwrap()
        .file_type()
        .is_symlink());
}
