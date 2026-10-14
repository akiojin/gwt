//! Route- and path-tagged filesystem errors for managed asset work.
//!
//! #4486 AC-6: a managed asset regeneration failure has to say *which path*
//! failed and *on which route* — reading an existing asset, backing one up,
//! writing the new one, or deleting a stale one. A bare [`std::io::Error`]
//! carries neither: `fs::read_to_string` reports "No such file or directory"
//! with no path at all, so the reason that reached
//! `worktree_freshness.failure_reason` named only the operation that happened
//! to wrap it ("failed to distribute gwt managed assets: …"). That is enough
//! to know something broke and not enough to know what to fix.
//!
//! These helpers are deliberately thin: they do exactly what the `std::fs`
//! call does and attach the route and the path when it fails.

use std::fmt;
use std::path::{Path, PathBuf};

/// Which stage of a managed asset transaction touched the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssetRoute {
    /// Reading an existing asset, or probing for one.
    Read,
    /// Copying an asset aside so it can be restored on failure.
    Backup,
    /// Writing a generated asset, including creating its parent directory.
    Write,
    /// Removing a stale or superseded asset.
    Delete,
}

impl AssetRoute {
    /// Stable lowercase name, used in error text and in assertions.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Backup => "backup",
            Self::Write => "write",
            Self::Delete => "delete",
        }
    }
}

impl fmt::Display for AssetRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A filesystem failure with the route and path that produced it.
#[derive(Debug, thiserror::Error)]
#[error("managed asset {route} failed at {}: {source}", path.display())]
pub struct AssetIoError {
    /// Which route was being walked when the call failed.
    pub route: AssetRoute,
    /// The path the failing call was given.
    pub path: PathBuf,
    /// The underlying failure.
    #[source]
    pub source: std::io::Error,
}

impl AssetIoError {
    /// Tag an error that has already been produced.
    pub fn new(route: AssetRoute, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self {
            route,
            path: path.into(),
            source,
        }
    }

    /// The kind of the underlying failure, so callers can keep branching on it
    /// (for example, treating an absent optional asset as "nothing to do").
    pub fn kind(&self) -> std::io::ErrorKind {
        self.source.kind()
    }
}

impl From<AssetIoError> for std::io::Error {
    /// Preserve the original [`std::io::ErrorKind`] so callers that branch on
    /// `NotFound` keep working once a site is converted, and carry the route
    /// and path in the message so they survive into `failure_reason`.
    fn from(error: AssetIoError) -> Self {
        std::io::Error::new(error.source.kind(), error.to_string())
    }
}

/// Read a file, tagging failure as [`AssetRoute::Read`].
pub fn read_to_string(path: impl AsRef<Path>) -> Result<String, AssetIoError> {
    let path = path.as_ref();
    std::fs::read_to_string(path)
        .map_err(|source| AssetIoError::new(AssetRoute::Read, path, source))
}

/// Read a file's bytes, tagging failure as [`AssetRoute::Read`].
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, AssetIoError> {
    let path = path.as_ref();
    std::fs::read(path).map_err(|source| AssetIoError::new(AssetRoute::Read, path, source))
}

/// Create a directory tree, tagging failure as [`AssetRoute::Write`].
pub fn create_dir_all(path: impl AsRef<Path>) -> Result<(), AssetIoError> {
    let path = path.as_ref();
    std::fs::create_dir_all(path)
        .map_err(|source| AssetIoError::new(AssetRoute::Write, path, source))
}

/// Write a file, tagging failure as [`AssetRoute::Write`]. The path reported is
/// the destination, which is the one a reader needs in order to act.
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<(), AssetIoError> {
    let path = path.as_ref();
    std::fs::write(path, contents)
        .map_err(|source| AssetIoError::new(AssetRoute::Write, path, source))
}

