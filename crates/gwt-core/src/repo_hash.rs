//! Repository identification via normalized origin URL hashing.
//!
//! `RepoHash` is the SHA256[:16] of a canonicalized origin URL. The same
//! upstream repository (HTTPS clone, SSH clone, second worktree) always
//! resolves to the same `RepoHash`.

use std::fmt;
use std::path::Path;

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

/// Detect a `RepoHash` from the `origin` remote configured for `repo_root`.
pub fn detect_repo_hash(repo_root: &Path) -> Option<RepoHash> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return None;
    }
    Some(compute_repo_hash(&url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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

    #[test]
    fn detect_repo_hash_reads_origin_remote() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_git_repo(&repo);
        add_origin(&repo, "git@github.com:example/project.git");

        let actual = detect_repo_hash(&repo).expect("repo hash");

        assert_eq!(
            actual.as_str(),
            compute_repo_hash("https://github.com/example/project.git").as_str()
        );
    }

    #[test]
    fn detect_repo_hash_returns_same_hash_for_linked_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let wt = dir.path().join("wt-feature");
        init_git_repo(&repo);
        add_origin(&repo, "https://github.com/example/project.git");
        commit_file(&repo, "README.md", "# repo\n");

        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "feature/shared",
                wt.to_str().unwrap(),
            ])
            .current_dir(&repo)
            .output()
            .expect("git worktree add");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let repo_hash = detect_repo_hash(&repo).expect("repo hash");
        let wt_hash = detect_repo_hash(&wt).expect("worktree hash");
        assert_eq!(repo_hash.as_str(), wt_hash.as_str());
    }

    fn init_git_repo(path: &Path) {
        let output = std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .expect("git init");
        assert!(output.status.success(), "git init failed");

        let email = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .expect("git config user.email");
        assert!(email.status.success(), "git config user.email failed");

        let name = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("git config user.name");
        assert!(name.status.success(), "git config user.name failed");
    }

    fn add_origin(path: &Path, url: &str) {
        let output = std::process::Command::new("git")
            .args(["remote", "add", "origin", url])
            .current_dir(path)
            .output()
            .expect("git remote add origin");
        assert!(
            output.status.success(),
            "git remote add origin failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit_file(path: &Path, name: &str, body: &str) {
        std::fs::write(path.join(name), body).unwrap();
        let add = std::process::Command::new("git")
            .args(["add", name])
            .current_dir(path)
            .output()
            .expect("git add");
        assert!(add.status.success(), "git add failed");

        let commit = std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(path)
            .output()
            .expect("git commit");
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
    }
}
