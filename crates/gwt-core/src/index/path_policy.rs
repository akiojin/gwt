use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use ignore::{
    gitignore::{Gitignore, GitignoreBuilder},
    Match,
};
use serde::Deserialize;

const INDEX_PATH_POLICY_SOURCE: &str = include_str!("../../runtime/index_path_policy.json");
static INDEX_PATH_POLICY: OnceLock<IndexPathPolicy> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
pub struct IndexPathPolicy {
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
    let output = Command::new("git")
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
    let canonical_path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical_path
        .strip_prefix(&canonical_root)
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
