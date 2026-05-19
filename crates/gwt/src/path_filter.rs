use std::path::{Component, Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

const BUILTIN_SKIP_PREFIXES: &[&str] = &[
    ".git",
    ".claude",
    ".codex",
    ".gemini",
    ".gwt",
    "tasks",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".nuxt",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PathFilterError {
    Escape,
    NotFound,
}

pub(crate) struct ResolvedPath {
    #[allow(dead_code)]
    pub canonical_root: PathBuf,
    pub canonical_path: PathBuf,
    #[allow(dead_code)]
    pub relative: PathBuf,
}

pub(crate) fn canonical_root(root: &Path) -> PathBuf {
    dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

pub(crate) fn normalize_relative(path: &Path) -> Result<PathBuf, PathFilterError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PathFilterError::Escape);
            }
        }
    }
    Ok(normalized)
}

pub(crate) fn safe_resolve(root: &Path, relative: &Path) -> Result<ResolvedPath, PathFilterError> {
    let canonical_root = canonical_root(root);
    let normalized = normalize_relative(relative)?;
    let joined = canonical_root.join(&normalized);
    let canonical_path = dunce::canonicalize(&joined).map_err(|_| PathFilterError::NotFound)?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(PathFilterError::Escape);
    }
    Ok(ResolvedPath {
        canonical_root,
        canonical_path,
        relative: normalized,
    })
}

pub(crate) fn build_gitignore(root: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    let gitignore_path = root.join(".gitignore");
    if gitignore_path.is_file() {
        let _ = builder.add(&gitignore_path);
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

pub(crate) fn is_builtin_skip(root: &Path, path: &Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let first = rel
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(part) => part.to_str(),
            _ => None,
        });
    match first {
        Some(name) => BUILTIN_SKIP_PREFIXES.contains(&name),
        None => false,
    }
}

pub(crate) fn is_path_ignored(gitignore: &Gitignore, root: &Path, path: &Path) -> bool {
    if is_builtin_skip(root, path) {
        return true;
    }
    let rel = path.strip_prefix(root).unwrap_or(path);
    let is_dir = path.is_dir();
    matches!(
        gitignore.matched_path_or_any_parents(rel, is_dir),
        ignore::Match::Ignore(_)
    )
}

pub(crate) fn is_relative_denied(canonical_root: &Path, relative: &Path) -> bool {
    let full = canonical_root.join(relative);
    let gitignore = build_gitignore(canonical_root);
    is_path_ignored(&gitignore, canonical_root, &full)
}
