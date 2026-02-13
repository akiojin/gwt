//! IPC module for inter-pane communication via Unix domain socket.
//!
//! Provides an IPC server at `~/.gwt/gwt.sock` for send_keys and
//! other pane-to-pane communication. Falls back gracefully when
//! socket creation fails (FR-065).

use std::fs;
use std::path::{Path, PathBuf};

use crate::terminal::TerminalError;

/// IPC server for inter-pane communication.
///
/// Manages a Unix domain socket at `~/.gwt/gwt.sock` with permissions
/// restricted to the owning user (0700 on the parent directory).
/// When the socket cannot be created, the server enters a fallback
/// state where `active` is false and basic features continue to work.
pub struct IpcServer {
    socket_path: PathBuf,
    active: bool,
}

impl Default for IpcServer {
    fn default() -> Self {
        Self::new()
    }
}

impl IpcServer {
    /// Create a new IPC server.
    ///
    /// Attempts to prepare the socket path at `~/.gwt/gwt.sock`.
    /// If the directory cannot be created or the path is otherwise
    /// invalid, the server falls back to an inactive state (FR-065).
    pub fn new() -> Self {
        match Self::prepare_socket_path() {
            Ok(socket_path) => Self {
                socket_path,
                active: true,
            },
            Err(_) => Self {
                socket_path: PathBuf::new(),
                active: false,
            },
        }
    }

