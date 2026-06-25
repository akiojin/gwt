use std::path::{Component, Path, PathBuf};

use gwt_core::index::path_policy::{
    build_project_ignore_matcher, default_index_path_policy, ProjectIgnoreMatcher,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PathFilterError {
    Escape,
    NotFound,
}

pub(crate) struct ResolvedPath {
    pub canonical_path: PathBuf,
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
    Ok(ResolvedPath { canonical_path })
}

pub(crate) fn build_gitignore(root: &Path) -> ProjectIgnoreMatcher {
    build_project_ignore_matcher(root)
}

pub(crate) fn is_path_ignored(gitignore: &ProjectIgnoreMatcher, root: &Path, path: &Path) -> bool {
    !default_index_path_policy().is_indexable_path(gitignore, root, path)
}

pub(crate) fn is_relative_denied(canonical_root: &Path, relative: &Path) -> bool {
    let full = canonical_root.join(relative);
    let gitignore = build_gitignore(canonical_root);
    is_path_ignored(&gitignore, canonical_root, &full)
}
