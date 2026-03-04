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
/// - Prefer `npx` when available — it reads registry settings from
///   `~/.npmrc` (typically the public npm registry), whereas `bunx`
///   reads from `~/.bunfig.toml` which may point to a private registry
///   that does not mirror public packages and can hang indefinitely.
///   `npx` also avoids lockfile conflicts with project-level
///   `packageManager` fields.
/// - Fall back to `bunx` when `npx` is not installed but a global `bunx`
///   is available.
pub fn choose_fallback_runner(
    bunx_path: Option<&Path>,
    npx_available: bool,
) -> Option<FallbackRunner> {
    if npx_available {
        return Some(FallbackRunner::Npx);
    }
    match bunx_path {
        Some(path) if !is_node_modules_bin(path) => Some(FallbackRunner::Bunx),
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

fn env_get<'a>(env: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    if cfg!(windows) {
        // Windows env keys are case-insensitive. `std::env::vars()` preserves casing, so
        // look up case-insensitively to avoid missing PATH/Path variations.
        env.iter()
            .find_map(|(k, v)| k.eq_ignore_ascii_case(key).then_some(v.as_str()))
    } else {
        env.get(key).map(|s| s.as_str())
    }
}

fn strip_wrapping_quotes(value: &str) -> Option<&str> {
    if value.len() < 2 {
        return None;
    }

    let bytes = value.as_bytes();
    let first = bytes[0];
    let last = bytes[value.len() - 1];
    let wrapped = (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'');
    if wrapped {
        Some(value[1..value.len() - 1].trim())
    } else {
        None
    }
}

fn escaped_wrapping_quote_prefix(value: &str) -> Option<(usize, u8)> {
    let bytes = value.as_bytes();
    let mut slash_count = 0;
    while slash_count < bytes.len() && bytes[slash_count] == b'\\' {
        slash_count += 1;
    }

    if slash_count == 0 || slash_count >= bytes.len() {
        return None;
    }

    let quote = bytes[slash_count];
    if quote != b'"' && quote != b'\'' {
        return None;
    }

    Some((slash_count, quote))
}

fn escaped_wrapping_quote_suffix(value: &str, quote: u8) -> Option<usize> {
    let bytes = value.as_bytes();
    if bytes.last().copied()? != quote || bytes.len() < 2 {
        return None;
    }

    let mut slash_count = 0usize;
    let mut idx = bytes.len() - 1;
    while idx > 0 && bytes[idx - 1] == b'\\' {
        slash_count += 1;
        idx -= 1;
    }

    if slash_count == 0 {
        None
    } else {
        Some(slash_count)
    }
}

fn strip_wrapping_escaped_quotes(value: &str) -> Option<&str> {
    if value.len() < 4 {
        return None;
    }

    let (prefix_slashes, quote) = escaped_wrapping_quote_prefix(value)?;
    let suffix_slashes = escaped_wrapping_quote_suffix(value, quote)?;
    if prefix_slashes != suffix_slashes {
        return None;
    }

    let prefix_len = prefix_slashes + 1;
    let suffix_len = suffix_slashes + 1;
    if value.len() < prefix_len + suffix_len {
        return None;
    }

    Some(value[prefix_len..value.len() - suffix_len].trim())
}

fn strip_wrapping_quotes_recursive(value: &str) -> String {
    let mut current = value.trim();
    loop {
        if let Some(next) = strip_wrapping_quotes(current) {
            current = next;
            continue;
        }
        if let Some(next) = strip_wrapping_escaped_quotes(current) {
            current = next;
            continue;
        }
        break;
    }
    current.to_string()
}

fn count_preceding_backslashes(bytes: &[u8], quote_index: usize) -> usize {
    let mut slash_count = 0usize;
    let mut idx = quote_index;
    while idx > 0 && bytes[idx - 1] == b'\\' {
        slash_count += 1;
        idx -= 1;
    }
    slash_count
}

fn is_escaped_quote(bytes: &[u8], quote_index: usize) -> bool {
    count_preceding_backslashes(bytes, quote_index) % 2 == 1
}

