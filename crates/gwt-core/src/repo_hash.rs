//! Repository identification via normalized origin URL hashing.
//!
//! `RepoHash` is the first 16 hex digits of the SHA256 of a canonicalized
//! origin URL (i.e., `SHA256(...)[..16]` in slice notation). The same
//! upstream repository (HTTPS clone, SSH clone, second worktree) always
//! resolves to the same `RepoHash`.

use std::{
    collections::HashSet,
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
    let local_configs = read_git_config_chain(
        &common_dir.join("config"),
        repo_root,
        &git_dir,
        &mut HashSet::new(),
        0,
    );
    let url = parse_origin_url_from_git_configs(&local_configs)?;
    let mut rewrite_configs = read_user_git_config_contents(repo_root, &git_dir);
    rewrite_configs.extend(local_configs);
    Some(apply_url_instead_of_rewrite(
        &url,
        &url_rewrite_rules_from_configs(&rewrite_configs),
    ))
}

fn resolve_git_dir(repo_root: &Path) -> Option<PathBuf> {
    let dot_git = repo_root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if dot_git.is_file() {
        return resolve_gitdir_file(&dot_git);
    }
    if repo_root.join("config").is_file() && repo_root.join("HEAD").is_file() {
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

fn read_git_config_chain(
    path: &Path,
    repo_root: &Path,
    git_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> Vec<String> {
    if depth > 8 {
        return Vec::new();
    }
    let key = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(key) {
        return Vec::new();
    }
    let Ok(config) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut configs = Vec::new();
    let mut current_config = String::new();
    let mut include_enabled = false;
    for raw_line in config.lines() {
        let line = raw_line.trim();
        let mut include_path = None;
        if !line.is_empty() && !line.starts_with('#') && !line.starts_with(';') {
            if let Some(section) = line
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
            {
                include_enabled = include_section_enabled(section.trim(), repo_root, git_dir);
            } else if include_enabled {
                if let Some((key, value)) = line.split_once('=') {
                    if key.trim().eq_ignore_ascii_case("path") {
                        let path = clean_git_config_value(value);
                        if !path.is_empty() {
                            include_path = Some(resolve_git_config_include_path(path, config_dir));
                        }
                    }
                }
            }
        }
        current_config.push_str(raw_line);
        current_config.push('\n');
        if let Some(include_path) = include_path {
            if !current_config.trim().is_empty() {
                configs.push(std::mem::take(&mut current_config));
            }
            configs.extend(read_git_config_chain(
                &include_path,
                repo_root,
                git_dir,
                visited,
                depth + 1,
            ));
        }
    }
    if !current_config.trim().is_empty() {
        configs.push(current_config);
    }
    configs
}

fn include_section_enabled(section: &str, repo_root: &Path, git_dir: &Path) -> bool {
    section.eq_ignore_ascii_case("include")
        || include_if_condition_from_section(section)
            .map(|condition| include_if_condition_matches(&condition, repo_root, git_dir))
            .unwrap_or(false)
}

fn include_if_condition_from_section(section: &str) -> Option<String> {
    if section.len() >= 9 && section[..9].eq_ignore_ascii_case("includeif") {
        let rest = section[9..].trim_start();
        if let Some(value) = rest.strip_prefix('.') {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
        let value = rest.trim();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            return Some(value[1..value.len() - 1].to_string());
        }
    }
    None
}

fn include_if_condition_matches(condition: &str, _repo_root: &Path, git_dir: &Path) -> bool {
    if let Some(pattern) = condition.strip_prefix("gitdir/i:") {
        return gitdir_pattern_matches(pattern, git_dir, true);
    }
    if let Some(pattern) = condition.strip_prefix("gitdir:") {
        return gitdir_pattern_matches(pattern, git_dir, false);
    }
    false
}

fn gitdir_pattern_matches(pattern: &str, git_dir: &Path, case_insensitive: bool) -> bool {
    let pattern = normalize_git_config_pattern(pattern);
    let mut candidate = normalize_path_for_config_match(git_dir);
    if !candidate.ends_with('/') {
        candidate.push('/');
    }
    let mut pattern = pattern.replace('\\', "/");
    if !pattern.ends_with('/') && !pattern.contains('*') && !pattern.contains('?') {
        pattern.push('/');
    }
    if case_insensitive {
        wildcard_match(&pattern.to_lowercase(), &candidate.to_lowercase())
    } else {
        wildcard_match(&pattern, &candidate)
    }
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        match pattern {
            [] => value.is_empty(),
            [b'*', rest @ ..] => {
                matches(rest, value) || (!value.is_empty() && matches(pattern, &value[1..]))
            }
            [b'?', rest @ ..] => !value.is_empty() && matches(rest, &value[1..]),
            [head, rest @ ..] => value.first() == Some(head) && matches(rest, &value[1..]),
        }
    }
    matches(pattern.as_bytes(), value.as_bytes())
}

fn resolve_git_config_include_path(path: &str, config_dir: &Path) -> PathBuf {
    let path = expand_home_path(path);
    if path.is_absolute() {
        path
    } else {
        config_dir.join(path)
    }
}

fn normalize_git_config_pattern(pattern: &str) -> String {
    let expanded = expand_home_path(pattern);
    if pattern.contains('*') || pattern.contains('?') {
        normalize_path_string(&expanded)
    } else {
        normalize_path_for_config_match(&expanded)
    }
}

fn normalize_path_for_config_match(path: &Path) -> String {
    normalize_path_string(&dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

fn normalize_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn expand_home_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir_from_env() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn parse_origin_url_from_git_configs(configs: &[String]) -> Option<String> {
    configs
        .iter()
        .filter_map(|config| parse_origin_url_from_git_config(config))
        .next_back()
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct UrlRewriteRule {
    base: String,
    instead_of: String,
}

fn url_rewrite_rules_from_configs(configs: &[String]) -> Vec<UrlRewriteRule> {
    configs
        .iter()
        .flat_map(|config| parse_url_rewrite_rules_from_git_config(config))
        .collect()
}

fn parse_url_rewrite_rules_from_git_config(config: &str) -> Vec<UrlRewriteRule> {
    let mut rules = Vec::new();
    let mut current_base = None;
    for raw_line in config.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            current_base = url_rewrite_base_from_section(section.trim());
            continue;
        }
        let Some(base) = current_base.as_ref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("insteadOf") {
            let instead_of = clean_git_config_value(value);
            if !instead_of.is_empty() {
                rules.push(UrlRewriteRule {
                    base: base.clone(),
                    instead_of: instead_of.to_string(),
                });
            }
        }
    }
    rules
}

fn url_rewrite_base_from_section(section: &str) -> Option<String> {
    if section.len() >= 4 && section[..3].eq_ignore_ascii_case("url") {
        let rest = section[3..].trim_start();
        if let Some(value) = rest.strip_prefix('.') {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
        let value = rest.trim();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            return Some(value[1..value.len() - 1].to_string());
        }
    }
    None
}

fn apply_url_instead_of_rewrite(url: &str, rules: &[UrlRewriteRule]) -> String {
    let Some(rule) = rules
        .iter()
        .filter(|rule| url.starts_with(&rule.instead_of))
        .max_by_key(|rule| rule.instead_of.len())
    else {
        return url.to_string();
    };
    format!("{}{}", rule.base, &url[rule.instead_of.len()..])
}

fn read_user_git_config_contents(repo_root: &Path, git_dir: &Path) -> Vec<String> {
    let mut visited = HashSet::new();
    user_git_config_paths()
        .into_iter()
        .flat_map(|path| read_git_config_chain(&path, repo_root, git_dir, &mut visited, 0))
        .collect()
}

fn user_git_config_paths() -> Vec<PathBuf> {
    if let Some(path) = non_empty_env_path("GIT_CONFIG_GLOBAL") {
        return vec![path];
    }
    let mut paths = Vec::new();
    if let Some(xdg_config_home) = non_empty_env_path("XDG_CONFIG_HOME") {
        paths.push(xdg_config_home.join("git").join("config"));
    } else if let Some(home) = home_dir_from_env() {
        paths.push(home.join(".config").join("git").join("config"));
    }
    if let Some(home) = home_dir_from_env() {
        paths.push(home.join(".gitconfig"));
    }
    paths
}

fn home_dir_from_env() -> Option<PathBuf> {
    non_empty_env_path("HOME").or_else(|| non_empty_env_path("USERPROFILE"))
}

fn non_empty_env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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
    fn detect_repo_hash_ignores_plain_directory_with_config_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config"),
            r#"
[remote "origin"]
    url = https://github.com/example/not-a-repo.git
"#,
        )
        .unwrap();

        assert_eq!(detect_repo_hash(dir.path()), None);
    }

    #[test]
    fn detect_repo_hash_applies_local_url_instead_of_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            repo.join(".git/config"),
            r#"
[url "git@github.com:"]
    insteadOf = gh:
[remote "origin"]
    url = gh:example/rewrite.git
    fetch = +refs/heads/*:refs/remotes/origin/*
"#,
        )
        .unwrap();

        let actual = detect_repo_hash(&repo).expect("repo hash");

        assert_eq!(
            actual.as_str(),
            compute_repo_hash("git@github.com:example/rewrite.git").as_str()
        );
    }

    #[test]
    fn detect_repo_hash_applies_url_rewrite_from_included_config() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            repo.join(".git/config"),
            r#"
[include]
    path = rewrites.inc
[remote "origin"]
    url = gh:example/included.git
    fetch = +refs/heads/*:refs/remotes/origin/*
"#,
        )
        .unwrap();
        fs::write(
            repo.join(".git/rewrites.inc"),
            r#"
[url "git@github.com:"]
    insteadOf = gh:
"#,
        )
        .unwrap();

        let actual = detect_repo_hash(&repo).expect("repo hash");

        assert_eq!(
            actual.as_str(),
            compute_repo_hash("git@github.com:example/included.git").as_str()
        );
    }

    #[test]
    fn detect_repo_hash_preserves_include_insertion_order_for_origin_url() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            repo.join(".git/config"),
            r#"
[include]
    path = included-origin.inc
[remote "origin"]
    url = https://github.com/example/local.git
    fetch = +refs/heads/*:refs/remotes/origin/*
"#,
        )
        .unwrap();
        fs::write(
            repo.join(".git/included-origin.inc"),
            r#"
[remote "origin"]
    url = https://github.com/example/included.git
    fetch = +refs/heads/*:refs/remotes/origin/*
"#,
        )
        .unwrap();

        let actual = detect_repo_hash(&repo).expect("repo hash");

        assert_eq!(
            actual.as_str(),
            compute_repo_hash("https://github.com/example/local.git").as_str()
        );
    }

    #[test]
    fn detect_repo_hash_applies_url_rewrite_from_matching_include_if_config() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let git_dir = repo.join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let git_dir_pattern = format!("{}/", git_dir.display());
        fs::write(
            git_dir.join("config"),
            format!(
                r#"
[includeIf "gitdir:{git_dir_pattern}"]
    path = conditional.inc
[remote "origin"]
    url = gh:example/conditional.git
    fetch = +refs/heads/*:refs/remotes/origin/*
"#
            ),
        )
        .unwrap();
        fs::write(
            git_dir.join("conditional.inc"),
            r#"
[url "git@github.com:"]
    insteadOf = gh:
"#,
        )
        .unwrap();

        let actual = detect_repo_hash(&repo).expect("repo hash");

        assert_eq!(
            actual.as_str(),
            compute_repo_hash("git@github.com:example/conditional.git").as_str()
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
