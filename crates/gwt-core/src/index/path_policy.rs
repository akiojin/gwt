use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

#[cfg(windows)]
use std::{collections::HashMap, ffi::OsString, os::windows::fs::MetadataExt};

use ignore::{
    gitignore::{Gitignore, GitignoreBuilder},
    Match,
};
use serde::Deserialize;

const INDEX_PATH_POLICY_SOURCE: &str = include_str!("../../runtime/index_path_policy.json");
static INDEX_PATH_POLICY: OnceLock<IndexPathPolicy> = OnceLock::new();

#[cfg(all(test, windows))]
thread_local! {
    static PATH_CANONICALIZE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexPathPolicy {
    // Keep this required u32 in the deserialization contract even though path
    // matching does not inspect it; removing it would accept missing or non-u32 values.
    #[allow(dead_code)]
    pub schema_version: u32,
    pub max_file_size: u64,
    pub allow_paths: HashSet<String>,
    pub deny_root_prefixes: HashSet<String>,
    pub deny_directory_names: HashSet<String>,
    pub deny_file_extensions: HashSet<String>,
    pub binary_extensions: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectIgnoreMatcher {
    scopes: Vec<ScopedGitignore>,
}

#[derive(Debug, Clone)]
struct ScopedGitignore {
    base: PathBuf,
    gitignore: Gitignore,
}

impl ProjectIgnoreMatcher {
    fn is_ignored(&self, _root: &Path, path: &Path) -> bool {
        let canonical_path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let is_dir = path.is_dir();
        let mut ignored = false;
        for scope in &self.scopes {
            if !canonical_path.starts_with(&scope.base) {
                continue;
            }
            let rel = canonical_path
                .strip_prefix(&scope.base)
                .unwrap_or(&canonical_path);
            match scope.gitignore.matched_path_or_any_parents(rel, is_dir) {
                Match::Ignore(_) => ignored = true,
                Match::Whitelist(_) => ignored = false,
                Match::None => {}
            }
        }
        ignored
    }
}

pub fn default_index_path_policy() -> IndexPathPolicy {
    INDEX_PATH_POLICY
        .get_or_init(|| {
            serde_json::from_str(INDEX_PATH_POLICY_SOURCE)
                .expect("bundled index_path_policy.json must be valid")
        })
        .clone()
}

pub fn build_project_ignore_matcher(root: &Path) -> ProjectIgnoreMatcher {
    let policy = default_index_path_policy();
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut scopes = Vec::new();
    add_gitignore_files(&policy, &root, &root, &mut scopes);
    ProjectIgnoreMatcher { scopes }
}

impl IndexPathPolicy {
    pub fn is_indexable_path(
        &self,
        matcher: &ProjectIgnoreMatcher,
        root: &Path,
        path: &Path,
    ) -> bool {
        if self.is_allowlisted(root, path) {
            return true;
        }
        if self.is_builtin_denied_path(root, path) {
            return false;
        }
        !matcher.is_ignored(root, path)
    }

    pub fn is_builtin_denied_path(&self, root: &Path, path: &Path) -> bool {
        let rel = relative_path(root, path);
        self.is_builtin_denied_relative_path(&rel, path)
    }

    /// Filter one inventory without reopening its root for every candidate.
    /// Resolve reparse targets afresh; metadata is shared only within this operation.
    pub fn filter_builtin_denied_paths(
        &self,
        root: &Path,
        mut paths: Vec<PathBuf>,
    ) -> Vec<PathBuf> {
        let canonical_root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        #[cfg(windows)]
        let ordinary_root = fs::symlink_metadata(root)
            .is_ok_and(|metadata| metadata.is_dir() && metadata.file_attributes() & 0x400 == 0);
        #[cfg(windows)]
        let mut ordinary_directories = HashMap::new();
        paths.retain(|path| {
            #[cfg(windows)]
            if ordinary_root
                && path.strip_prefix(root).is_ok_and(|relative| {
                    self.is_builtin_denied_relative_path(relative, path)
                        && ordinary_batch_path(root, relative, &mut ordinary_directories)
                })
            {
                return false;
            }
            let rel = relative_path_with_canonical_root(root, &canonical_root, path);
            !self.is_builtin_denied_relative_path(&rel, path)
        });
        paths
    }

    fn is_builtin_denied_relative_path(&self, rel: &Path, path: &Path) -> bool {
        let mut normal_components = rel.components().filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            _ => None,
        });

        if let Some(first) = normal_components.next() {
            if self.deny_root_prefixes.contains(first) {
                return true;
            }
            if self.deny_directory_names.contains(first) {
                return true;
            }
        }

        for component in normal_components {
            if self.deny_directory_names.contains(component) {
                return true;
            }
        }

        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!(".{}", extension.to_ascii_lowercase()))
            .is_some_and(|extension| self.deny_file_extensions.contains(&extension))
    }

    pub fn is_allowlisted(&self, root: &Path, path: &Path) -> bool {
        self.allow_paths
            .contains(&normalize_rel_path(&relative_path(root, path)))
    }
}

