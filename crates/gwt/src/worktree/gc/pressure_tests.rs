use super::*;

fn git(cwd: &Path, args: &[&str]) {
    let output = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Issue #4704 AC-4: exercise the complete pressure-triggered sweep, including
/// the merged cache (#4009), automatic trigger (#4388/#4391), non-repository
/// daemon input (#4566), and stale launch with a reused live PID (#4594).
#[test]
fn disk_pressure_reclaims_idle_merged_and_unmerged_caches_from_project_container() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path().join("home"));
    let config_path = gwt_core::paths::gwt_config_path();
    std::fs::create_dir_all(config_path.parent().expect("config parent")).expect("home");
    std::fs::write(
        &config_path,
        "[build_artifact_gc]\nauto = true\nbelow_bytes = 9223372036854775807\n",
    )
    .expect("config");

    let seed = tmp.path().join("seed");
    std::fs::create_dir_all(&seed).expect("seed dir");
    git(&seed, &["init", "-q", "-b", "develop"]);
    git(&seed, &["config", "user.email", "t@example.com"]);
    git(&seed, &["config", "user.name", "t"]);
    git(&seed, &["commit", "-q", "--allow-empty", "-m", "base"]);

    let project_root = tmp.path().join("project");
    std::fs::create_dir_all(&project_root).expect("project dir");
    let bare = project_root.join("repo.git");
    git(
        &project_root,
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().expect("seed path"),
            bare.to_str().expect("bare path"),
        ],
    );
    git(
        &bare,
        &["remote", "set-url", "origin", "https://example.com/gwt.git"],
    );
    git(&bare, &["config", "user.email", "t@example.com"]);
    git(&bare, &["config", "user.name", "t"]);
    git(
        &bare,
        &["update-ref", "refs/remotes/origin/develop", "HEAD"],
    );
    assert!(!project_root.join(".git").exists());

    let mut targets = Vec::new();
    // Enumeration puts the unmerged names first; priority must reorder them.
    for name in ["z-merged", "a-unmerged", "b-unmerged"] {
        let worktree = project_root.join("work").join(name);
        let branch = format!("work/{name}");
        git(
            &bare,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                &branch,
                worktree.to_str().expect("worktree path"),
            ],
        );
        if name != "z-merged" {
            git(&worktree, &["commit", "-q", "--allow-empty", "-m", name]);
        }
        let target = worktree.join("target");
        std::fs::create_dir_all(&target).expect("target dir");
        std::fs::write(target.join("cache.bin"), [7u8; 4096]).expect("cache");
        targets.push(target);

        // A live non-Host PID used to make an abandoned runtime namespace
        // keep its worktree forever. The test process stays alive without a
        // timeout or child process; its cwd/exe are outside this fixture.
        let sessions = gwt_core::paths::gwt_sessions_dir();
        std::fs::create_dir_all(&sessions).expect("sessions dir");
        let session = gwt_agent::Session::new(worktree, &branch, gwt_agent::AgentId::ClaudeCode);
        session.save(&sessions).expect("stale session");
        let pid = std::process::id();
        let sidecar = gwt_agent::runtime_state_path_for_pid(&sessions, pid, &session.id);
        std::fs::create_dir_all(sidecar.parent().expect("namespace")).expect("namespace dir");
        let mut runtime = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
        runtime.child_pid = Some(pid);
        runtime.save(&sidecar).expect("stale sidecar");
    }

    let config = current_config();
    assert_eq!(
        config.below_bytes,
        i64::MAX as u64,
        "fixture config must load"
    );
    let disk = probe_disk(&project_root, &config);
    let AutoGcDecision::Run { trigger } = decide(&config, &disk, None, Utc::now()) else {
        panic!("configured disk pressure must trigger automatic GC: {disk:?}");
    };
    let history = record_path(&project_root);
    assert!(last_record(&history).is_none());
    run_exclusive(&project_root, &history, &trigger);

    let record = last_record(&history).expect("automatic GC record");
    assert_eq!(
        record.outcome(),
        BuildArtifactGcOutcome::Swept,
        "{record:?}"
    );
    assert_eq!(record.error, None, "{record:?}");
    assert!(record.failed.is_empty(), "{record:?}");
    assert_eq!(record.trigger, trigger);
    assert_eq!(record.candidates, 3, "{record:?}");
    assert_eq!(record.removed.len(), 3, "{record:?}");
    assert_eq!(record.reclaimable_bytes, 3 * 4096, "{record:?}");
    assert_eq!(record.reclaimed_bytes, 3 * 4096, "{record:?}");
    for target in &targets {
        let expected = dunce::canonicalize(target.parent().expect("worktree"))
            .expect("worktree remains")
            .join("target");
        assert!(!target.exists(), "cache was kept: {}", target.display());
        assert!(record.removed.iter().any(|entry| entry.target == expected));
    }

    // AC-3 chooses priority, not an age timeout: after the merged cache
    // relieves pressure, unmerged caches must remain. Reuse the same real
    // repository and make the disk observation depend on actual deletion.
    for target in &targets {
        std::fs::create_dir_all(target).expect("recreate cache");
        std::fs::write(target.join("cache.bin"), [7u8; 4096]).expect("cache");
    }
    let observe_disk = || {
        if targets[0].exists() {
            disk.clone()
        } else {
            crate::disk_space::evaluate(Vec::new())
        }
    };
    let report = worktree_gc::run_gc_with_pressure(
        &project_root,
        "develop",
        AUTO_GC_OPTIONS,
        false,
        Some(&observe_disk),
    )
    .expect("pressure-prioritized sweep");
    assert_eq!(report.removed.len(), 1, "{report:?}");
    assert_eq!(report.reclaimed_bytes, 4096, "{report:?}");
    assert!(!targets[0].exists());
    assert!(targets[1].exists());
    assert!(targets[2].exists());
}
