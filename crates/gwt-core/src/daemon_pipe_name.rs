//! Windows named-pipe naming for the runtime daemon (Issue #3526).
//!
//! Kept in its own file with no crate dependencies so the Windows transport
//! can be type-checked from a non-Windows host by including this file into a
//! scratch crate; see `.github/workflows/test.yml` for the runtime check.

/// Prefix every local named pipe lives under.
pub const WINDOWS_PIPE_PREFIX: &str = r"\\.\pipe\";

/// Windows named-pipe name for a daemon bind string.
///
/// A value that already lives in the pipe namespace is returned unchanged,
/// so a persisted endpoint `bind` round-trips. Any other value (the
/// endpoint file path, a test's temp path, a legacy `.sock` path) is hashed
/// into a unique, length-bounded pipe name: pipe names cannot contain path
/// separators, and a distinct gwt home / [`RuntimeScope`] pair must never
/// share a pipe with another one.
///
/// [`RuntimeScope`]: crate::daemon::RuntimeScope
pub fn windows_pipe_name_for(bind: &str) -> String {
    if has_windows_pipe_prefix(bind) {
        return bind.to_string();
    }
    // Split on both separators by hand so the derived name is identical on
    // every host (a `\` is not a separator for `std::path` on Unix).
    let stem = bind
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("daemon")
        .rsplit_once('.')
        .map_or("daemon", |(stem, _)| stem);
    let stem = stem
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .take(32)
        .collect::<String>();
    let digest = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bind.as_bytes());
        hex::encode(hasher.finalize())
    };
    format!("{WINDOWS_PIPE_PREFIX}gwtd-{stem}-{}", &digest[..16])
}

/// Whether `bind` already names a pipe in the local pipe namespace
/// (case-insensitive prefix match). `str::get` keeps a bind whose UTF-8
/// boundary falls inside the prefix width from panicking; it is simply not a
/// pipe name.
pub fn has_windows_pipe_prefix(bind: &str) -> bool {
    bind.get(..WINDOWS_PIPE_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(WINDOWS_PIPE_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multibyte_bind_inside_the_prefix_width_is_not_a_pipe_name_and_does_not_panic() {
        assert!(!has_windows_pipe_prefix("ééééé"));
        assert!(!has_windows_pipe_prefix("日本語"));
        assert!(windows_pipe_name_for("ééééé").starts_with(WINDOWS_PIPE_PREFIX));
        assert!(has_windows_pipe_prefix(r"\\.\PIPE\x"));
    }

    #[test]
    fn pipe_name_is_stable_unique_and_idempotent() {
        let a = windows_pipe_name_for(r"C:\Users\me\.gwt\projects\r\runtime\daemon\abcd.json");
        let b = windows_pipe_name_for(r"C:\Users\me\.gwt\projects\r\runtime\daemon\abcd.json");
        let c = windows_pipe_name_for(r"C:\Users\other\.gwt\projects\r\runtime\daemon\abcd.json");
        assert_eq!(a, b, "same path must map to the same pipe name");
        assert_ne!(a, c, "different homes must not share a pipe name");
        assert!(a.starts_with(r"\\.\pipe\gwtd-abcd-"), "{a}");
        assert!(a.len() < 256, "pipe names are limited to 256 characters");
        assert_eq!(
            windows_pipe_name_for(&a),
            a,
            "pipe names must round-trip unchanged"
        );
        assert_eq!(
            windows_pipe_name_for(r"\\.\PIPE\already-a-pipe"),
            r"\\.\PIPE\already-a-pipe"
        );
        assert!(windows_pipe_name_for("/tmp/x/daemon.sock").starts_with(r"\\.\pipe\gwtd-daemon-"));
    }
}
