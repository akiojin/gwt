//! Utilities for launching npm-based tools (bunx/npx) in environments where PATH may differ
//! from an interactive shell (e.g., GUI apps).

use std::path::{Path, PathBuf};

/// Fallback runner for executing npm packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackRunner {
    Bunx,
    Npx,
}

#[derive(Debug, Clone, Default)]
struct EnvSnapshot {
    path: Option<String>,
    home: Option<PathBuf>,
    user_profile: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    bun_install: Option<PathBuf>,
}

impl EnvSnapshot {
    fn capture() -> Self {
        Self {
            path: std::env::var_os("PATH").map(|v| v.to_string_lossy().to_string()),
            home: std::env::var_os("HOME").map(PathBuf::from),
            user_profile: std::env::var_os("USERPROFILE").map(PathBuf::from),
            local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            bun_install: std::env::var_os("BUN_INSTALL").map(PathBuf::from),
        }
    }
}

/// Returns true if the given path appears to be a project-local `node_modules/.bin` shim.
pub fn is_node_modules_bin(path: &Path) -> bool {
    // Cross-platform substring match is sufficient here.
    let p = path.to_string_lossy();
    p.contains("node_modules/.bin") || p.contains("node_modules\\.bin")
}

/// Chooses which runner to use when launching npm packages.
///
/// - Prefer `bunx` when it exists and is not a project-local shim.
/// - Otherwise, use `npx` when available.
pub fn choose_fallback_runner(
    bunx_path: Option<&Path>,
    npx_available: bool,
) -> Option<FallbackRunner> {
    match bunx_path {
        Some(path) if !is_node_modules_bin(path) => Some(FallbackRunner::Bunx),
        _ if npx_available => Some(FallbackRunner::Npx),
        _ => None,
    }
}

fn command_candidates_in_dir(dir: &Path, command: &str) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![
            dir.join(format!("{command}.exe")),
            dir.join(format!("{command}.cmd")),
            dir.join(format!("{command}.bat")),
            dir.join(command),
        ]
    } else {
        vec![dir.join(command)]
    }
}

fn resolve_command_path_with_env(command: &str, env: &EnvSnapshot) -> Option<PathBuf> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return None;
    }

    // 1) Search PATH explicitly using which_in so tests can control the environment safely.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let paths = env.path.as_deref().filter(|s| !s.trim().is_empty());
    if let Ok(found) = which::which_in(cmd, paths, &cwd) {
        return Some(found);
    }

    // 2) Search common install locations (best-effort).
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Bun install env var (typically "~/.bun").
    if let Some(root) = env.bun_install.as_ref() {
        candidates.extend(command_candidates_in_dir(&root.join("bin"), cmd));
    }

    if cfg!(windows) {
        // Bun default: %USERPROFILE%\.bun\bin (fallback to HOME if USERPROFILE is not set).
        if let Some(home) = env.user_profile.as_ref().or(env.home.as_ref()) {
            candidates.extend(command_candidates_in_dir(
                &home.join(".bun").join("bin"),
                cmd,
            ));
        }
        // Alternative Bun location on Windows: %LOCALAPPDATA%\bun\bin
        if let Some(local) = env.local_app_data.as_ref() {
            candidates.extend(command_candidates_in_dir(
                &local.join("bun").join("bin"),
                cmd,
            ));
        }
    } else {
        // Bun default: ~/.bun/bin
        if let Some(home) = env.home.as_ref() {
            candidates.extend(command_candidates_in_dir(
                &home.join(".bun").join("bin"),
                cmd,
            ));
        }

        // Common system paths (macOS/Linux).
        for base in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            candidates.extend(command_candidates_in_dir(Path::new(base), cmd));
        }
    }

    candidates.into_iter().find(|p| p.is_file())
}

/// Resolve a command to an absolute path when possible.
///
/// This is a best-effort helper intended to make GUI-launched processes more reliable when their
/// `PATH` differs from an interactive shell.
pub fn resolve_command_path(command: &str) -> Option<PathBuf> {
    resolve_command_path_with_env(command, &EnvSnapshot::capture())
}

