//! All-or-nothing file publication (Issue #4360).
//!
//! `fs::write` creates or truncates the destination first and fills it
//! afterwards. Anything that watches for the path — a sibling process, a heal
//! loop, a test waiting on a marker — can therefore read it while it is still
//! empty, and the busier the host, the wider that window gets. It has produced
//! the same `left: "" / right: "<expected>"` failure in three separate places:
//! the index coordinator's cross-process markers (#4360), the daemon endpoint
//! descriptor (#3911), and the verification lease release channel (#4449).
//!
//! [`write_atomic`] closes that window by filling a sibling temp file and
//! renaming it over the destination, so the path appears only once it already
//! holds the whole payload.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to `path` so that no reader ever observes the destination
/// holding a partial payload.
///
/// The temp file is created next to the destination, which keeps the rename
/// within one filesystem — a rename across filesystems is not atomic and would
/// reintroduce the very race this exists to close. An existing destination is
/// replaced; a failed rename removes the temp file rather than leaving litter
/// behind.
///
/// This publishes atomically, it does not make the write durable: the payload
/// may still be lost to a machine crash because nothing is fsynced. Callers
/// that need crash durability must sync the file and its parent directory
/// themselves.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let temp = temp_path_for(path, parent);
    fs::write(&temp, bytes)?;
    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Builds a temp path beside `path` that no concurrent writer can collide
/// with: the process id separates hosts of the same file, and the counter
/// separates threads and repeated writes inside one process.
fn temp_path_for(path: &Path, parent: Option<&Path>) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let name = format!(
        ".{stem}.{}.{}.publishing",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    match parent {
        Some(parent) => parent.join(name),
        None => std::path::PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_preserves_the_previous_file_contents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("descriptor.json");
        let previous = dir.path().join("previous.json");
        let old = br#"{"turn":0}"#;
        let new = br#"{"turn":100,"complete":true}"#;
        write_atomic(&path, old).expect("publish initial descriptor");

        // Keep the old file observable without an open reader racing Windows
        // replacement. An in-place truncate/write would also change this link,
        // so the assertion catches that regression without scheduler timing.
        fs::hard_link(&path, &previous).expect("retain the previous file");
        write_atomic(&path, new).expect("publish replacement descriptor");

        assert_eq!(fs::read(&previous).expect("read previous descriptor"), old);
        assert_eq!(fs::read(&path).expect("read published descriptor"), new);
    }

    #[test]
    fn it_replaces_an_existing_destination_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("marker");
        fs::write(&path, b"stale").expect("seed the destination");

        write_atomic(&path, b"fresh").expect("publish over the stale value");

        assert_eq!(fs::read_to_string(&path).expect("read marker"), "fresh");
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("read arena")
            .map(|entry| entry.expect("dir entry").file_name())
            .filter(|name| name != "marker")
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files must not survive: {leftovers:?}"
        );
    }

    #[test]
    fn an_empty_payload_is_a_value_and_still_publishes() {
        // The verification lease release channel signals "no reason given"
        // with an empty file (#4449), so emptiness must stay writable — the
        // race this module closes is an empty file that nobody wrote yet.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("release");

        write_atomic(&path, b"").expect("publish an empty reason");

        assert!(
            path.exists(),
            "an empty payload must still publish the path"
        );
        assert_eq!(fs::read_to_string(&path).expect("read release"), "");
    }
}