/// Copy a file aside, tagging failure as [`AssetRoute::Backup`]. The path
/// reported is the destination, because that is where a backup fails.
pub fn backup(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64, AssetIoError> {
    let to = to.as_ref();
    std::fs::copy(from.as_ref(), to)
        .map_err(|source| AssetIoError::new(AssetRoute::Backup, to, source))
}

/// Move a file into place, tagging failure as [`AssetRoute::Write`].
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<(), AssetIoError> {
    let to = to.as_ref();
    std::fs::rename(from.as_ref(), to)
        .map_err(|source| AssetIoError::new(AssetRoute::Write, to, source))
}

/// Read a directory listing, tagging failure as [`AssetRoute::Read`].
pub fn read_dir(path: impl AsRef<Path>) -> Result<std::fs::ReadDir, AssetIoError> {
    let path = path.as_ref();
    std::fs::read_dir(path).map_err(|source| AssetIoError::new(AssetRoute::Read, path, source))
}

/// Read a path's metadata without following symlinks, tagging failure as
/// [`AssetRoute::Read`].
pub fn symlink_metadata(path: impl AsRef<Path>) -> Result<std::fs::Metadata, AssetIoError> {
    let path = path.as_ref();
    std::fs::symlink_metadata(path)
        .map_err(|source| AssetIoError::new(AssetRoute::Read, path, source))
}

/// Remove an empty directory, tagging failure as [`AssetRoute::Delete`].
pub fn remove_dir(path: impl AsRef<Path>) -> Result<(), AssetIoError> {
    let path = path.as_ref();
    std::fs::remove_dir(path).map_err(|source| AssetIoError::new(AssetRoute::Delete, path, source))
}

/// Remove a file, tagging failure as [`AssetRoute::Delete`].
pub fn remove_file(path: impl AsRef<Path>) -> Result<(), AssetIoError> {
    let path = path.as_ref();
    std::fs::remove_file(path).map_err(|source| AssetIoError::new(AssetRoute::Delete, path, source))
}

/// Remove a directory tree, tagging failure as [`AssetRoute::Delete`].
pub fn remove_dir_all(path: impl AsRef<Path>) -> Result<(), AssetIoError> {
    let path = path.as_ref();
    std::fs::remove_dir_all(path)
        .map_err(|source| AssetIoError::new(AssetRoute::Delete, path, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_has_a_distinct_stable_name() {
        let names: Vec<&str> = [
            AssetRoute::Read,
            AssetRoute::Backup,
            AssetRoute::Write,
            AssetRoute::Delete,
        ]
        .iter()
        .map(|route| route.as_str())
        .collect();
        assert_eq!(names, ["read", "backup", "write", "delete"]);
        let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "route names must be distinguishable"
        );
    }

    #[test]
    fn a_read_failure_names_the_route_and_the_path() {
        let directory = tempfile::tempdir().expect("tempdir");
        let missing = directory.path().join("absent.md");
        let error = read_to_string(&missing).expect_err("reading an absent file must fail");
        assert_eq!(error.route, AssetRoute::Read);
        assert_eq!(error.path, missing);
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        let rendered = error.to_string();
        assert!(rendered.contains("read"), "{rendered}");
        assert!(
            rendered.contains(&missing.display().to_string()),
            "{rendered}"
        );
    }

    #[test]
    fn a_write_failure_names_the_write_route_and_the_destination() {
        let directory = tempfile::tempdir().expect("tempdir");
        // A path whose parent is a file cannot be written, on every platform.
        let blocker = directory.path().join("blocker");
        std::fs::write(&blocker, b"x").expect("seed blocker");
        let destination = blocker.join("child.md");
        let error = write(&destination, b"y").expect_err("writing under a file must fail");
        assert_eq!(error.route, AssetRoute::Write);
        assert_eq!(error.path, destination);
        assert!(error.to_string().contains("write"), "{error}");
    }

    #[test]
    fn a_backup_failure_names_the_backup_route_and_the_destination() {
        let directory = tempfile::tempdir().expect("tempdir");
        let source = directory.path().join("source.md");
        std::fs::write(&source, b"x").expect("seed source");
        let blocker = directory.path().join("blocker");
        std::fs::write(&blocker, b"x").expect("seed blocker");
        let destination = blocker.join("backup.md");
        let error = backup(&source, &destination).expect_err("backup under a file must fail");
        assert_eq!(error.route, AssetRoute::Backup);
        assert_eq!(
            error.path, destination,
            "a backup failure reports where the copy could not land"
        );
        assert!(error.to_string().contains("backup"), "{error}");
    }

    #[test]
    fn a_delete_failure_names_the_delete_route_and_the_path() {
        let directory = tempfile::tempdir().expect("tempdir");
        let missing = directory.path().join("absent.md");
        let error = remove_file(&missing).expect_err("removing an absent file must fail");
        assert_eq!(error.route, AssetRoute::Delete);
        assert_eq!(error.path, missing);
        assert!(error.to_string().contains("delete"), "{error}");
    }

    #[test]
    fn conversion_to_io_error_keeps_the_kind_and_carries_route_and_path() {
        let directory = tempfile::tempdir().expect("tempdir");
        let missing = directory.path().join("absent.md");
        let error = read_to_string(&missing).expect_err("reading an absent file must fail");
        let converted: std::io::Error = error.into();
        assert_eq!(
            converted.kind(),
            std::io::ErrorKind::NotFound,
            "callers that branch on NotFound must keep working"
        );
        let rendered = converted.to_string();
        assert!(rendered.contains("read"), "{rendered}");
        assert!(
            rendered.contains(&missing.display().to_string()),
            "{rendered}"
        );
    }
}