#[cfg(windows)]
fn ordinary_batch_path(
    root: &Path,
    relative: &Path,
    directories: &mut HashMap<PathBuf, HashSet<OsString>>,
) -> bool {
    let mut parent = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return false;
        };
        let names = directories.entry(parent.clone()).or_insert_with(|| {
            let mut names = HashSet::new();
            if let Ok(entries) = fs::read_dir(&parent) {
                for entry in entries.flatten() {
                    // DirEntry metadata retains reparse attributes on Windows without
                    // reopening each file. Exact names also preserve canonical casing.
                    if entry.metadata().is_ok_and(|metadata| {
                        (metadata.is_file() || metadata.is_dir())
                            && metadata.file_attributes() & 0x400 == 0
                    }) {
                        names.insert(entry.file_name());
                    }
                }
            }
            names
        });
        if !names.contains(name) {
            return false;
        }
        parent.push(name);
    }
    true
}

fn add_gitignore_files(
    policy: &IndexPathPolicy,
    root: &Path,
    dir: &Path,
    scopes: &mut Vec<ScopedGitignore>,
) {
    let mut files = Vec::new();
    let ignore_file = dir.join(".gitignore");
    if ignore_file.is_file() {
        files.push(ignore_file);
    }
    if dir == root {
        if let Some(info_exclude) = git_info_exclude_path(root) {
            if info_exclude.is_file() {
                files.push(info_exclude);
            }
        }
    }
    if let Some(scope) = build_scoped_gitignore(dir, files) {
        scopes.push(scope);
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if policy.is_builtin_denied_path(root, &path) {
            continue;
        }
        add_gitignore_files(policy, root, &path, scopes);
    }
}

fn build_scoped_gitignore(base: &Path, files: Vec<PathBuf>) -> Option<ScopedGitignore> {
    if files.is_empty() {
        return None;
    }
    let base = dunce::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let mut builder = GitignoreBuilder::new(&base);
    for file in files {
        let _ = builder.add(file);
    }
    builder
        .build()
        .ok()
        .map(|gitignore| ScopedGitignore { base, gitignore })
}

fn git_info_exclude_path(root: &Path) -> Option<PathBuf> {
    let output = crate::process::hidden_command("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--git-path")
        .arg("info/exclude")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if path.is_empty() {
        return None;
    }
    let path = PathBuf::from(path);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn relative_path(root: &Path, path: &Path) -> PathBuf {
    let canonical_root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    relative_path_with_canonical_root(root, &canonical_root, path)
}

fn relative_path_with_canonical_root(root: &Path, canonical_root: &Path, path: &Path) -> PathBuf {
    #[cfg(all(test, windows))]
    PATH_CANONICALIZE_CALLS.with(|calls| calls.set(calls.get() + 1));
    let canonical_path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical_path
        .strip_prefix(canonical_root)
        .unwrap_or(path.strip_prefix(root).unwrap_or(path))
        .to_path_buf()
}

fn normalize_rel_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn builtin_path_batch_skips_canonicalizing_ordinary_denied_files() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join(".gwt/work/events");
        fs::create_dir_all(&parent).unwrap();
        let paths = [parent.join("first.jsonl"), parent.join("second.jsonl")].to_vec();
        for path in &paths {
            fs::write(path, "event").unwrap();
        }
        let before = PATH_CANONICALIZE_CALLS.with(|calls| calls.get());
        assert!(default_index_path_policy()
            .filter_builtin_denied_paths(dir.path(), paths)
            .is_empty());
        let calls = PATH_CANONICALIZE_CALLS.with(|calls| calls.get() - before);
        assert_eq!(calls, 0, "ordinary denied files must not be canonicalized");
    }
}