fn escaped_wrapped_token_len(value: &str) -> Option<usize> {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    let (prefix_slashes, quote) = escaped_wrapping_quote_prefix(trimmed)?;
    let mut idx = prefix_slashes + 1;

    while idx < bytes.len() {
        if bytes[idx] == quote {
            let slash_count = count_preceding_backslashes(bytes, idx);
            if slash_count == prefix_slashes {
                let next = idx + 1;
                if next == bytes.len() || bytes[next].is_ascii_whitespace() {
                    return Some(next);
                }
            }
        }
        idx += 1;
    }

    None
}

fn plain_wrapped_token_len(value: &str) -> Option<usize> {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    let quote = *bytes.first()?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }

    let mut idx = 1usize;
    while idx < bytes.len() {
        if bytes[idx] == quote && !is_escaped_quote(bytes, idx) {
            let next = idx + 1;
            if next == bytes.len() || bytes[next].is_ascii_whitespace() {
                return Some(next);
            }
        }
        idx += 1;
    }

    None
}

fn leading_windows_command_token(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return trimmed;
    }

    let bytes = trimmed.as_bytes();
    let starts_with_plain_quote = bytes.first().is_some_and(|b| *b == b'\'' || *b == b'"');
    let starts_with_escaped_quote = escaped_wrapping_quote_prefix(trimmed).is_some();

    // Unquoted command strings can legitimately contain spaces in Windows paths.
    // Only tokenize when the input starts with an explicit quote wrapper.
    if !starts_with_plain_quote && !starts_with_escaped_quote {
        return trimmed;
    }

    if starts_with_plain_quote {
        if let Some(len) = plain_wrapped_token_len(trimmed) {
            return trimmed[..len].trim_end();
        }
    }

    if let Some(len) = escaped_wrapped_token_len(trimmed) {
        return trimmed[..len].trim_end();
    }

    let mut in_single = false;
    let mut in_double = false;

    for (idx, byte) in bytes.iter().enumerate() {
        match *byte {
            b'\'' if !in_double && !is_escaped_quote(bytes, idx) => {
                in_single = !in_single;
            }
            b'"' if !in_single && !is_escaped_quote(bytes, idx) => {
                in_double = !in_double;
            }
            b if b.is_ascii_whitespace() && !in_single && !in_double => {
                return trimmed[..idx].trim_end();
            }
            _ => {}
        }
    }

    trimmed
}

fn strip_windows_invocation_prefix(value: &str) -> &str {
    let mut current = value.trim();

    loop {
        let next = if let Some(rest) = current.strip_prefix('&') {
            Some(rest.trim_start())
        } else if current
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("call"))
            && current
                .as_bytes()
                .get(4)
                .is_some_and(|b| b.is_ascii_whitespace())
        {
            current.get(4..).map(|rest| rest.trim_start())
        } else {
            None
        };

        match next {
            Some(rest) if !rest.is_empty() => current = rest,
            _ => break,
        }
    }

    current
}

/// Normalize a potentially wrapped/escaped Windows command path.
///
/// This helper is intentionally defensive because command strings can arrive with
/// quoting artifacts (`'\"...\"'`, `\\\"...\\\"`) from environment snapshots.
/// It extracts only the executable token and removes nested wrapping quotes.
pub fn normalize_windows_command_path(command: &str) -> String {
    let candidate = strip_windows_invocation_prefix(command);
    let token = leading_windows_command_token(candidate);
    if token.is_empty() {
        return String::new();
    }
    strip_wrapping_quotes_recursive(token)
}

fn normalize_resolved_path(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    let normalized = normalize_windows_command_path(raw.as_ref());
    if normalized.is_empty() {
        path.to_path_buf()
    } else {
        PathBuf::from(normalized)
    }
}

