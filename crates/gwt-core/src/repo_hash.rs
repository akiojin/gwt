//! Repository identification via normalized origin URL hashing.
//!
//! `RepoHash` is the SHA256[:16] of a canonicalized origin URL. The same
//! upstream repository (HTTPS clone, SSH clone, second worktree) always
//! resolves to the same `RepoHash`.

use std::fmt;

use sha2::{Digest, Sha256};

const HASH_HEX_LEN: usize = 16;

/// 16-character lowercase hex SHA256 prefix identifying a repository.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoHash(String);

impl RepoHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Normalize an origin URL to the canonical `host/path` form used by repo hashing.
///
/// Examples — these all produce `github.com/akiojin/gwt`:
///
/// - `https://github.com/akiojin/gwt.git`
/// - `https://github.com/Akiojin/gwt`
/// - `git@github.com:akiojin/gwt.git`
/// - `ssh://git@github.com:22/akiojin/gwt.git`
pub fn normalize_origin_url(url: &str) -> String {
    let trimmed = url.trim();

    // Strip surrounding whitespace and trailing slashes; keep working on a copy.
    let mut s = trimmed.to_string();
    while s.ends_with('/') {
        s.pop();
    }

    // 1. SSH shorthand: git@host:user/repo[.git]
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some(idx) = rest.find(':') {
            let host = &rest[..idx];
            let path = &rest[idx + 1..];
            return finalize_normalized(host, path);
        }
    }

    // 2. Scheme://[user[:pass]@]host[:port]/path
    if let Some(scheme_end) = s.find("://") {
        let after_scheme = &s[scheme_end + 3..];
        // Drop leading user[:pass]@
        let after_user = match after_scheme.find('@') {
            Some(at) => &after_scheme[at + 1..],
            None => after_scheme,
        };
        if let Some(slash) = after_user.find('/') {
            let host_port = &after_user[..slash];
            let host = host_port.split(':').next().unwrap_or(host_port);
            let path = &after_user[slash + 1..];
            return finalize_normalized(host, path);
        }
    }

    // 3. Bare `host/path` form (already mostly normalized).
    if let Some(slash) = s.find('/') {
        let host = &s[..slash];
        let path = &s[slash + 1..];
        return finalize_normalized(host, path);
    }

    s.to_lowercase()
}

fn finalize_normalized(host: &str, path: &str) -> String {
    let mut path = path.trim_matches('/').to_string();
    if let Some(stripped) = path.strip_suffix(".git") {
        path = stripped.to_string();
    }
    format!("{}/{}", host.to_lowercase(), path.to_lowercase())
}

/// Compute a `RepoHash` from a raw origin URL.
pub fn compute_repo_hash(origin_url: &str) -> RepoHash {
    let normalized = normalize_origin_url(origin_url);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    let hex_full = hex::encode(digest);
    RepoHash(hex_full[..HASH_HEX_LEN].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_https_https_form() {
        assert_eq!(
            normalize_origin_url("https://github.com/Akiojin/gwt.git"),
            "github.com/akiojin/gwt"
        );
    }

    #[test]
    fn normalizes_ssh_shorthand() {
        assert_eq!(
            normalize_origin_url("git@github.com:akiojin/gwt.git"),
            "github.com/akiojin/gwt"
        );
    }

    #[test]
    fn normalizes_ssh_protocol_form() {
        assert_eq!(
            normalize_origin_url("ssh://git@github.com:22/akiojin/gwt.git"),
            "github.com/akiojin/gwt"
        );
    }

    #[test]
    fn https_and_ssh_yield_same_hash() {
        let a = compute_repo_hash("https://github.com/akiojin/gwt.git");
        let b = compute_repo_hash("git@github.com:akiojin/gwt.git");
        assert_eq!(a.as_str(), b.as_str());
    }

    #[test]
    fn hash_is_16_lowercase_hex_chars() {
        let h = compute_repo_hash("https://github.com/akiojin/gwt.git");
        assert_eq!(h.as_str().len(), 16);
        assert!(h
            .as_str()
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
