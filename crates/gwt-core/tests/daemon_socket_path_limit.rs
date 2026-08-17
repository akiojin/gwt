#![cfg(unix)]
//! Unix socket path-limit contract for the runtime daemon (Issue #3476).
//!
//! `bind(2)` copies the socket path into `sockaddr_un.sun_path`, a fixed
//! byte array (104 on macOS/BSD, 108 on Linux). A daemon endpoint living
//! under a long `HOME` — a fresh-home PM session, a deep worktree — pushes
//! the colocated `<worktree_hash>.sock` past that boundary and `bind`
//! fails with `path must be shorter than SUN_LEN`. These tests pin the
//! boundary itself and the shortening contract that keeps the daemon
//! reachable on such paths.

use std::{
    fs,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
};

use gwt_core::daemon::{
    resolve_daemon_socket_path, resolve_daemon_socket_path_in, short_socket_base_candidates,
    short_socket_base_candidates_for, DaemonSocketPlacement, DAEMON_SOCKET_DIR_ENV,
    MAX_UNIX_SOCKET_PATH_LEN, UNIX_SOCKET_PATH_CAPACITY,
};
use tempfile::tempdir;

/// Builds a path directly under `dir` whose full byte length is `len`.
fn path_of_len(dir: &Path, len: usize) -> PathBuf {
    let prefix = dir.join("x");
    let prefix_len = prefix.as_os_str().len();
    assert!(
        prefix_len <= len,
        "temp dir {} is already longer than the requested {len} bytes",
        dir.display()
    );
    let path = dir.join("x".repeat(len - prefix_len + 1));
    assert_eq!(path.as_os_str().len(), len);
    path
}

/// Builds an endpoint path under `root` whose colocated `.sock` sibling is
/// guaranteed to overflow `sun_path`.
fn overlong_endpoint_path(root: &Path) -> PathBuf {
    let mut dir = root.to_path_buf();
    while dir.join("0123456789abcdef.sock").as_os_str().len() <= MAX_UNIX_SOCKET_PATH_LEN {
        dir = dir.join("runtime-daemon-padding");
    }
    fs::create_dir_all(&dir).expect("create overlong endpoint dir");
    dir.join("0123456789abcdef.json")
}

#[test]
fn unix_socket_path_capacity_matches_the_platform_sockaddr_un() {
    // AC-5: the limit is a platform fact, not a guess. Assert the
    // published constant and then prove it empirically against the
    // kernel, so a libc layout change cannot silently shift it.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    assert_eq!(
        UNIX_SOCKET_PATH_CAPACITY, 104,
        "macOS sockaddr_un.sun_path is 104 bytes"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        UNIX_SOCKET_PATH_CAPACITY, 108,
        "Linux sockaddr_un.sun_path is 108 bytes"
    );
    assert_eq!(
        MAX_UNIX_SOCKET_PATH_LEN,
        UNIX_SOCKET_PATH_CAPACITY - 1,
        "one byte of sun_path is reserved for the NUL terminator"
    );

    let dir = tempdir().expect("tempdir");
    let longest_ok = path_of_len(dir.path(), MAX_UNIX_SOCKET_PATH_LEN);
    let listener = UnixListener::bind(&longest_ok);
    assert!(
        listener.is_ok(),
        "a {MAX_UNIX_SOCKET_PATH_LEN}-byte socket path must bind, got: {:?}",
        listener.err()
    );
    drop(listener);
    let _ = fs::remove_file(&longest_ok);

    let one_too_long = path_of_len(dir.path(), MAX_UNIX_SOCKET_PATH_LEN + 1);
    assert!(
        UnixListener::bind(&one_too_long).is_err(),
        "a {}-byte socket path must be rejected by bind(2)",
        MAX_UNIX_SOCKET_PATH_LEN + 1
    );
}

#[test]
fn daemon_socket_path_stays_colocated_when_it_fits() {
    // No regression for the ordinary `~/.gwt` layout: the socket keeps
    // living next to the endpoint metadata it belongs to (#2338).
    let home = tempdir().expect("tempdir");
    let endpoint_path = home.path().join("daemon").join("abcdef0123456789.json");
    fs::create_dir_all(endpoint_path.parent().unwrap()).expect("create daemon dir");

    let resolved = resolve_daemon_socket_path(&endpoint_path).expect("resolve socket path");

    assert_eq!(resolved.placement, DaemonSocketPlacement::Colocated);
    assert_eq!(resolved.path, endpoint_path.with_extension("sock"));
    assert!(resolved.path.as_os_str().len() <= MAX_UNIX_SOCKET_PATH_LEN);
}

