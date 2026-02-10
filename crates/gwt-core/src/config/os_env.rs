//! OS environment variable capture from login shell.
//!
//! Captures the full environment from the user's login shell so that
//! PTY sessions can inherit PATH extensions, locale settings, etc.

use std::collections::HashMap;

/// Known shell types for environment capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    Nushell,
    Sh,
}

/// How the environment snapshot was obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvSource {
    /// Successfully captured from a login shell invocation.
    LoginShell,
    /// Used the current process environment (expected on Windows).
    ProcessEnv,
    /// Fell back to `std::env::vars()` due to an error.
    StdEnvFallback { reason: String },
}

/// Result of an environment capture attempt.
#[derive(Debug, Clone)]
pub struct OsEnvResult {
    pub env: HashMap<String, String>,
    pub source: EnvSource,
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse NUL-separated `env -0` output into a map.
///
/// Entries without `=` are silently skipped (handles MOTD / banner noise).
/// Empty keys are also skipped.
pub fn parse_env_null_separated(bytes: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in bytes.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        if let Some(pos) = entry.iter().position(|&b| b == b'=') {
            let key = &entry[..pos];
            let value = &entry[pos + 1..];
            if key.is_empty() {
                continue;
            }
            if let (Ok(k), Ok(v)) = (std::str::from_utf8(key), std::str::from_utf8(value)) {
                map.insert(k.to_owned(), v.to_owned());
            }
        }
        // no '=' → skip
    }
    map
}

/// Parse nushell JSON (`$env | to json`) into a map.
///
/// Non-string values (objects, arrays, numbers, booleans) are serialised
/// back to their JSON representation so no information is lost.
pub fn parse_env_json(json: &str) -> Result<HashMap<String, String>, String> {
    let val: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("JSON parse error: {e}"))?;

    let obj = val
        .as_object()
        .ok_or_else(|| "expected top-level JSON object".to_string())?;

    let mut map = HashMap::new();
    for (k, v) in obj {
        let s = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        map.insert(k.clone(), s);
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Shell detection & command building
// ---------------------------------------------------------------------------

/// Detect shell type from a path (e.g. `/usr/bin/zsh` → `Zsh`).
pub fn detect_shell_type(shell_path: &str) -> ShellType {
    let basename = shell_path.rsplit('/').next().unwrap_or(shell_path);
    match basename {
        "bash" => ShellType::Bash,
        "zsh" => ShellType::Zsh,
        "fish" => ShellType::Fish,
        "nu" => ShellType::Nushell,
        _ => ShellType::Sh,
    }
}

/// Build the command + args to capture the login-shell environment.
///
/// Returns `(program, arguments)`.
pub fn build_env_capture_command(shell_type: ShellType, shell_path: &str) -> (String, Vec<String>) {
    let prog = shell_path.to_owned();
    let args = match shell_type {
        ShellType::Nushell => vec![
            "-l".to_owned(),
            "-c".to_owned(),
            "$env | to json".to_owned(),
        ],
        ShellType::Bash | ShellType::Zsh | ShellType::Fish | ShellType::Sh => {
            vec!["-l".to_owned(), "-c".to_owned(), "env -0".to_owned()]
        }
    };
    (prog, args)
}

// ---------------------------------------------------------------------------
// Async capture
// ---------------------------------------------------------------------------

/// Capture the login shell's environment variables.
///
/// On Unix this spawns a login shell and reads its `env -0` (or nushell
/// JSON) output. On Windows it uses the current process environment. On any
/// Unix error it falls back to `std::env::vars()`.
#[cfg(unix)]
pub async fn capture_login_shell_env() -> OsEnvResult {
    use std::time::Duration;
    use tokio::process::Command;

    let shell_path = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let shell_type = detect_shell_type(&shell_path);
    let (prog, args) = build_env_capture_command(shell_type, &shell_path);

    tracing::info!(shell = %shell_path, ?shell_type, "capturing login shell environment");

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(&prog).args(&args).output(),
    )
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            let env = match shell_type {
                ShellType::Nushell => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    match parse_env_json(&stdout) {
                        Ok(map) => map,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to parse nushell JSON, falling back");
                            return fallback_env(format!("nushell JSON parse error: {e}"));
                        }
                    }
                }
                _ => parse_env_null_separated(&output.stdout),
            };
            tracing::info!(count = env.len(), "login shell env captured");
            OsEnvResult {
                env,
                source: EnvSource::LoginShell,
            }
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = format!("shell exited with {}: {}", output.status, stderr.trim_end());
            tracing::warn!(%reason, "login shell env capture failed");
            fallback_env(reason)
        }
        Ok(Err(e)) => {
            let reason = format!("failed to spawn shell: {e}");
            tracing::warn!(%reason, "login shell env capture failed");
            fallback_env(reason)
        }
        Err(_) => {
            let reason = "login shell timed out (5s)".to_owned();
            tracing::warn!(%reason, "login shell env capture failed");
            fallback_env(reason)
        }
    }
}

#[cfg(windows)]
pub async fn capture_login_shell_env() -> OsEnvResult {
    OsEnvResult {
        env: std::env::vars().collect(),
        source: EnvSource::ProcessEnv,
    }
}

