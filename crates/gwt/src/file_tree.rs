use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io;
use std::path::{Component, Path, PathBuf};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTreeEntryKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTreeEntry {
    pub name: String,
    pub path: String,
    pub kind: FileTreeEntryKind,
}

pub fn list_directory_entries(
    root: &Path,
    relative_dir: Option<&Path>,
) -> io::Result<Vec<FileTreeEntry>> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("repository root is not a directory: {}", root.display()),
        ));
    }

    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let relative_dir = normalize_relative_dir(relative_dir.unwrap_or_else(|| Path::new("")))?;
    let target = resolve_directory(&root, &relative_dir)?;
    let gitignore = build_gitignore(&root);

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&target)? {
        let entry = entry?;
        let path = entry.path();
        if is_ignored(&gitignore, &root, &path) {
            continue;
        }

        let file_type = entry.file_type()?;
        let kind = if file_type.is_dir() {
            FileTreeEntryKind::Directory
        } else {
            FileTreeEntryKind::File
        };
        entries.push(FileTreeEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: relative_path_string(path.strip_prefix(&root).unwrap_or(&path)),
            kind,
        });
    }

    entries.sort_by(|left, right| match (&left.kind, &right.kind) {
        (FileTreeEntryKind::Directory, FileTreeEntryKind::File) => Ordering::Less,
        (FileTreeEntryKind::File, FileTreeEntryKind::Directory) => Ordering::Greater,
        _ => left
            .name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.name.cmp(&right.name)),
    });

    Ok(entries)
}

fn normalize_relative_dir(path: &Path) -> io::Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("path escapes repository root: {}", path.display()),
                ));
            }
        }
    }
    Ok(normalized)
}

fn resolve_directory(root: &Path, relative_dir: &Path) -> io::Result<PathBuf> {
    let target = root.join(relative_dir);
    let target = dunce::canonicalize(&target)?;
    if !target.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("path escapes repository root: {}", relative_dir.display()),
        ));
    }
    if !target.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("directory does not exist: {}", relative_dir.display()),
        ));
    }
    Ok(target)
}

fn relative_path_string(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn build_gitignore(worktree: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(worktree);
    let gitignore_path = worktree.join(".gitignore");
    if gitignore_path.is_file() {
        let _ = builder.add(&gitignore_path);
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

fn is_builtin_skip(worktree: &Path, path: &Path) -> bool {
    let rel = path.strip_prefix(worktree).unwrap_or(path);
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

fn is_ignored(gitignore: &Gitignore, worktree: &Path, path: &Path) -> bool {
    if is_builtin_skip(worktree, path) {
        return true;
    }
    let rel = path.strip_prefix(worktree).unwrap_or(path);
    let is_dir = path.is_dir();
    matches!(
        gitignore.matched_path_or_any_parents(rel, is_dir),
        ignore::Match::Ignore(_)
    )
}
