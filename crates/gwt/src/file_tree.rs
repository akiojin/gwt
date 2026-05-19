use std::{
    cmp::Ordering,
    io,
    path::{Component, Path},
};

use serde::{Deserialize, Serialize};

use crate::path_filter::{
    self, build_gitignore, canonical_root, is_path_ignored, normalize_relative, PathFilterError,
};

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

    let canonical = canonical_root(root);
    let relative_dir = relative_dir.unwrap_or_else(|| Path::new(""));
    let relative = match normalize_relative(relative_dir) {
        Ok(rel) => rel,
        Err(PathFilterError::Escape) | Err(PathFilterError::NotFound) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("path escapes repository root: {}", relative_dir.display()),
            ));
        }
    };
    let target = resolve_directory(&canonical, &relative, relative_dir)?;
    let gitignore = build_gitignore(&canonical);

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&target)? {
        let entry = entry?;
        let path = entry.path();
        if is_path_ignored(&gitignore, &canonical, &path) {
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
            path: relative_path_string(path.strip_prefix(&canonical).unwrap_or(&path)),
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

fn resolve_directory(
    canonical_root: &Path,
    relative: &Path,
    original: &Path,
) -> io::Result<std::path::PathBuf> {
    let resolved =
        path_filter::safe_resolve(canonical_root, relative).map_err(|err| match err {
            PathFilterError::Escape => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("path escapes repository root: {}", original.display()),
            ),
            PathFilterError::NotFound => io::Error::new(
                io::ErrorKind::NotFound,
                format!("directory does not exist: {}", original.display()),
            ),
        })?;
    if !resolved.canonical_path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("directory does not exist: {}", original.display()),
        ));
    }
    Ok(resolved.canonical_path)
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