fn fallback_env(reason: String) -> OsEnvResult {
    OsEnvResult {
        env: std::env::vars().collect(),
        source: EnvSource::StdEnvFallback { reason },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_env_null_separated -------------------------------------------

    #[test]
    fn test_parse_normal_env() {
        let input = b"HOME=/home/user\0PATH=/usr/bin\0";
        let map = parse_env_null_separated(input);
        assert_eq!(map.get("HOME").unwrap(), "/home/user");
        assert_eq!(map.get("PATH").unwrap(), "/usr/bin");
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_parse_value_with_newline() {
        let input = b"MSG=hello\nworld\0";
        let map = parse_env_null_separated(input);
        assert_eq!(map.get("MSG").unwrap(), "hello\nworld");
    }

    #[test]
    fn test_parse_empty_input() {
        let map = parse_env_null_separated(b"");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_value_with_equals() {
        let input = b"FORMULA=a=b=c\0";
        let map = parse_env_null_separated(input);
        assert_eq!(map.get("FORMULA").unwrap(), "a=b=c");
    }

    #[test]
    fn test_parse_ignores_entries_without_equals() {
        let input = b"GOOD=val\0banner_noise\0ALSO=ok\0";
        let map = parse_env_null_separated(input);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("GOOD").unwrap(), "val");
        assert_eq!(map.get("ALSO").unwrap(), "ok");
    }

    #[test]
    fn test_parse_empty_value() {
        let input = b"EMPTY=\0";
        let map = parse_env_null_separated(input);
        assert_eq!(map.get("EMPTY").unwrap(), "");
    }

    #[test]
    fn test_parse_empty_key_ignored() {
        let input = b"=value\0VALID=ok\0";
        let map = parse_env_null_separated(input);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("VALID").unwrap(), "ok");
    }

    // -- parse_env_json -----------------------------------------------------

    #[test]
    fn test_parse_json_normal() {
        let json = r#"{"HOME":"/home/user","LANG":"en_US.UTF-8"}"#;
        let map = parse_env_json(json).unwrap();
        assert_eq!(map.get("HOME").unwrap(), "/home/user");
        assert_eq!(map.get("LANG").unwrap(), "en_US.UTF-8");
    }

    #[test]
    fn test_parse_json_nested_values_as_string() {
        let json = r#"{"SIMPLE":"val","OBJ":{"a":1},"ARR":[1,2]}"#;
        let map = parse_env_json(json).unwrap();
        assert_eq!(map.get("SIMPLE").unwrap(), "val");
        assert_eq!(map.get("OBJ").unwrap(), r#"{"a":1}"#);
        assert_eq!(map.get("ARR").unwrap(), "[1,2]");
    }

    #[test]
    fn test_parse_json_empty_object() {
        let map = parse_env_json("{}").unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_json_invalid() {
        assert!(parse_env_json("not json").is_err());
    }

    // -- detect_shell_type --------------------------------------------------

    #[test]
    fn test_detect_bash() {
        assert_eq!(detect_shell_type("/bin/bash"), ShellType::Bash);
    }

    #[test]
    fn test_detect_zsh() {
        assert_eq!(detect_shell_type("/usr/bin/zsh"), ShellType::Zsh);
    }

    #[test]
    fn test_detect_fish() {
        assert_eq!(detect_shell_type("/usr/local/bin/fish"), ShellType::Fish);
    }

    #[test]
    fn test_detect_nushell() {
        assert_eq!(detect_shell_type("/usr/bin/nu"), ShellType::Nushell);
    }

    #[test]
    fn test_detect_unknown_falls_back_to_sh() {
        assert_eq!(detect_shell_type("/bin/csh"), ShellType::Sh);
    }

    // -- build_env_capture_command ------------------------------------------

    #[test]
    fn test_build_command_bash() {
        let (prog, args) = build_env_capture_command(ShellType::Bash, "/bin/bash");
        assert_eq!(prog, "/bin/bash");
        assert_eq!(args, vec!["-l", "-c", "env -0"]);
    }

    #[test]
    fn test_build_command_zsh() {
        let (prog, args) = build_env_capture_command(ShellType::Zsh, "/bin/zsh");
        assert_eq!(prog, "/bin/zsh");
        assert_eq!(args, vec!["-l", "-c", "env -0"]);
    }

    #[test]
    fn test_build_command_fish() {
        let (prog, args) = build_env_capture_command(ShellType::Fish, "/usr/bin/fish");
        assert_eq!(prog, "/usr/bin/fish");
        assert_eq!(args, vec!["-l", "-c", "env -0"]);
    }

    #[test]
    fn test_build_command_nushell() {
        let (prog, args) = build_env_capture_command(ShellType::Nushell, "/usr/bin/nu");
        assert_eq!(prog, "/usr/bin/nu");
        assert_eq!(args, vec!["-l", "-c", "$env | to json"]);
    }

    #[test]
    fn test_build_command_sh_fallback() {
        let (prog, args) = build_env_capture_command(ShellType::Sh, "/bin/sh");
        assert_eq!(prog, "/bin/sh");
        assert_eq!(args, vec!["-l", "-c", "env -0"]);
    }
}
