//! Utilities for launching npm-based tools (bunx/npx) in environments where PATH may differ
//! from an interactive shell (e.g., GUI apps).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Fallback runner for executing npm packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackRunner {
    Bunx,
    Npx,
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

fn resolve_command_path_with_env(command: &str, env: &HashMap<String, String>) -> Option<PathBuf> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return None;
    }

    // 1) Search PATH explicitly using which_in so tests can control the environment safely.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let paths = env.get("PATH").map(|s| s.as_str()).filter(|s| !s.trim().is_empty());
    let mut weak_path: Option<PathBuf> = None;
    if let Ok(iter) = which::which_in_all(cmd, paths, &cwd) {
        for found in iter {
            // PATH may contain project-local shims (e.g. node_modules/.bin) when running under
            // temporary executors (bunx/npx). Prefer global installs when available.
            if is_node_modules_bin(&found) {
                if weak_path.is_none() {
                    weak_path = Some(found);
                }
                continue;
            }

            return Some(found);
        }
    }

    // 2) Search common install locations (best-effort).
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Bun install env var (typically "~/.bun").
    if let Some(root) = env.get("BUN_INSTALL").map(PathBuf::from) {
        candidates.extend(command_candidates_in_dir(&root.join("bin"), cmd));
    }

    let home = env.get("HOME").map(PathBuf::from);

    if cfg!(windows) {
        let user_profile = env.get("USERPROFILE").map(PathBuf::from);
        // Bun default: %USERPROFILE%\.bun\bin (fallback to HOME if USERPROFILE is not set).
        if let Some(h) = user_profile.as_ref().or(home.as_ref()) {
            candidates.extend(command_candidates_in_dir(
                &h.join(".bun").join("bin"),
                cmd,
            ));
        }
        // Alternative Bun location on Windows: %LOCALAPPDATA%\bun\bin
        if let Some(local) = env.get("LOCALAPPDATA").map(PathBuf::from) {
            candidates.extend(command_candidates_in_dir(
                &local.join("bun").join("bin"),
                cmd,
            ));
        }
    } else {
        // Bun default: ~/.bun/bin
        if let Some(h) = home.as_ref() {
            candidates.extend(command_candidates_in_dir(
                &h.join(".bun").join("bin"),
                cmd,
            ));
        }

        // Common system paths (macOS/Linux).
        for base in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            candidates.extend(command_candidates_in_dir(Path::new(base), cmd));
        }
    }

    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }

    weak_path
}

/// Resolve a command to an absolute path when possible.
///
/// This is a best-effort helper intended to make GUI-launched processes more reliable when their
/// `PATH` differs from an interactive shell.
pub fn resolve_command_path(command: &str) -> Option<PathBuf> {
    let env: HashMap<String, String> = std::env::vars().collect();
    resolve_command_path_with_env(command, &env)
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

    fn command_path_in_dir(dir: &Path, command: &str) -> PathBuf {
        if cfg!(windows) {
            dir.join(format!("{command}.exe"))
        } else {
            dir.join(command)
        }
    }

    fn write_stub_command(path: &Path) {
        std::fs::write(path, "").expect("write command stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perm = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(path, perm).expect("chmod command stub");
        }
    }

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

        let mut env = HashMap::new();
        env.insert("HOME".to_string(), dir.path().to_string_lossy().to_string());

        assert_eq!(resolve_command_path_with_env("bunx", &env), Some(bunx));
    }

    #[test]
    fn resolve_command_path_prefers_global_when_path_points_to_node_modules_bin() {
        let dir = tempdir().expect("tempdir");
        let local_bin = dir.path().join("project").join("node_modules").join(".bin");
        std::fs::create_dir_all(&local_bin).expect("create node_modules bin dir");

        let global_bin = dir.path().join(".bun").join("bin");
        std::fs::create_dir_all(&global_bin).expect("create global bun bin dir");

        let command = "gwt-resolve-test";
        let local_cmd = command_path_in_dir(&local_bin, command);
        let global_cmd = command_path_in_dir(&global_bin, command);
        write_stub_command(&local_cmd);
        write_stub_command(&global_cmd);

        let path = std::env::join_paths([&local_bin])
            .expect("join PATH")
            .to_string_lossy()
            .to_string();

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), path);
        env.insert("HOME".to_string(), dir.path().to_string_lossy().to_string());

        assert_eq!(
            resolve_command_path_with_env(command, &env),
            Some(global_cmd)
        );
    }

    #[test]
    fn resolve_command_path_uses_node_modules_bin_when_no_global_candidate_exists() {
        let dir = tempdir().expect("tempdir");
        let local_bin = dir.path().join("project").join("node_modules").join(".bin");
        std::fs::create_dir_all(&local_bin).expect("create node_modules bin dir");

        let command = "gwt-resolve-test-only-local";
        let local_cmd = command_path_in_dir(&local_bin, command);
        write_stub_command(&local_cmd);

        let path = std::env::join_paths([&local_bin])
            .expect("join PATH")
            .to_string_lossy()
            .to_string();

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), path);
        env.insert("HOME".to_string(), dir.path().to_string_lossy().to_string());

        assert_eq!(
            resolve_command_path_with_env(command, &env),
            Some(local_cmd)
        );
    }

    #[test]
    fn resolve_command_path_picks_non_shim_from_later_in_path() {
        let dir = tempdir().expect("tempdir");
        let local_bin = dir.path().join("project").join("node_modules").join(".bin");
        std::fs::create_dir_all(&local_bin).expect("create node_modules bin dir");

        let custom_bin = dir.path().join("custom").join("bin");
        std::fs::create_dir_all(&custom_bin).expect("create custom bin dir");

        let command = "gwt-resolve-test-path-order";
        let local_cmd = command_path_in_dir(&local_bin, command);
        let custom_cmd = command_path_in_dir(&custom_bin, command);
        write_stub_command(&local_cmd);
        write_stub_command(&custom_cmd);

        let path = std::env::join_paths([&local_bin, &custom_bin])
            .expect("join PATH")
            .to_string_lossy()
            .to_string();

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), path);
        env.insert("HOME".to_string(), dir.path().to_string_lossy().to_string());

        assert_eq!(
            resolve_command_path_with_env(command, &env),
            Some(custom_cmd)
        );
    }
}
