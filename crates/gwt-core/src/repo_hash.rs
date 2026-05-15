//! Repository identification via normalized origin URL hashing.
//!
//! `RepoHash` is the first 16 hex digits of the SHA256 of a canonicalized
//! origin URL (i.e., `SHA256(...)[..16]` in slice notation). The same
//! upstream repository (HTTPS clone, SSH clone, second worktree) always
//! resolves to the same `RepoHash`.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

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

/// Compute a deterministic fallback hash from a canonical filesystem path.
pub fn compute_path_hash(path: &Path) -> RepoHash {
    let canonical = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let normalized = canonical.to_string_lossy().replace('\\', "/");
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    let hex_full = hex::encode(digest);
    RepoHash(hex_full[..HASH_HEX_LEN].to_string())
}

/// Detect a `RepoHash` from the `origin` remote configured for `repo_root`.
pub fn detect_repo_hash(repo_root: &Path) -> Option<RepoHash> {
    let url = origin_url_from_git_config(repo_root)?;
    if url.is_empty() {
        return None;
    }
    Some(compute_repo_hash(&url))
}

fn origin_url_from_git_config(repo_root: &Path) -> Option<String> {
    let git_dir = resolve_git_dir(repo_root)?;
    let common_dir = resolve_common_git_dir(&git_dir);
    let config = fs::read_to_string(common_dir.join("config")).ok()?;
    parse_origin_url_from_git_config(&config)
}

fn resolve_git_dir(repo_root: &Path) -> Option<PathBuf> {
    let dot_git = repo_root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if dot_git.is_file() {
        return resolve_gitdir_file(&dot_git);
    }
    if repo_root.join("config").is_file() {
        return Some(repo_root.to_path_buf());
    }
    None
}

fn resolve_gitdir_file(dot_git: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(dot_git).ok()?;
    let raw = contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))?
        .trim();
    if raw.is_empty() {
        return None;
    }
    let git_dir = PathBuf::from(raw);
    if git_dir.is_absolute() {
        Some(git_dir)
    } else {
        Some(dot_git.parent()?.join(git_dir))
    }
}

fn resolve_common_git_dir(git_dir: &Path) -> PathBuf {
    let Ok(contents) = fs::read_to_string(git_dir.join("commondir")) else {
        return git_dir.to_path_buf();
    };
    let raw = contents.lines().next().unwrap_or_default().trim();
    if raw.is_empty() {
        return git_dir.to_path_buf();
    }
    let common_dir = PathBuf::from(raw);
    if common_dir.is_absolute() {
        common_dir
    } else {
        git_dir.join(common_dir)
    }
}

fn parse_origin_url_from_git_config(config: &str) -> Option<String> {
    let mut in_origin_remote = false;
    for raw_line in config.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            in_origin_remote = is_origin_remote_section(section.trim());
            continue;
        }
        if !in_origin_remote {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("url") {
            let url = clean_git_config_value(value);
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
    }
    None
}

fn is_origin_remote_section(section: &str) -> bool {
    section.eq_ignore_ascii_case(r#"remote "origin""#)
        || section.eq_ignore_ascii_case("remote.origin")
}

fn clean_git_config_value(value: &str) -> &str {
    let trimmed = value.trim();
    let trimmed = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(trimmed);
    let mut end = trimmed.len();
    for marker in [" #", "\t#", " ;", "\t;"] {
        if let Some(index) = trimmed.find(marker) {
            end = end.min(index);
        }
    }
    trimmed[..end].trim()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use crate::process::scrub_git_env;

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
    fn detect_repo_hash_reads_origin_remote_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git/config"),
            r#"
[core]
    repositoryformatversion = 0
[remote "origin"]
    url = https://github.com/example/config-only.git
    fetch = +refs/heads/*:refs/remotes/origin/*
"#,
        )
        .unwrap();

        let actual = detect_repo_hash(&repo).expect("repo hash");

        assert_eq!(
            actual.as_str(),
            compute_repo_hash("https://github.com/example/config-only.git").as_str()
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

        let mut wt_cmd = crate::process::hidden_command("git");
        wt_cmd
            .args([
                "worktree",
                "add",
                "-b",
                "feature/shared",
                wt.to_str().unwrap(),
            ])
            .current_dir(&repo);
        scrub_git_env(&mut wt_cmd);
        let output = wt_cmd.output().expect("git worktree add");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let repo_hash = detect_repo_hash(&repo).expect("repo hash");
        let wt_hash = detect_repo_hash(&wt).expect("worktree hash");
        assert_eq!(repo_hash.as_str(), wt_hash.as_str());
    }

    #[test]
    fn compute_path_hash_is_deterministic_for_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = compute_path_hash(dir.path());
        let b = compute_path_hash(dir.path());
        assert_eq!(a.as_str(), b.as_str());
    }

    fn init_git_repo(path: &Path) {
        let mut init_cmd = crate::process::hidden_command("git");
        init_cmd.args(["init", path.to_str().unwrap()]);
        scrub_git_env(&mut init_cmd);
        let output = init_cmd.output().expect("git init");
        assert!(output.status.success(), "git init failed");

        let mut email_cmd = crate::process::hidden_command("git");
        email_cmd
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path);
        scrub_git_env(&mut email_cmd);
        let email = email_cmd.output().expect("git config user.email");
        assert!(email.status.success(), "git config user.email failed");

        let mut name_cmd = crate::process::hidden_command("git");
        name_cmd
            .args(["config", "user.name", "Test User"])
            .current_dir(path);
        scrub_git_env(&mut name_cmd);
        let name = name_cmd.output().expect("git config user.name");
        assert!(name.status.success(), "git config user.name failed");
    }

    fn add_origin(path: &Path, url: &str) {
        let mut cmd = crate::process::hidden_command("git");
        cmd.args(["remote", "add", "origin", url]).current_dir(path);
        scrub_git_env(&mut cmd);
        let output = cmd.output().expect("git remote add origin");
        assert!(
            output.status.success(),
            "git remote add origin failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit_file(path: &Path, name: &str, body: &str) {
        std::fs::write(path.join(name), body).unwrap();
        let mut add_cmd = crate::process::hidden_command("git");
        add_cmd.args(["add", name]).current_dir(path);
        scrub_git_env(&mut add_cmd);
        let add = add_cmd.output().expect("git add");
        assert!(add.status.success(), "git add failed");

        let mut commit_cmd = crate::process::hidden_command("git");
        commit_cmd.args(["commit", "-m", "init"]).current_dir(path);
        scrub_git_env(&mut commit_cmd);
        let commit = commit_cmd.output().expect("git commit");
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
    }
}