fn resolve_command_path_with_env(command: &str, env: &HashMap<String, String>) -> Option<PathBuf> {
    let normalized = normalize_windows_command_path(command);
    let cmd_owned = if normalized.is_empty() {
        command.trim().to_string()
    } else {
        normalized
    };
    let cmd = cmd_owned.trim();
    if cmd.is_empty() {
        return None;
    }

    // 1) Search PATH explicitly using which_in so tests can control the environment safely.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let paths = env_get(env, "PATH").filter(|s| !s.trim().is_empty());
    let mut weak_path: Option<PathBuf> = None;
    if let Ok(iter) = which::which_in_all(cmd, paths, &cwd) {
        for found in iter {
            let normalized_found = normalize_resolved_path(&found);
            // PATH may contain project-local shims (e.g. node_modules/.bin) when running under
            // temporary executors (bunx/npx). Prefer global installs when available.
            if is_node_modules_bin(&normalized_found) {
                if weak_path.is_none() {
                    weak_path = Some(normalized_found);
                }
                continue;
            }

            return Some(normalized_found);
        }
    }

    // 2) Search common install locations (best-effort).
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Bun install env var (typically "~/.bun").
    if let Some(root) = env_get(env, "BUN_INSTALL").map(PathBuf::from) {
        candidates.extend(command_candidates_in_dir(&root.join("bin"), cmd));
    }

    let home = env_get(env, "HOME").map(PathBuf::from);

    if cfg!(windows) {
        let user_profile = env_get(env, "USERPROFILE").map(PathBuf::from);
        // Bun default: %USERPROFILE%\.bun\bin (fallback to HOME if USERPROFILE is not set).
        if let Some(h) = user_profile.as_ref().or(home.as_ref()) {
            candidates.extend(command_candidates_in_dir(&h.join(".bun").join("bin"), cmd));
        }
        // Alternative Bun location on Windows: %LOCALAPPDATA%\bun\bin
        if let Some(local) = env_get(env, "LOCALAPPDATA").map(PathBuf::from) {
            candidates.extend(command_candidates_in_dir(
                &local.join("bun").join("bin"),
                cmd,
            ));
        }
    } else {
        // Bun default: ~/.bun/bin
        if let Some(h) = home.as_ref() {
            candidates.extend(command_candidates_in_dir(&h.join(".bun").join("bin"), cmd));
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
    fn normalize_launch_command(command: String) -> String {
        let normalized = normalize_windows_command_path(&command);
        if normalized.is_empty() {
            command
        } else {
            normalized
        }
    }

    match runner {
        FallbackRunner::Bunx => (
            normalize_launch_command(
                bunx_path
                    .unwrap_or_else(|| Path::new("bunx"))
                    .to_string_lossy()
                    .to_string(),
            ),
            vec![package.to_string()],
        ),
        FallbackRunner::Npx => (
            normalize_launch_command(
                npx_path
                    .unwrap_or_else(|| Path::new("npx"))
                    .to_string_lossy()
                    .to_string(),
            ),
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
    fn choose_fallback_runner_prefers_npx_when_both_available() {
        assert_eq!(
            choose_fallback_runner(Some(Path::new("/usr/local/bin/bunx")), true),
            Some(FallbackRunner::Npx)
        );
    }

    #[test]
    fn choose_fallback_runner_uses_bunx_when_npx_unavailable() {
        assert_eq!(
            choose_fallback_runner(Some(Path::new("/usr/local/bin/bunx")), false),
            Some(FallbackRunner::Bunx)
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
    fn build_fallback_launch_npx_normalizes_wrapped_resolved_path_when_provided() {
        let (cmd, args) = build_fallback_launch(
            FallbackRunner::Npx,
            "@openai/codex@latest",
            None,
            Some(Path::new(r#"'\"C:\Program Files\nodejs\npx.cmd\"'"#)),
        );
        assert_eq!(cmd, r#"C:\Program Files\nodejs\npx.cmd"#);
        assert_eq!(
            args,
            vec!["--yes".to_string(), "@openai/codex@latest".to_string()]
        );
    }

    #[test]
    fn normalize_windows_command_path_unwraps_issue_1265_pattern() {
        assert_eq!(
            normalize_windows_command_path(r#"'\"C:\Program Files\nodejs\npx.cmd\"'"#),
            r#"C:\Program Files\nodejs\npx.cmd"#
        );
    }

    #[test]
    fn normalize_windows_command_path_ignores_trailing_arguments() {
        assert_eq!(
            normalize_windows_command_path(
                r#"'\"C:\Program Files\nodejs\npx.cmd\"' --yes @openai/codex@latest"#
            ),
            r#"C:\Program Files\nodejs\npx.cmd"#
        );
    }

    #[test]
    fn normalize_windows_command_path_handles_single_quoted_path_with_apostrophe() {
        assert_eq!(
            normalize_windows_command_path(
                "'C:\\Tools\\O'Neil Folder\\npx.cmd' --yes @openai/codex@latest"
            ),
            "C:\\Tools\\O'Neil Folder\\npx.cmd"
        );
    }

    #[test]
    fn normalize_windows_command_path_handles_double_escaped_prefix_without_outer_quotes() {
        assert_eq!(
            normalize_windows_command_path(
                r#"\\\"C:\Program Files\nodejs\npx.cmd\\\" --yes @openai/codex@latest"#
            ),
            r#"C:\Program Files\nodejs\npx.cmd"#
        );
    }

    #[test]
    fn normalize_windows_command_path_keeps_mismatched_escape_sequences() {
        assert_eq!(
            normalize_windows_command_path(r#"\\\"C:\Tools\npx.cmd\""#),
            r#"\\\"C:\Tools\npx.cmd\""#
        );
    }

    #[test]
    fn normalize_windows_command_path_handles_leading_powershell_call_operator() {
        assert_eq!(
            normalize_windows_command_path(
                r#"& '\"C:\Program Files\nodejs\npx.cmd\"' --yes @openai/codex@latest"#
            ),
            r#"C:\Program Files\nodejs\npx.cmd"#
        );
    }

    #[test]
    fn normalize_windows_command_path_handles_leading_call_keyword() {
        assert_eq!(
            normalize_windows_command_path(
                r#"call '\"C:\Program Files\nodejs\npx.cmd\"' --yes @openai/codex@latest"#
            ),
            r#"C:\Program Files\nodejs\npx.cmd"#
        );
    }

    #[test]
    fn normalize_windows_command_path_handles_multibyte_prefix_without_panic() {
        assert_eq!(
            normalize_windows_command_path("あ¢call npx.cmd --yes @openai/codex@latest"),
            "あ¢call npx.cmd --yes @openai/codex@latest"
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
    fn resolve_command_path_normalizes_wrapped_command_token_before_lookup() {
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create bin dir");

        let command = "gwt-resolve-wrapped-test";
        let command_path = command_path_in_dir(&bin_dir, command);
        write_stub_command(&command_path);

        let path = std::env::join_paths([&bin_dir])
            .expect("join PATH")
            .to_string_lossy()
            .to_string();

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), path);
        env.insert("HOME".to_string(), dir.path().to_string_lossy().to_string());

        assert_eq!(
            resolve_command_path_with_env(&format!(r#"'\"{command}\"'"#), &env),
            Some(command_path)
        );
    }

    #[cfg(windows)]
    #[test]
    fn normalize_resolved_path_unwraps_issue_1265_pattern() {
        let raw = Path::new(r#"'\"C:\Program Files\nodejs\npx.cmd\"'"#);
        assert_eq!(
            normalize_resolved_path(raw),
            PathBuf::from(r#"C:\Program Files\nodejs\npx.cmd"#)
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

    #[cfg(windows)]
    #[test]
    fn resolve_command_path_reads_path_case_insensitively_on_windows() {
        let dir = tempdir().expect("tempdir");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("create bin dir");

        let command = "gwt-path-case-test";
        let cmd = bin.join(format!("{command}.exe"));
        write_stub_command(&cmd);

        let path = std::env::join_paths([&bin])
            .expect("join PATH")
            .to_string_lossy()
            .to_string();

        let mut env = HashMap::new();
        env.insert("Path".to_string(), path);

        assert_eq!(resolve_command_path_with_env(command, &env), Some(cmd));
    }
}