    /// Create an IPC server with an explicit socket path.
    ///
    /// Useful for testing with temporary directories.
    pub fn with_path(socket_path: PathBuf) -> Result<Self, TerminalError> {
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent).map_err(|e| TerminalError::IpcError {
                details: format!("failed to create socket directory: {e}"),
            })?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = fs::Permissions::from_mode(0o700);
                fs::set_permissions(parent, perms).map_err(|e| TerminalError::IpcError {
                    details: format!("failed to set directory permissions: {e}"),
                })?;
            }
        }

        // Remove stale socket file if it exists
        if socket_path.exists() {
            fs::remove_file(&socket_path).map_err(|e| TerminalError::IpcError {
                details: format!("failed to remove stale socket: {e}"),
            })?;
        }

        Ok(Self {
            socket_path,
            active: true,
        })
    }

    /// Send keys to a target pane's PTY.
    ///
    /// This is a placeholder that returns an error when the IPC
    /// server is inactive. The actual send_keys implementation will
    /// require a reference to PaneManager to route the keys.
    pub fn send_keys(&self, pane_id: &str, keys: &[u8]) -> Result<(), TerminalError> {
        if !self.active {
            return Err(TerminalError::IpcError {
                details: "IPC server is not active".to_string(),
            });
        }

        if pane_id.is_empty() {
            return Err(TerminalError::IpcError {
                details: "pane_id must not be empty".to_string(),
            });
        }

        if keys.is_empty() {
            return Ok(());
        }

        // Placeholder: actual routing to PaneManager will be wired
        // when the integration layer is built.
        tracing::debug!(
            pane_id = pane_id,
            bytes = keys.len(),
            "send_keys: queued for delivery"
        );
        Ok(())
    }

    /// Remove the socket file from disk.
    ///
    /// Should be called on gwt exit to ensure clean shutdown (FR-055).
    pub fn cleanup(&self) -> Result<(), TerminalError> {
        if self.socket_path.as_os_str().is_empty() {
            return Ok(());
        }
        if self.socket_path.exists() {
            fs::remove_file(&self.socket_path).map_err(|e| TerminalError::IpcError {
                details: format!("failed to remove socket file: {e}"),
            })?;
        }
        Ok(())
    }

    /// Returns the socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Returns whether the IPC server is active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Build the default socket path (`~/.gwt/gwt.sock`).
    fn prepare_socket_path() -> Result<PathBuf, TerminalError> {
        let home = dirs::home_dir().ok_or_else(|| TerminalError::IpcError {
            details: "failed to determine home directory".to_string(),
        })?;
        let dir = home.join(".gwt");
        fs::create_dir_all(&dir).map_err(|e| TerminalError::IpcError {
            details: format!("failed to create .gwt directory: {e}"),
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o700);
            fs::set_permissions(&dir, perms).map_err(|e| TerminalError::IpcError {
                details: format!("failed to set directory permissions: {e}"),
            })?;
        }

        Ok(dir.join("gwt.sock"))
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- Test 1: socket_path returns expected path ---

    #[test]
    fn test_socket_path_returns_expected_path() {
        let tmp = TempDir::new().unwrap();
        let expected = tmp.path().join("gwt.sock");
        let server = IpcServer::with_path(expected.clone()).unwrap();
        assert_eq!(server.socket_path(), expected.as_path());
    }

    // --- Test 2: cleanup removes socket file ---

    #[test]
    fn test_cleanup_removes_socket_file() {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("gwt.sock");

        // Create a dummy socket file to simulate an existing socket
        fs::write(&sock_path, b"").unwrap();
        assert!(sock_path.exists());

        let server = IpcServer {
            socket_path: sock_path.clone(),
            active: true,
        };
        server.cleanup().unwrap();
        assert!(!sock_path.exists());
    }

    // --- Test 3: fallback when socket path is invalid ---

    #[test]
    fn test_fallback_on_invalid_path() {
        // A path under /dev/null/... is invalid and cannot be created
        let bad_path = PathBuf::from("/dev/null/impossible/gwt.sock");
        let result = IpcServer::with_path(bad_path);
        assert!(result.is_err());
    }

    // --- Test 4: IpcServer default state via new() ---

    #[test]
    fn test_new_creates_active_server() {
        let server = IpcServer::new();
        // new() should produce an active server (assuming home dir exists)
        assert!(server.is_active());
        assert!(server.socket_path().to_str().unwrap().contains("gwt.sock"));
    }

    // --- Test 5: with_path sets correct active state ---

    #[test]
    fn test_with_path_sets_active() {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("gwt.sock");
        let server = IpcServer::with_path(sock_path).unwrap();
        assert!(server.is_active());
    }

    // --- Test 6: send_keys succeeds on active server ---

    #[test]
    fn test_send_keys_on_active_server() {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("gwt.sock");
        let server = IpcServer::with_path(sock_path).unwrap();

        let result = server.send_keys("pane-1", b"ls\n");
        assert!(result.is_ok());
    }

    // --- Test 7: send_keys fails on inactive server ---

    #[test]
    fn test_send_keys_on_inactive_server() {
        let server = IpcServer {
            socket_path: PathBuf::new(),
            active: false,
        };

        let result = server.send_keys("pane-1", b"ls\n");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not active"),
            "Expected 'not active' in: {err_msg}"
        );
    }

    // --- Test 8: send_keys with empty pane_id returns error ---

    #[test]
    fn test_send_keys_empty_pane_id() {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("gwt.sock");
        let server = IpcServer::with_path(sock_path).unwrap();

        let result = server.send_keys("", b"ls\n");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("pane_id"),
            "Expected 'pane_id' in: {err_msg}"
        );
    }

    // --- Test 9: send_keys with empty keys is no-op ---

    #[test]
    fn test_send_keys_empty_keys() {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("gwt.sock");
        let server = IpcServer::with_path(sock_path).unwrap();

        let result = server.send_keys("pane-1", b"");
        assert!(result.is_ok());
    }

    // --- Test 10: cleanup on empty path is no-op ---

    #[test]
    fn test_cleanup_empty_path_is_noop() {
        let server = IpcServer {
            socket_path: PathBuf::new(),
            active: false,
        };
        let result = server.cleanup();
        assert!(result.is_ok());
    }

    // --- Test 11: cleanup on non-existent file is no-op ---

    #[test]
    fn test_cleanup_nonexistent_file_is_noop() {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("does-not-exist.sock");
        let server = IpcServer {
            socket_path: sock_path,
            active: true,
        };
        let result = server.cleanup();
        assert!(result.is_ok());
    }

    // --- Test 12: with_path removes stale socket ---

    #[test]
    fn test_with_path_removes_stale_socket() {
        let tmp = TempDir::new().unwrap();
        let sock_path = tmp.path().join("gwt.sock");

        // Create a stale file
        fs::write(&sock_path, b"stale").unwrap();
        assert!(sock_path.exists());

        // with_path should remove it
        let _server = IpcServer::with_path(sock_path.clone()).unwrap();
        assert!(!sock_path.exists());
    }

    // --- Test 13: directory permissions (Unix only) ---

    #[cfg(unix)]
    #[test]
    fn test_directory_permissions_0700() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("ipc-dir");
        let sock_path = sub.join("gwt.sock");

        let _server = IpcServer::with_path(sock_path).unwrap();

        let meta = fs::metadata(&sub).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "Expected 0700, got {:o}", mode);
    }
}
