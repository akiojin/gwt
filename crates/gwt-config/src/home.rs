//! Shared user-home resolution and fail-closed test isolation (Issue #5339).
//!
//! This lives below gwt-core so settings and runtime paths cannot disagree.

use std::{
    cell::RefCell,
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

thread_local! {
    static HOME_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub fn gwt_home_override() -> Option<PathBuf> {
    HOME_OVERRIDE.with(|home| home.borrow().clone())
}

/// Resolve the user-home root, isolating Cargo test executables by default.
pub fn user_home() -> Option<PathBuf> {
    if let Some(home) = gwt_home_override() {
        return Some(home);
    }
    let home = resolve_config_home_dir(
        std::env::var_os("HOME"),
        std::env::var_os("USERPROFILE"),
        dirs::home_dir(),
    );
    if is_test_process() {
        if let Some(home) = &home {
            if is_isolated(home) {
                return Some(home.clone());
            }
        }
        static TEST_HOME: OnceLock<PathBuf> = OnceLock::new();
        return Some(
            TEST_HOME
                .get_or_init(|| {
                    std::env::temp_dir()
                        .join("gwt-cargo-test-home")
                        .join(std::process::id().to_string())
                })
                .clone(),
        );
    }
    home
}

pub(crate) fn resolve_config_home_dir(
    home: Option<OsString>,
    userprofile: Option<OsString>,
    fallback: Option<PathBuf>,
) -> Option<PathBuf> {
    home.filter(|value| !value.is_empty())
        .or_else(|| userprofile.filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .or(fallback)
}

fn is_test_process() -> bool {
    static IS_TEST: OnceLock<bool> = OnceLock::new();
    *IS_TEST.get_or_init(|| {
        cfg!(test)
            || std::env::current_exe()
                .ok()
                .is_some_and(|exe| is_cargo_test_binary(&exe))
    })
}

fn is_cargo_test_binary(exe: &Path) -> bool {
    exe.parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "deps")
        && exe
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.contains('-'))
}

/// Panic before test config IO can access a path outside temporary storage.
///
/// Recovery is to use a TempDir with ScopedGwtHome, or pass an explicit
/// temporary path. No environment opt-out is accepted. Symlinks and `..`
/// are resolved before checking; this function never creates or changes files.
#[track_caller]
pub fn assert_test_path(path: &Path) {
    if is_test_process() || gwt_home_override().is_some() {
        assert!(
            is_isolated(path),
            "test HOME isolation: refusing access to {}; use TempDir and ScopedGwtHome",
            path.display()
        );
    }
}

fn is_isolated(path: &Path) -> bool {
    std::env::temp_dir()
        .canonicalize()
        .is_ok_and(|temporary| is_isolated_under(path, &temporary))
}

fn is_isolated_under(path: &Path, temporary: &Path) -> bool {
    let Some(resolved) = resolved_path(path) else {
        return false;
    };
    // Atomic publication writes a sibling and replaces the leaf itself.
    // An outside symlink pointing INTO temp must not authorize that write.
    resolved.starts_with(temporary)
        && path
            .parent()
            .and_then(resolved_path)
            .is_some_and(|parent| parent.starts_with(temporary))
}

/// Resolve existing ancestors too, since the final file may not exist yet.
fn resolved_path(path: &Path) -> Option<PathBuf> {
    // Windows absolute() normalizes `..` before we could inspect it below.
    if path.components().any(|part| part == Component::ParentDir) {
        return None;
    }
    let absolute = std::path::absolute(path).ok()?;
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            // create_dir_all(original_path) can create an escaped ancestor
            // before traversing back into temp. Reject rather than normalize.
            Component::ParentDir => return None,
            component => {
                resolved.push(component.as_os_str());
                match std::fs::symlink_metadata(&resolved) {
                    Ok(_) => resolved = resolved.canonicalize().ok()?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return None,
                }
            }
        }
    }
    Some(resolved)
}

/// Thread-local user-home override shared by every gwt crate.
#[cfg(any(test, feature = "test-support"))]
pub struct ScopedGwtHome {
    previous: Option<PathBuf>,
    _not_send: std::marker::PhantomData<*const ()>,
}

#[cfg(any(test, feature = "test-support"))]
impl ScopedGwtHome {
    pub fn set(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        assert!(
            is_isolated(path),
            "test HOME isolation: ScopedGwtHome requires a temporary home"
        );
        Self {
            previous: HOME_OVERRIDE.with(|home| home.replace(Some(path.to_path_buf()))),
            _not_send: std::marker::PhantomData,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for ScopedGwtHome {
    fn drop(&mut self) {
        HOME_OVERRIDE.with(|home| home.replace(self.previous.take()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_cargo_tests_but_not_the_application() {
        assert!(is_cargo_test_binary(Path::new(
            "target/debug/deps/gwt-abc123"
        )));
        assert!(!is_cargo_test_binary(Path::new("target/debug/gwtd")));
    }

    #[test]
    fn scoped_home_is_nested_and_thread_local() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let initial = user_home();
        let outer = ScopedGwtHome::set(first.path());
        assert_eq!(user_home(), Some(first.path().to_path_buf()));
        {
            let _inner = ScopedGwtHome::set(second.path());
            assert_eq!(user_home(), Some(second.path().to_path_buf()));
        }
        assert_eq!(user_home(), Some(first.path().to_path_buf()));
        assert_ne!(std::thread::spawn(user_home).join().unwrap(), user_home());
        drop(outer);
        assert_eq!(user_home(), initial);
    }

    #[test]
    fn guard_rejects_parent_traversal() {
        let path =
            std::env::temp_dir().join("../../../../../../__gwt_protected_home__/.gwt/config.toml");
        assert!(std::panic::catch_unwind(|| assert_test_path(&path)).is_err());
    }

    #[test]
    fn guard_rejects_missing_ancestors_before_parent_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("must-not-be-created");
        let path = missing.join("../config.toml");
        assert!(std::panic::catch_unwind(|| {
            let _ = crate::atomic::write_atomic(&path, "forbidden");
        })
        .is_err());
        assert!(!missing.exists());
        assert!(!temp.path().join("config.toml").exists());
    }

    #[cfg(unix)]
    #[test]
    fn guard_rejects_symlinks_out_of_temporary_storage() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("escape");
        std::os::unix::fs::symlink("/", &link).unwrap();
        assert!(
            std::panic::catch_unwind(|| assert_test_path(&link.join(".gwt/config.toml"))).is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn guard_rejects_outside_leaf_symlinks_into_temporary_storage() {
        let arena = tempfile::tempdir().unwrap();
        let trusted = arena.path().join("trusted");
        std::fs::create_dir(&trusted).unwrap();
        let target = trusted.join("config.toml");
        std::fs::write(&target, "unchanged").unwrap();
        let outside_link = arena.path().join("outside-config.toml");
        std::os::unix::fs::symlink(&target, &outside_link).unwrap();
        assert!(!is_isolated_under(
            &outside_link,
            &trusted.canonicalize().unwrap()
        ));
        assert!(outside_link
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
    }
}
