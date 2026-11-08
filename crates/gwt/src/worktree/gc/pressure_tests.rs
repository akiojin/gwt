use super::*;

/// Invoked in a child process so the sweep can observe a launch after its
/// initial snapshot, without a timer or a platform-specific shell.
#[test]
fn gc_launch_child_fixture() {
    if std::env::var_os("GWT_TEST_GC_CHILD").is_none() {
        return;
    }
    println!("ready");
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .expect("release child");
}

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
    for (name, bytes) in [
        ("z-merged", 4096),
        ("a-unmerged", 8192),
        ("b-unmerged", 16384),
    ] {
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
        std::fs::write(target.join("cache.bin"), vec![7u8; bytes]).expect("cache");
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
    assert_eq!(record.reclaimable_bytes, 28672, "{record:?}");
    assert_eq!(record.reclaimed_bytes, 28672, "{record:?}");
    assert_eq!(
        record
            .removed
            .iter()
            .map(|entry| entry.bytes)
            .collect::<Vec<_>>(),
        vec![4096, 16384, 8192],
        "merged first, then largest idle cache first"
    );
    assert!(!record.pressure_volumes.is_empty());
    assert_eq!(
        record.reclaimed_volumes.values().sum::<u64>(),
        record.reclaimed_bytes
    );
    assert!(record
        .reclaimed_volumes
        .keys()
        .all(|volume| record.pressure_volumes.contains(volume)));
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

    std::fs::create_dir_all(&targets[0]).expect("restore merged cache");
    std::fs::write(targets[0].join("cache.bin"), [7u8; 4096]).expect("cache");

    // Issue #5624 AC-1/3: a healthy cache volume must not be reclaimed to
    // relieve pressure on another volume. Inject the disk observation so
    // this regression also runs on CI hosts with only one physical disk.
    let mut foreign_disk = disk.clone();
    for volume in &mut foreign_disk.volumes {
        volume.volume_id = Some("other-volume".to_string());
    }
    let foreign = || foreign_disk.clone();
    let report = worktree_gc::run_gc_with_pressure(
        &project_root,
        "develop",
        AUTO_GC_OPTIONS,
        false,
        Some(&foreign),
    )
    .expect("foreign-volume pressure sweep");
    assert!(report.removed.is_empty(), "{report:?}");
    assert!(targets
        .iter()
        .all(|target| target.join("cache.bin").is_file()));

    // AC-2/3: launch a process only after enumeration. Its working directory
    // must protect the cache at the removal boundary.
    use std::io::BufRead as _;
    let child = std::cell::RefCell::new(None);
    let launching_worktree = targets[0].parent().expect("worktree");
    let launch = || {
        if child.borrow().is_none() {
            let mut process =
                gwt_core::process::hidden_command(std::env::current_exe().expect("exe"))
                    .args([
                        "--exact",
                        "worktree::gc::pressure_tests::gc_launch_child_fixture",
                        "--nocapture",
                    ])
                    .env("GWT_TEST_GC_CHILD", "1")
                    .current_dir(launching_worktree)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .expect("launch child");
            let mut output = std::io::BufReader::new(process.stdout.take().expect("stdout"));
            let mut line = String::new();
            while !line.contains("ready") {
                assert!(
                    output.read_line(&mut line).expect("child ready") > 0,
                    "{line}"
                );
            }
            // Keep the read end open until libtest prints its final result.
            process.stdout = Some(output.into_inner());
            child.replace(Some(process));
        }
        disk.clone()
    };
    let result = worktree_gc::run_gc_with_pressure(
        &project_root,
        "develop",
        AUTO_GC_OPTIONS,
        false,
        Some(&launch),
    );
    let mut process = child.into_inner().expect("launched child");
    drop(process.stdin.take());
    assert!(process.wait().expect("child exits").success());
    let report = result.expect("launch during sweep");
    assert!(targets[0].join("cache.bin").is_file(), "{report:?}");
    assert!(
        report
            .kept
            .iter()
            .any(|kept| kept.reason.starts_with("active process")),
        "{report:?}"
    );
}