#[test]
fn daemon_socket_path_is_shortened_and_bindable_when_the_endpoint_path_is_too_long() {
    // AC-1: the reported failure. A long fresh-home runtime root must
    // still yield a socket the kernel accepts.
    let home = tempdir().expect("tempdir");
    let endpoint_path = overlong_endpoint_path(home.path());
    assert!(
        endpoint_path.with_extension("sock").as_os_str().len() > MAX_UNIX_SOCKET_PATH_LEN,
        "fixture must overflow sun_path to exercise the shortening path"
    );

    let base = tempdir().expect("base tempdir");
    let resolved = resolve_daemon_socket_path_in(&endpoint_path, &[base.path().to_path_buf()])
        .expect("resolve shortened socket path");

    assert_eq!(resolved.placement, DaemonSocketPlacement::Shortened);
    assert!(
        resolved.path.as_os_str().len() <= MAX_UNIX_SOCKET_PATH_LEN,
        "shortened socket path is still {} bytes: {}",
        resolved.path.as_os_str().len(),
        resolved.path.display()
    );
    assert!(resolved.path.starts_with(base.path()));

    let listener = UnixListener::bind(&resolved.path);
    assert!(
        listener.is_ok(),
        "shortened socket path must bind, got: {:?}",
        listener.err()
    );
}

#[test]
fn shortened_daemon_socket_path_is_deterministic_for_the_same_endpoint() {
    // AC-2: a restarting daemon must land on the same socket so stale
    // socket cleanup and endpoint reuse keep working.
    let home = tempdir().expect("tempdir");
    let endpoint_path = overlong_endpoint_path(home.path());
    let base = tempdir().expect("base tempdir");
    let bases = [base.path().to_path_buf()];

    let first = resolve_daemon_socket_path_in(&endpoint_path, &bases).expect("first resolve");
    let second = resolve_daemon_socket_path_in(&endpoint_path, &bases).expect("second resolve");

    assert_eq!(first.path, second.path);
}

#[test]
fn shortened_daemon_socket_paths_do_not_collide_across_scopes() {
    // AC-2: distinct repo hash, worktree hash, or gwt home must never
    // share one socket, or two daemons would fight over the same bind.
    let base = tempdir().expect("base tempdir");
    let bases = [base.path().to_path_buf()];

    let home_a = tempdir().expect("home a");
    let home_b = tempdir().expect("home b");
    let endpoint_a = overlong_endpoint_path(home_a.path());
    let endpoint_b = overlong_endpoint_path(home_b.path());
    let sibling_worktree = endpoint_a.with_file_name("fedcba9876543210.json");
    let sibling_repo = endpoint_a
        .parent()
        .unwrap()
        .with_file_name("other-repo-hash")
        .join(endpoint_a.file_name().unwrap());

    let mut paths = vec![
        resolve_daemon_socket_path_in(&endpoint_a, &bases)
            .expect("resolve a")
            .path,
        resolve_daemon_socket_path_in(&endpoint_b, &bases)
            .expect("resolve b")
            .path,
        resolve_daemon_socket_path_in(&sibling_worktree, &bases)
            .expect("resolve sibling worktree")
            .path,
        resolve_daemon_socket_path_in(&sibling_repo, &bases)
            .expect("resolve sibling repo")
            .path,
    ];
    let total = paths.len();
    paths.sort();
    paths.dedup();
    assert_eq!(
        paths.len(),
        total,
        "every distinct endpoint must map to its own socket path"
    );
}

#[test]
fn shortened_daemon_socket_directory_is_private_to_its_owner() {
    // AC-4: moving the socket out of the project runtime root must not
    // widen the local-only, owner-scoped IPC boundary.
    let home = tempdir().expect("tempdir");
    let endpoint_path = overlong_endpoint_path(home.path());
    let base = tempdir().expect("base tempdir");

    let resolved = resolve_daemon_socket_path_in(&endpoint_path, &[base.path().to_path_buf()])
        .expect("resolve shortened socket path");

    // A file this process just created is owned by our effective uid, so
    // it is a dependency-free reference for "owned by us".
    let probe = base.path().join("uid-probe");
    fs::write(&probe, b"").expect("write uid probe");
    let own_uid = fs::metadata(&probe).expect("stat uid probe").uid();

    let dir = resolved.path.parent().expect("socket parent");
    let metadata = fs::symlink_metadata(dir).expect("stat socket dir");
    assert!(metadata.is_dir(), "socket parent must be a real directory");
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o700,
        "socket directory must not be readable or writable by other users"
    );
    assert_eq!(
        metadata.uid(),
        own_uid,
        "socket directory must be owned by the daemon's own user"
    );
}

