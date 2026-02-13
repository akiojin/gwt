use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
const GH_FALLBACK_PATHS: &[&str] = &[
    r"C:\Program Files\GitHub CLI\gh.exe",
    r"C:\Program Files (x86)\GitHub CLI\gh.exe",
];

#[cfg(not(target_os = "windows"))]
const GH_FALLBACK_PATHS: &[&str] = &[
    "/opt/homebrew/bin/gh",
    "/usr/local/bin/gh",
    "/opt/local/bin/gh",
    "/usr/bin/gh",
];

fn fallback_gh_path() -> Option<PathBuf> {
    GH_FALLBACK_PATHS
        .iter()
        .map(Path::new)
        .find(|path| path.exists())
        .map(PathBuf::from)
}

pub fn resolve_gh_path() -> Option<PathBuf> {
    which::which("gh").ok().or_else(fallback_gh_path)
}

pub fn gh_command() -> std::process::Command {
    match resolve_gh_path() {
        Some(path) => {
            let program = path.to_string_lossy().into_owned();
            crate::process::command(&program)
        }
        None => crate::process::command("gh"),
    }
}

pub fn is_gh_available() -> bool {
    gh_command()
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_gh_available_returns_bool() {
        let _result: bool = is_gh_available();
    }

    #[test]
    fn resolve_gh_path_accepts_missing_or_present() {
        let _result = resolve_gh_path();
    }
}