/// Build the executable + base args for a bunx/npx launch.
///
/// Returns:
/// - bunx: `[bunx] [@pkg@version]`
/// - npx:  `[npx]  [--yes] [@pkg@version]`
pub fn build_fallback_launch(
    runner: FallbackRunner,
    package: &str,
    bunx_path: Option<&Path>,
    npx_path: Option<&Path>,
) -> (String, Vec<String>) {
    match runner {
        FallbackRunner::Bunx => (
            bunx_path
                .unwrap_or_else(|| Path::new("bunx"))
                .to_string_lossy()
                .to_string(),
            vec![package.to_string()],
        ),
        FallbackRunner::Npx => (
            npx_path
                .unwrap_or_else(|| Path::new("npx"))
                .to_string_lossy()
                .to_string(),
            vec!["--yes".to_string(), package.to_string()],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn is_node_modules_bin_matches_common_paths() {
        assert!(is_node_modules_bin(Path::new(
            "/repo/node_modules/.bin/bunx"
        )));
        assert!(is_node_modules_bin(Path::new(
            "C:\\repo\\node_modules\\.bin\\bunx"
        )));
        assert!(!is_node_modules_bin(Path::new("/usr/local/bin/bunx")));
    }

    #[test]
    fn choose_fallback_runner_prefers_bunx_when_not_local() {
        assert_eq!(
            choose_fallback_runner(Some(Path::new("/usr/local/bin/bunx")), true),
            Some(FallbackRunner::Bunx)
        );
    }

    #[test]
    fn choose_fallback_runner_uses_npx_when_bunx_is_local_node_modules() {
        assert_eq!(
            choose_fallback_runner(Some(Path::new("/repo/node_modules/.bin/bunx")), true),
            Some(FallbackRunner::Npx)
        );
    }

    #[test]
    fn choose_fallback_runner_none_when_only_local_bunx_and_no_npx() {
        assert_eq!(
            choose_fallback_runner(Some(Path::new("/repo/node_modules/.bin/bunx")), false),
            None
        );
    }

    #[test]
    fn choose_fallback_runner_uses_npx_when_bunx_is_missing() {
        assert_eq!(
            choose_fallback_runner(None, true),
            Some(FallbackRunner::Npx)
        );
    }

    #[test]
    fn build_fallback_launch_bunx_uses_resolved_path_when_provided() {
        let (cmd, args) = build_fallback_launch(
            FallbackRunner::Bunx,
            "@openai/codex@latest",
            Some(Path::new("/usr/local/bin/bunx")),
            None,
        );
        assert_eq!(cmd, "/usr/local/bin/bunx");
        assert_eq!(args, vec!["@openai/codex@latest".to_string()]);
    }

    #[test]
    fn build_fallback_launch_npx_uses_resolved_path_and_yes_flag_when_provided() {
        let (cmd, args) = build_fallback_launch(
            FallbackRunner::Npx,
            "@openai/codex@latest",
            None,
            Some(Path::new("/usr/bin/npx")),
        );
        assert_eq!(cmd, "/usr/bin/npx");
        assert_eq!(
            args,
            vec!["--yes".to_string(), "@openai/codex@latest".to_string()]
        );
    }

    #[test]
    fn resolve_command_path_finds_bunx_in_home_bun_bin_when_path_is_unset() {
        let dir = tempdir().expect("tempdir");
        let bun_bin = dir.path().join(".bun").join("bin");
        std::fs::create_dir_all(&bun_bin).expect("create bun bin dir");

        let bunx = if cfg!(windows) {
            bun_bin.join("bunx.exe")
        } else {
            bun_bin.join("bunx")
        };
        std::fs::write(&bunx, "").expect("write bunx stub");

        let env = EnvSnapshot {
            path: None,
            home: Some(dir.path().to_path_buf()),
            user_profile: None,
            local_app_data: None,
            bun_install: None,
        };

        assert_eq!(resolve_command_path_with_env("bunx", &env), Some(bunx));
    }
}
