use super::*;

#[test]
fn split_diagnostic_reports_a_store_recreated_after_quarantine() {
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::ScopedGwtHome::set(temp.path().join("home"));
    let repo = temp.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(
        repo.join(".git/config"),
        "[remote \"origin\"]\nurl = https://github.com/example/recreated-store.git\n",
    )
    .unwrap();
    let identity = detect_repo_identity(&repo).unwrap();
    let legacy_hash = compute_path_hash(&repo);
    let manifest = quarantine_manifest_path(&gwt_project_dir(&identity.hash), legacy_hash.as_str());
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(manifest, "{}").unwrap();
    assert!(split_project_store_paths(&repo).is_empty());

    let legacy_store = gwt_project_dir(&legacy_hash);
    fs::create_dir_all(&legacy_store).unwrap();
    assert_eq!(split_project_store_paths(&repo), vec![legacy_store]);
}

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

#[test]
fn transaction_temp_cleanup_retains_data_written_before_writer_exit() {
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::ScopedGwtHome::set(temp.path().join("home"));
    let directory = crate::paths::gwt_home().join(WORKSPACE_STATE_TRANSACTION_COORDINATOR_DIR);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!(
        ".{}.json.tmp-11-{}",
        uuid::Uuid::new_v4(),
        uuid::Uuid::new_v4(),
    ));
    fs::write(&path, []).unwrap();

    let removed = cleanup_abandoned_workspace_transaction_temps(|_| {
        fs::write(&path, b"transaction data written before exit").unwrap();
        Some(false)
    })
    .unwrap();

    assert_eq!(removed, 0);
    assert_eq!(
        fs::read(&path).unwrap(),
        b"transaction data written before exit"
    );
}
