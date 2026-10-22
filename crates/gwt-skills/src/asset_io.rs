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
//! These helpers attach the route and path when an operation fails. Managed
//! reads and publication retry transient Windows sharing failures for up to
//! one second of backoff; all other failures retain their original behavior.

use std::fmt;
use std::path::{Path, PathBuf};

use gwt_core::atomic_file::retry_on_sharing_violation;

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
    retry_on_sharing_violation(|| std::fs::read_to_string(path))
        .map_err(|source| AssetIoError::new(AssetRoute::Read, path, source))
}

/// Read a file's bytes, tagging failure as [`AssetRoute::Read`].
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, AssetIoError> {
    let path = path.as_ref();
    retry_on_sharing_violation(|| std::fs::read(path))
        .map_err(|source| AssetIoError::new(AssetRoute::Read, path, source))
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
    let from = from.as_ref();
    let to = to.as_ref();
    #[cfg(windows)]
    let result = retry_on_sharing_violation(|| rename_with_posix_semantics(from, to));
    #[cfg(not(windows))]
    let result = std::fs::rename(from, to);
    result.map_err(|source| AssetIoError::new(AssetRoute::Write, to, source))
}

#[cfg(windows)]
fn rename_with_posix_semantics(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            FileRenameInfoEx, SetFileInformationByHandle, DELETE, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_RENAME_INFO,
        },
        System::WindowsProgramming::{
            FILE_RENAME_FLAG_POSIX_SEMANTICS, FILE_RENAME_FLAG_REPLACE_IF_EXISTS,
        },
    };

    let destination = std::path::absolute(to)?;
    let mut name: Vec<u16> = destination.as_os_str().encode_wide().collect();
    if name.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "asset destination contains a null character",
        ));
    }
    let name_bytes = u32::try_from(name.len() * size_of::<u16>()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "asset path too long")
    })?;
    name.push(0);
    let buffer_bytes =
        std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * size_of::<u16>();
    let buffer_size = u32::try_from(buffer_bytes).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "asset path too long")
    })?;
    // The typed allocation provides both alignment and space for the trailing
    // UTF-16 name, which extends beyond FILE_RENAME_INFO's one-element field.
    let mut buffer =
        vec![FILE_RENAME_INFO::default(); buffer_bytes.div_ceil(size_of::<FILE_RENAME_INFO>())];
    let info = buffer.as_mut_ptr();
    let source = std::fs::OpenOptions::new()
        .access_mode(DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(from)?;

    // SAFETY: info is aligned, holds buffer_size bytes, and remains alive for
    // the synchronous call. The file owns the handle throughout the call.
    unsafe {
        (*info).Anonymous.Flags =
            FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS;
        (*info).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
        // POSIX replacement preserves existing handles. Concurrent path opens
        // can still report sharing failures, handled by the bounded retry above.
        // Do not fall back to the legacy operation on unsupported filesystems.
        SetFileInformationByHandle(
            HANDLE(source.as_raw_handle()),
            FileRenameInfoEx,
            info.cast(),
            buffer_size,
        )
        // This BOOL API returns HRESULT_FROM_WIN32; retain the native error kind.
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0 & 0xffff))
    }
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

    #[cfg(windows)]
    #[test]
    fn sharing_retry_recovers_native_conflicts_but_returns_other_errors() {
        let mut attempts = 0;
        let value = retry_on_sharing_violation(|| {
            attempts += 1;
            match attempts {
                1 => Err(std::io::Error::from_raw_os_error(5)),
                2 => Err(std::io::Error::from_raw_os_error(32)),
                _ => Ok("complete asset"),
            }
        });
        assert_eq!(value.unwrap(), "complete asset");
        assert_eq!(attempts, 3);

        let mut attempts = 0;
        let error = retry_on_sharing_violation::<()>(|| {
            attempts += 1;
            Err(std::io::Error::from_raw_os_error(2))
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(2));
        assert_eq!(attempts, 1, "missing files must fail immediately");
    }

    #[cfg(windows)]
    #[test]
    fn sharing_retry_is_bounded_and_preserves_the_original_error() {
        let mut attempts = 0;
        let error = retry_on_sharing_violation::<()>(|| {
            attempts += 1;
            Err(std::io::Error::from_raw_os_error(if attempts == 1 {
                5
            } else {
                32
            }))
        })
        .unwrap_err();
        assert_eq!(attempts, 11, "initial attempt plus bounded backoff");
        assert_eq!(error.raw_os_error(), Some(5));
    }

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