#[test]
fn shortened_daemon_socket_path_skips_bases_that_are_themselves_too_long() {
    let home = tempdir().expect("tempdir");
    let endpoint_path = overlong_endpoint_path(home.path());

    let long_base_root = tempdir().expect("long base tempdir");
    let mut long_base = long_base_root.path().to_path_buf();
    while long_base.as_os_str().len() <= MAX_UNIX_SOCKET_PATH_LEN {
        long_base = long_base.join("socket-base-padding");
    }
    fs::create_dir_all(&long_base).expect("create long base");
    let usable_base = tempdir().expect("usable base tempdir");

    let resolved = resolve_daemon_socket_path_in(
        &endpoint_path,
        &[long_base.clone(), usable_base.path().to_path_buf()],
    )
    .expect("resolve shortened socket path");

    assert!(resolved.path.starts_with(usable_base.path()));
    assert!(!resolved.path.starts_with(&long_base));
}

#[test]
fn daemon_socket_path_reports_a_diagnosable_error_when_no_short_base_is_usable() {
    // AC-6: never surface a bare `path must be shorter than SUN_LEN`.
    // The operator needs the cause and a way out.
    let home = tempdir().expect("tempdir");
    let endpoint_path = overlong_endpoint_path(home.path());

    let error = resolve_daemon_socket_path_in(&endpoint_path, &[])
        .expect_err("resolution must fail when no base is usable");
    let message = error.to_string();

    assert!(
        message.contains(&endpoint_path.with_extension("sock").display().to_string()),
        "error must name the socket path that did not fit: {message}"
    );
    assert!(
        message.contains(&MAX_UNIX_SOCKET_PATH_LEN.to_string()),
        "error must state the platform limit: {message}"
    );
    assert!(
        message.contains(DAEMON_SOCKET_DIR_ENV),
        "error must name the environment lever that fixes it: {message}"
    );
}

#[test]
fn default_short_socket_bases_are_non_empty_and_prefer_runtime_dirs() {
    let candidates = short_socket_base_candidates();
    assert!(
        !candidates.is_empty(),
        "there must always be a last-resort short socket base"
    );
    assert!(
        candidates.iter().any(|base| base == Path::new("/tmp")),
        "/tmp must remain the last-resort base: {candidates:?}"
    );
}

#[test]
fn short_socket_bases_order_runtime_dirs_ahead_of_the_last_resort() {
    let candidates = short_socket_base_candidates_for(
        None,
        Some(PathBuf::from("/run/user/501")),
        Some(PathBuf::from("/var/folders/xy/T")),
    );
    assert_eq!(
        candidates,
        vec![
            PathBuf::from("/run/user/501"),
            PathBuf::from("/var/folders/xy/T"),
            PathBuf::from("/tmp"),
        ]
    );
}

#[test]
fn short_socket_bases_deduplicate_and_survive_missing_runtime_dirs() {
    let candidates =
        short_socket_base_candidates_for(None, None, Some(PathBuf::from(SHORT_SOCKET_LAST_RESORT)));
    assert_eq!(candidates, vec![PathBuf::from(SHORT_SOCKET_LAST_RESORT)]);
}

#[test]
fn an_explicit_socket_dir_override_replaces_the_default_candidates() {
    // An operator who pins the directory must not be silently rerouted to
    // /tmp when their choice is wrong; the failure has to be visible.
    let candidates = short_socket_base_candidates_for(
        Some(PathBuf::from("/pinned/socket/dir")),
        Some(PathBuf::from("/run/user/501")),
        Some(PathBuf::from("/var/folders/xy/T")),
    );
    assert_eq!(candidates, vec![PathBuf::from("/pinned/socket/dir")]);
}

const SHORT_SOCKET_LAST_RESORT: &str = "/tmp";
