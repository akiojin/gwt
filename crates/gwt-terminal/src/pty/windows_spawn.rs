use std::{
    collections::HashMap,
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
};

use super::SpawnConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsSpawnTarget {
    command: String,
    args_prefix: Vec<String>,
}

const CMD_WRAPPER_EXPRESSION_ENV: &str = "GWT_WINDOWS_CMD_WRAPPER_EXPRESSION";

pub(super) fn normalize_spawn_config(mut config: SpawnConfig) -> SpawnConfig {
    config.command = normalize_command_token(&config.command);
    if let Some(cwd) = config.cwd.as_ref() {
        config.cwd = Some(gwt_core::paths::normalize_windows_child_process_path(cwd));
    }

    let (command, args) = normalize_host_shell_command(
        &config.command,
        &config.args,
        &config.env,
        &config.remove_env,
    );

    match spawn_wrapper(Path::new(&command), &args, &config.env, &config.remove_env) {
        Some((command, args, expression)) => {
            config.command = command;
            config.args = args;
            config
                .env
                .insert(CMD_WRAPPER_EXPRESSION_ENV.to_string(), expression);
        }
        None => {
            config.command = command;
            config.args = args;
        }
    }

    config
}

pub(super) fn normalize_host_shell_command(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> (String, Vec<String>) {
    let command = normalize_command_token(command);
    let resolved = resolve_spawn_target(&command, env, remove_env).unwrap_or(WindowsSpawnTarget {
        command,
        args_prefix: Vec::new(),
    });
    let mut normalized_args = resolved.args_prefix;
    normalized_args.extend(args.iter().cloned());
    (resolved.command, normalized_args)
}

fn normalize_command_token(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.len() < 2 {
        return trimmed.to_string();
    }

    let mut chars = trimmed.chars();
    let first = chars.next();
    let last = chars.next_back();
    if matches!(
        (first, last),
        (Some('"'), Some('"')) | (Some('\''), Some('\''))
    ) {
        chars.as_str().to_string()
    } else {
        trimmed.to_string()
    }
}

fn escape_cmd_double_quoted(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn quote_cmd_token_if_needed(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.chars().any(|c| {
            c.is_whitespace()
                || matches!(c, '&' | '|' | '<' | '>' | '(' | ')' | '^' | '%' | '!' | '"')
        });

    if needs_quotes {
        format!("\"{}\"", escape_cmd_double_quoted(value))
    } else {
        value.to_string()
    }
}

fn build_cmd_command_expression(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push("call".to_string());
    parts.push(quote_cmd_token_if_needed(command));
    parts.extend(args.iter().map(|arg| quote_cmd_token_if_needed(arg)));
    parts.join(" ")
}

fn resolve_spawn_target(
    command: &str,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    let command_path = Path::new(command);
    let has_separator = command_path
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty());

    if has_separator || command_path.is_absolute() {
        return resolve_path_candidate(command_path, env, remove_env);
    }

    let path_value = windows_env_value("PATH", env, remove_env)?;
    for dir in std::env::split_paths(&path_value) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(command_path);
        if let Some(resolved) = resolve_path_candidate(&candidate, env, remove_env) {
            return Some(resolved);
        }
    }

    resolve_path_candidate(command_path, env, remove_env)
}

fn resolve_path_candidate(
    candidate: &Path,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    if has_executable_extension(candidate) && candidate.exists() {
        if let Some(target) = parse_bun_pe_shim(candidate, env, remove_env) {
            tracing::debug!(
                target: "gwt_spawn_trace",
                bun_pe_shim = %candidate.display(),
                rewritten_command = %target.command,
                "rewrote bun PE shim to JS entry to avoid Windows 16-bit loader error",
            );
            return Some(target);
        }
        if let Some(target) = redirect_placeholder_executable(candidate, env, remove_env) {
            tracing::debug!(
                target: "gwt_spawn_trace",
                placeholder_stub = %candidate.display(),
                rewritten_command = %target.command,
                "redirected native-binary placeholder stub to package launcher",
            );
            return Some(target);
        }
        return Some(WindowsSpawnTarget {
            command: candidate.display().to_string(),
            args_prefix: Vec::new(),
        });
    }

    if candidate.extension().is_none() {
        if candidate.exists() {
            if let Some(target) = parse_npm_shim(candidate) {
                return Some(guard_shim_target(target, env, remove_env));
            }
        }
        for ext in windows_path_extensions(env, remove_env) {
            let with_ext = candidate.with_extension(ext.trim_start_matches('.'));
            if let Some(target) = resolve_path_candidate(&with_ext, env, remove_env) {
                return Some(target);
            }
        }
    }

    resolve_existing_path(candidate).map(|target| guard_shim_target(target, env, remove_env))
}

fn spawn_wrapper(
    resolved: &Path,
    forwarded_args: &[String],
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<(String, Vec<String>, String)> {
    let ext = resolved
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())?;
    if ext != "cmd" && ext != "bat" {
        return None;
    }

    let comspec =
        windows_env_value("ComSpec", env, remove_env).unwrap_or_else(|| OsString::from("cmd.exe"));

    // SPEC-1921 FR-082: Do NOT pass `/s`. The command expression is expanded
    // from an env var so `portable-pty` does not backslash-escape the inner
    // quotes before CMD parses spaced `.cmd` paths.
    let expression = format!(
        "{} & exit",
        build_cmd_command_expression(&resolved.display().to_string(), forwarded_args)
    );
    Some((
        PathBuf::from(comspec).display().to_string(),
        vec![
            "/d".to_string(),
            "/k".to_string(),
            format!("%{CMD_WRAPPER_EXPRESSION_ENV}%"),
        ],
        expression,
    ))
}

fn resolve_existing_path(candidate: &Path) -> Option<WindowsSpawnTarget> {
    if !candidate.exists() {
        return None;
    }

    if let Some(target) = parse_npm_shim(candidate) {
        return Some(target);
    }

    Some(WindowsSpawnTarget {
        command: candidate.display().to_string(),
        args_prefix: Vec::new(),
    })
}

fn parse_npm_shim(candidate: &Path) -> Option<WindowsSpawnTarget> {
    match candidate
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("cmd") | Some("bat") => parse_cmd_shim(candidate),
        Some("exe") | Some("com") => None,
        _ => parse_shell_shim(candidate),
    }
}

fn parse_shell_shim(candidate: &Path) -> Option<WindowsSpawnTarget> {
    let content = std::fs::read_to_string(candidate).ok()?;
    let base_dir = candidate.parent()?;
    let basedir_paths = collect_marker_paths(&content, "$basedir/");
    if basedir_paths.is_empty() {
        return None;
    }
    build_shim_target(base_dir, &basedir_paths)
}

fn parse_cmd_shim(candidate: &Path) -> Option<WindowsSpawnTarget> {
    let content = std::fs::read_to_string(candidate).ok()?;
    let base_dir = candidate.parent()?;
    let dp0_paths = collect_marker_paths(&content, "%dp0%\\");
    if dp0_paths.is_empty() {
        return None;
    }
    build_shim_target(base_dir, &dp0_paths)
}

fn build_shim_target(base_dir: &Path, raw_paths: &[String]) -> Option<WindowsSpawnTarget> {
    let executable = raw_paths.iter().find_map(|path| {
        let lower = path.to_ascii_lowercase();
        (lower.ends_with(".exe") || lower.ends_with(".com"))
            .then(|| base_dir.join(normalize_rel_path(path)))
    });
    let script = raw_paths.iter().find_map(|path| {
        let lower = path.to_ascii_lowercase();
        (lower.ends_with(".js") || lower.ends_with(".cjs"))
            .then(|| base_dir.join(normalize_rel_path(path)))
    });

    match (executable, script) {
        (Some(executable), Some(script)) if is_node_runtime(&executable) => {
            let command = if executable.exists() {
                executable.display().to_string()
            } else {
                local_node_command(base_dir)
            };
            Some(WindowsSpawnTarget {
                command,
                args_prefix: vec![script.display().to_string()],
            })
        }
        // SPEC-1921 FR-081: Node.js distribution shims (e.g.
        // `C:\Program Files\nodejs\npx`) reference `$basedir/node.exe` but
        // dereference the CLI script via a separate variable such as
        // `$CLI_BASEDIR`. Our marker scan never pairs node with a `.js`
        // script in that case. Substituting `node.exe` alone would drop the
        // script and pass the caller's agent args (`--yes @pkg@version ...`)
        // straight to node, yielding `bad option: --yes`. Refuse the
        // substitution so resolution falls back to the `.cmd` sibling.
        (Some(executable), None) if is_node_runtime(&executable) => None,
        (Some(executable), _) if executable.exists() => Some(WindowsSpawnTarget {
            command: executable.display().to_string(),
            args_prefix: Vec::new(),
        }),
        (_, Some(script)) => Some(WindowsSpawnTarget {
            command: local_node_command(base_dir),
            args_prefix: vec![script.display().to_string()],
        }),
        _ => None,
    }
}

fn collect_marker_paths(content: &str, marker: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut remaining = content;
    while let Some(index) = remaining.find(marker) {
        let start = index + marker.len();
        let tail = &remaining[start..];
        let end = tail.find(['"', '\r', '\n']).unwrap_or(tail.len());
        let value = tail[..end].trim();
        if !value.is_empty() {
            values.push(value.to_string());
        }
        remaining = &tail[end..];
    }
    values
}

fn normalize_rel_path(value: &str) -> PathBuf {
    PathBuf::from(value.replace('/', "\\"))
}

fn local_node_command(base_dir: &Path) -> String {
    ["node.exe", "node"]
        .into_iter()
        .map(|name| base_dir.join(name))
        .find(|path| path.exists())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "node".to_string())
}

fn is_node_runtime(path: &Path) -> bool {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("node"))
        .unwrap_or(false)
}

fn windows_env_value(
    key: &str,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<OsString> {
    if let Some(value) = env
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| OsString::from(value))
    {
        return Some(value);
    }

    if remove_env
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(key))
    {
        return None;
    }

    std::env::var_os(key)
}

fn windows_path_extensions(env: &HashMap<String, String>, remove_env: &[String]) -> Vec<String> {
    let raw = windows_env_value("PATHEXT", env, remove_env)
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());

    raw.split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.to_ascii_lowercase())
        .collect()
}

// Detect bun's global install layout
// (`<...>/.bun/install/global/node_modules/...`). bun emits PE32+ shims here
// that occasionally trip Windows' PE loader and surface as a misleading
// "supported 16-bit application" dialog (notably when the user profile path
// contains non-ASCII characters or when bun's shim builder produces an image
// the loader rejects). Detection is case-insensitive because Windows paths
// are.
fn is_bun_managed_pe_shim(candidate: &Path) -> bool {
    let lowered: Vec<String> = candidate
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str().map(|s| s.to_ascii_lowercase()),
            _ => None,
        })
        .collect();

    lowered.windows(4).any(|window| {
        window[0] == ".bun"
            && window[1] == "install"
            && window[2] == "global"
            && window[3] == "node_modules"
    })
}

fn bun_global_node_modules_from_bin_shim(candidate: &Path) -> Option<PathBuf> {
    let bin_dir = candidate.parent()?;
    if !path_file_name_eq(bin_dir, "bin") {
        return None;
    }
    let bun_root = bin_dir.parent()?;
    if !path_file_name_eq(bun_root, ".bun") {
        return None;
    }
    Some(bun_root.join("install").join("global").join("node_modules"))
}

fn path_file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

// Walk up at most 5 levels from `<bin>/<name>.exe` to find the directory that
// owns `package.json`. Both flat (`node_modules/<pkg>/`) and scoped
// (`node_modules/@scope/<pkg>/`) layouts settle within this limit.
fn locate_bun_package_root(candidate: &Path) -> Option<PathBuf> {
    let mut current = candidate.parent()?;
    for _ in 0..5 {
        current = current.parent()?;
        if current.join("package.json").is_file() {
            return Some(current.to_path_buf());
        }
    }
    None
}

// Read the package's `bin` field. Accepts both string form
// (`"bin": "cli.js"`) and object form (`"bin": {"<name>": "cli.js"}`). When
// the object has a single entry we accept it regardless of name, matching
// what bun's resolver does when generating shims.
fn resolve_bin_entry_from_package_json(package_root: &Path, desired_name: &str) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(package_root.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let bin = json.get("bin")?;
    let relative = match bin {
        serde_json::Value::String(value) => value.as_str(),
        serde_json::Value::Object(map) => {
            let entry = map.get(desired_name).or_else(|| {
                if map.len() == 1 {
                    map.values().next()
                } else {
                    None
                }
            })?;
            entry.as_str()?
        }
        _ => return None,
    };
    Some(PathBuf::from(relative))
}

fn resolve_bin_entry_from_package_json_for_global(
    package_root: &Path,
    desired_name: &str,
) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(package_root.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let bin = json.get("bin")?;
    let relative = match bin {
        serde_json::Value::String(value) => {
            let package_name = package_root.file_name()?.to_str()?;
            package_name
                .eq_ignore_ascii_case(desired_name)
                .then_some(value.as_str())?
        }
        serde_json::Value::Object(map) => {
            let entry = map
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(desired_name))
                .map(|(_, value)| value)?;
            entry.as_str()?
        }
        _ => return None,
    };
    Some(PathBuf::from(relative))
}

fn resolve_bun_package_bin(
    package_root: &Path,
    desired_name: &str,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    let cli_relative = resolve_bin_entry_from_package_json(package_root, desired_name)?;
    resolve_bun_package_entry(package_root, desired_name, &cli_relative, env, remove_env)
}

fn resolve_bun_global_package_bin(
    package_root: &Path,
    desired_name: &str,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    let cli_relative = resolve_bin_entry_from_package_json_for_global(package_root, desired_name)?;
    resolve_bun_package_entry(package_root, desired_name, &cli_relative, env, remove_env)
}

fn resolve_bun_package_entry(
    package_root: &Path,
    desired_name: &str,
    cli_relative: &Path,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    let cli_absolute = package_root.join(cli_relative);
    if !cli_absolute.is_file() {
        return None;
    }
    if let Some(target) =
        resolve_bun_placeholder_target(package_root, desired_name, &cli_absolute, env, remove_env)
    {
        return Some(target);
    }
    let runtime = locate_bun_runtime(env, remove_env);
    Some(WindowsSpawnTarget {
        command: runtime,
        args_prefix: vec![cli_absolute.display().to_string()],
    })
}

fn resolve_bun_placeholder_target(
    package_root: &Path,
    desired_name: &str,
    cli_absolute: &Path,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    if !is_bun_text_placeholder_stub(cli_absolute) {
        return None;
    }

    let cli_wrapper = package_root.join("cli-wrapper.cjs");
    if cli_wrapper.is_file() {
        return Some(WindowsSpawnTarget {
            command: locate_bun_runtime(env, remove_env),
            args_prefix: vec![cli_wrapper.display().to_string()],
        });
    }

    optional_windows_native_binary(package_root, desired_name).map(|native| WindowsSpawnTarget {
        command: native.display().to_string(),
        args_prefix: Vec::new(),
    })
}

// Generic placeholder guard for shim resolution that is NOT bun-managed (npm
// global / npx / any npm-style `bin` shim). When a shim — or a directly
// resolved `.exe` — points at a native-binary text placeholder stub
// (`<pkg>/bin/<name>.exe`, shipped by packages like `@anthropic-ai/claude-code`
// before the postinstall copies the real binary in), redirect to the package's
// `cli-wrapper.cjs` (run via bun/node) or its `*-win32-x64` native binary,
// mirroring `resolve_bun_placeholder_target`. Returns `None` for a real PE, or
// for a placeholder with no redirect target; in the latter case the pre-spawn
// `reject_non_pe_executable` net refuses the raw stub with an actionable error
// instead of letting the Windows loader raise the misleading "16-bit
// application" dialog. The 2026-06-10 fix only guarded the bun PE-shim path
// (`parse_bun_pe_shim`); this extends the same guard to every other exit.
fn redirect_placeholder_executable(
    executable: &Path,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    let stem = executable.file_stem()?.to_str()?;
    let package_root = placeholder_package_root(executable)?;
    resolve_bun_placeholder_target(&package_root, stem, executable, env, remove_env)
}

// Find the package root that owns `executable` by walking up from its directory
// until a `package.json` is found. Covers both the common `<pkg>/bin/<name>.exe`
// layout and a root-level `<pkg>/<name>.exe`. Falls back to the immediate
// grandparent so behaviour is never worse than the prior `parent().parent()`.
fn placeholder_package_root(executable: &Path) -> Option<PathBuf> {
    let mut current = executable.parent()?;
    for _ in 0..4 {
        if current.join("package.json").is_file() {
            return Some(current.to_path_buf());
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    executable.parent()?.parent().map(Path::to_path_buf)
}

// Apply `redirect_placeholder_executable` to a shim-resolved target that points
// straight at a bare `.exe` (no runtime prefix). Every other target — runtime +
// script, or an already-rewritten command — is returned untouched.
fn guard_shim_target(
    target: WindowsSpawnTarget,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> WindowsSpawnTarget {
    if target.args_prefix.is_empty() {
        if let Some(redirected) =
            redirect_placeholder_executable(Path::new(&target.command), env, remove_env)
        {
            return redirected;
        }
    }
    target
}

fn is_bun_text_placeholder_stub(path: &Path) -> bool {
    if !has_executable_extension(path) {
        return false;
    }
    let Some(prefix) = read_file_prefix(path, 4096) else {
        return false;
    };
    if prefix.starts_with(b"MZ") {
        return false;
    }
    let text = String::from_utf8_lossy(&prefix).to_ascii_lowercase();
    text.contains("native binary not installed")
}

fn read_file_prefix(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0; limit];
    let len = file.read(&mut buf).ok()?;
    buf.truncate(len);
    Some(buf)
}

fn optional_windows_native_binary(package_root: &Path, desired_name: &str) -> Option<PathBuf> {
    let package_name = package_root.file_name()?.to_str()?;
    let package_parent = package_root.parent()?;
    let candidate = package_parent
        .join(format!("{package_name}-win32-x64"))
        .join(format!("{desired_name}.exe"));
    candidate.is_file().then_some(candidate)
}

// Locate a runtime that can execute the JS entry. Preference order:
//   1. `bun.exe` on PATH
//   2. `%USERPROFILE%\.bun\bin\bun.exe`
//   3. `node.exe` on PATH
//   4. `bun` (lets CreateProcess perform its own PATH search)
fn locate_bun_runtime(env: &HashMap<String, String>, remove_env: &[String]) -> String {
    if let Some(found) = find_executable_on_path("bun.exe", env, remove_env) {
        return found;
    }
    if let Some(home) =
        windows_env_value("USERPROFILE", env, remove_env).and_then(|value| value.into_string().ok())
    {
        let candidate = PathBuf::from(home).join(".bun").join("bin").join("bun.exe");
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }
    if let Some(found) = find_executable_on_path("node.exe", env, remove_env) {
        return found;
    }
    "bun".to_string()
}

fn find_executable_on_path(
    name: &str,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<String> {
    let path_value = windows_env_value("PATH", env, remove_env)?;
    for dir in std::env::split_paths(&path_value) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }
    None
}

fn collect_bun_global_package_roots(node_modules: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Ok(entries) = std::fs::read_dir(node_modules) else {
        return roots;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if name.starts_with('@') {
            if let Ok(scoped_entries) = std::fs::read_dir(&path) {
                for scoped_entry in scoped_entries.flatten() {
                    let scoped_path = scoped_entry.path();
                    if scoped_path.join("package.json").is_file() {
                        roots.push(scoped_path);
                    }
                }
            }
        } else if path.join("package.json").is_file() {
            roots.push(path);
        }
    }
    roots.sort();
    roots
}

fn parse_bun_pe_shim(
    candidate: &Path,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    let stem = candidate.file_stem()?.to_str()?;
    if is_bun_managed_pe_shim(candidate) {
        let package_root = locate_bun_package_root(candidate)?;
        return resolve_bun_package_bin(&package_root, stem, env, remove_env);
    }

    if let Some(node_modules) = bun_global_node_modules_from_bin_shim(candidate) {
        for package_root in collect_bun_global_package_roots(&node_modules) {
            if let Some(target) =
                resolve_bun_global_package_bin(&package_root, stem, env, remove_env)
            {
                return Some(target);
            }
        }
    }
    None
}

fn has_executable_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "exe" | "com"))
        .unwrap_or(false)
}

// Final pre-spawn safety net. Returns `Some(reason)` when `command` names an
// existing `.exe`/`.com` file whose leading bytes are not a PE (`MZ`) header —
// i.e. a native-binary text placeholder stub. Windows would otherwise reject
// such a file at `CreateProcess` time with the misleading "unsupported 16-bit
// application" dialog; surfacing an actionable error is far better UX. Bare
// commands, non-`.exe` paths, and real PE images all return `None`.
pub(super) fn reject_non_pe_executable(command: &str) -> Option<String> {
    let path = Path::new(command);
    if !has_executable_extension(path) || !path.is_file() {
        return None;
    }
    let prefix = read_file_prefix(path, 2)?;
    if prefix.starts_with(b"MZ") {
        return None;
    }
    // Tailor the remediation only when the file is a recognised native-binary
    // placeholder stub; otherwise keep the message accurate for any non-PE file.
    let detail = if is_bun_text_placeholder_stub(path) {
        " Its package ships a native-binary placeholder; the real executable was \
not installed (postinstall did not run). Reinstall the agent (e.g. \
`bun install -g <package>` or `npm install -g <package>`), or remove the stale \
install so a working launcher resolves on PATH."
    } else {
        ""
    };
    Some(format!(
        "'{}' is not a valid Windows executable (PE) image, so it cannot be launched.{}",
        path.display(),
        detail
    ))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, time::Duration};

    use super::*;
    use crate::pty::PtyHandle;
    use crate::test_util::{answer_cursor_position_query, lock_pty_test, read_until_contains};

    fn normalized_config(
        command: &str,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> SpawnConfig {
        normalize_spawn_config(SpawnConfig {
            command: command.to_string(),
            args,
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        })
    }

    #[test]
    fn strips_windows_verbatim_cwd_before_spawn() {
        let normalized = normalize_spawn_config(SpawnConfig {
            command: "cmd.exe".to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(PathBuf::from(
                r"Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\20260525-0919",
            )),
        });

        assert_eq!(
            normalized.cwd,
            Some(PathBuf::from(r"E:\gwt\work\20260525-0919"))
        );
    }

    #[test]
    fn wraps_cmd_shims_with_comspec() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let shim = bin_dir.join("claude");
        let cmd = bin_dir.join("claude.cmd");
        std::fs::write(&shim, "#!/bin/sh\n").expect("shim");
        std::fs::write(&cmd, "@echo off\r\n").expect("cmd");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert(
            "PATHEXT".to_string(),
            ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC".to_string(),
        );
        env.insert(
            "ComSpec".to_string(),
            r"C:\Windows\System32\cmd.exe".to_string(),
        );

        let normalized = normalized_config(
            "claude",
            vec!["--dangerously-skip-permissions".to_string()],
            env,
        );

        assert_eq!(normalized.command, r"C:\Windows\System32\cmd.exe");
        assert_eq!(
            normalized.args,
            vec![
                "/d".to_string(),
                "/k".to_string(),
                format!("%{CMD_WRAPPER_EXPRESSION_ENV}%"),
            ]
        );
        assert_eq!(
            normalized.env.get(CMD_WRAPPER_EXPRESSION_ENV),
            Some(&format!(
                "call {} --dangerously-skip-permissions & exit",
                cmd.display()
            ))
        );
    }

    #[test]
    fn build_cmd_command_expression_quotes_paths_and_metacharacters() {
        let expression = build_cmd_command_expression(
            r"C:\Program Files\nodejs\npx.cmd",
            &[
                "--cwd".to_string(),
                r"C:\Users\Test User\repo".to_string(),
                "a&b".to_string(),
                r#"arg "quoted" value"#.to_string(),
            ],
        );

        assert_eq!(
            expression,
            r#"call "C:\Program Files\nodejs\npx.cmd" --cwd "C:\Users\Test User\repo" "a&b" "arg ""quoted"" value""#
        );
    }

    #[test]
    fn prefers_real_exe_before_extensionless_shim() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let shim = bin_dir.join("codex");
        let exe = bin_dir.join("codex.exe");
        std::fs::write(&shim, "#!/bin/sh\n").expect("shim");
        std::fs::write(&exe, "not-a-real-pe-but-good-enough-for-path-selection").expect("exe");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config("codex", vec!["--no-alt-screen".to_string()], env);

        assert_eq!(normalized.command, exe.display().to_string());
        assert_eq!(normalized.args, vec!["--no-alt-screen".to_string()]);
    }

    #[test]
    fn resolves_shell_shim_js_entry_to_node() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let node_modules = bin_dir
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin");
        std::fs::create_dir_all(&node_modules).expect("node modules");
        let shim = bin_dir.join("codex");
        let local_node = bin_dir.join("node.exe");
        let script = node_modules.join("codex.js");
        std::fs::write(&local_node, "not-a-real-pe").expect("node exe");
        std::fs::write(&script, "console.log('codex');\n").expect("script");
        std::fs::write(
            &shim,
            "#!/bin/sh\nif [ -x \"$basedir/node\" ]; then\n  exec \"$basedir/node\" \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nelse\n  exec node \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nfi\n",
        )
        .expect("shim");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config("codex", vec!["--no-alt-screen".to_string()], env);

        assert_eq!(normalized.command, local_node.display().to_string());
        assert_eq!(
            normalized.args,
            vec![script.display().to_string(), "--no-alt-screen".to_string()]
        );
    }

    #[test]
    fn preserves_script_arg_for_node_exe_shims() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let node_modules = bin_dir
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin");
        std::fs::create_dir_all(&node_modules).expect("node modules");
        let shim = bin_dir.join("codex");
        let local_node = bin_dir.join("node.exe");
        let script = node_modules.join("codex.js");
        std::fs::write(&local_node, "not-a-real-pe").expect("node exe");
        std::fs::write(&script, "console.log('codex');\n").expect("script");
        std::fs::write(
            &shim,
            "#!/bin/sh\nif [ -x \"$basedir/node.exe\" ]; then\n  exec \"$basedir/node.exe\" \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nelse\n  exec node \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nfi\n",
        )
        .expect("shim");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config("codex", vec!["--no-alt-screen".to_string()], env);

        assert_eq!(normalized.command, local_node.display().to_string());
        assert_eq!(
            normalized.args,
            vec![script.display().to_string(), "--no-alt-screen".to_string()]
        );
    }

    #[test]
    fn falls_back_to_node_when_shim_runtime_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let node_modules = bin_dir
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin");
        std::fs::create_dir_all(&node_modules).expect("node modules");
        let shim = bin_dir.join("codex");
        let script = node_modules.join("codex.js");
        std::fs::write(&script, "console.log('codex');\n").expect("script");
        std::fs::write(
            &shim,
            "#!/bin/sh\nif [ -x \"$basedir/node.exe\" ]; then\n  exec \"$basedir/node.exe\" \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nelse\n  exec node \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nfi\n",
        )
        .expect("shim");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config("codex", vec!["--no-alt-screen".to_string()], env);

        assert_eq!(normalized.command, "node");
        assert_eq!(
            normalized.args,
            vec![script.display().to_string(), "--no-alt-screen".to_string()]
        );
    }

    #[test]
    fn env_override_beats_remove_env() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), r"C:\custom\bin".to_string());

        let value = windows_env_value("PATH", &env, &[String::from("PATH")]);

        assert_eq!(value, Some(OsString::from(r"C:\custom\bin")));
    }

    #[test]
    fn spawn_succeeds_via_shell_shim_resolved_to_exe() {
        let _pty_guard = lock_pty_test();
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let shim = bin_dir.join("claude");
        let tool = bin_dir.join("tool.exe");
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
        let whoami = PathBuf::from(system_root)
            .join("System32")
            .join("whoami.exe");
        std::fs::copy(&whoami, &tool).expect("copy whoami");
        std::fs::write(&shim, "#!/bin/sh\nexec \"$basedir/tool.exe\" \"$@\"\n").expect("shim");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert(
            "PATHEXT".to_string(),
            ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC".to_string(),
        );

        let config = SpawnConfig {
            command: "claude".to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        };

        let handle = PtyHandle::spawn(config).expect("spawn failed");
        assert!(handle.process_id().is_some(), "expected spawned process id");
    }

    #[test]
    fn nodejs_distribution_npx_shim_does_not_collapse_to_node_exe() {
        // Regression for `node.exe: bad option: --yes` (SPEC-1921 FR-081).
        // Mechanism is documented in `build_shim_target`.
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("Program Files").join("nodejs");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");

        let npx_shim = bin_dir.join("npx");
        let npx_cmd = bin_dir.join("npx.cmd");
        let node_exe = bin_dir.join("node.exe");
        std::fs::write(&node_exe, "not-a-real-pe").expect("node exe placeholder");
        std::fs::write(
            &npx_shim,
            concat!(
                "#!/usr/bin/env bash\n",
                "basedir=`dirname \"$0\"`\n",
                "NODE_EXE=\"$basedir/node.exe\"\n",
                "if ! [ -x \"$NODE_EXE\" ]; then\n",
                "  NODE_EXE=\"$basedir/node\"\n",
                "fi\n",
                "CLI_BASEDIR=\"$(\"$NODE_EXE\" -p 'require(\"path\").dirname(process.execPath)' 2> /dev/null)\"\n",
                "NPX_CLI_JS=\"$CLI_BASEDIR/node_modules/npm/bin/npx-cli.js\"\n",
                "\"$NODE_EXE\" \"$NPX_CLI_JS\" \"$@\"\n",
            ),
        )
        .expect("npx shim");
        std::fs::write(
            &npx_cmd,
            concat!(
                "@ECHO OFF\n",
                "SET \"NODE_EXE=%~dp0\\node.exe\"\n",
                "\"%NODE_EXE%\" \"%~dp0\\node_modules\\npm\\bin\\npx-cli.js\" %*\n",
            ),
        )
        .expect("npx.cmd");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config(
            "npx",
            vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
            ],
            env,
        );

        assert_ne!(
            normalized.command,
            node_exe.display().to_string(),
            "parser must not collapse a Node.js distribution shim to node.exe alone (FR-081): {:?} {:?}",
            normalized.command,
            normalized.args,
        );
        assert!(
            normalized.args.iter().any(|a| a.eq_ignore_ascii_case("/k")),
            "interactive batch shim should stay on /k wrapper, got {:?}",
            normalized.args,
        );
        let expected_expression = format!(
            "call \"{}\" --yes @anthropic-ai/claude-code@latest & exit",
            npx_cmd.display()
        );
        assert!(
            normalized.env.get(CMD_WRAPPER_EXPRESSION_ENV) == Some(&expected_expression),
            "expected original package spec preserved inside cmd wrapper expression, got {:?}",
            normalized.env,
        );
    }

    #[test]
    fn spawn_succeeds_via_spaced_npx_cmd_path() {
        let _pty_guard = lock_pty_test();
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("Program Files").join("nodejs");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let cmd_path = bin_dir.join("npx.cmd");
        std::fs::write(
            &cmd_path,
            "@echo off\r\necho GWT_NPX_OK %*\r\nexit /b 0\r\n",
        )
        .expect("npx.cmd");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let handle = PtyHandle::spawn(SpawnConfig {
            command: "npx".to_string(),
            args: vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
            ],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        })
        .expect("spawn npx.cmd");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader");

        let output = read_until_contains(reader, Duration::from_secs(5), "GWT_NPX_OK")
            .expect("read npx output");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("GWT_NPX_OK --yes @anthropic-ai/claude-code@latest"),
            "expected fake npx.cmd to receive forwarded args, got: {text}"
        );
    }

    #[test]
    fn spawn_strips_outer_quotes_from_spaced_npx_cmd_path() {
        let _pty_guard = lock_pty_test();
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("Program Files").join("nodejs");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let cmd_path = bin_dir.join("npx.cmd");
        std::fs::write(
            &cmd_path,
            "@echo off\r\necho GWT_QUOTED_NPX_OK %*\r\nexit /b 0\r\n",
        )
        .expect("npx.cmd");

        let handle = PtyHandle::spawn(SpawnConfig {
            command: format!("\"{}\"", cmd_path.display()),
            args: vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
            ],
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        })
        .expect("spawn quoted npx.cmd");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader");

        let output = read_until_contains(reader, Duration::from_secs(5), "GWT_QUOTED_NPX_OK")
            .expect("read quoted npx output");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("GWT_QUOTED_NPX_OK --yes @anthropic-ai/claude-code@latest"),
            "expected quoted fake npx.cmd to receive forwarded args, got: {text}"
        );
    }

    /// Build a fake bun-managed package layout under `root` and return
    /// `(bin_exe, package_root, cli_js)`. Mirrors the real bun layout:
    /// `<root>/.bun/install/global/node_modules/<pkg>/bin/<name>.exe` plus a
    /// sibling `cli.js` and a minimal `package.json`.
    fn fake_bun_install(
        root: &Path,
        pkg: &str,
        bin_name: &str,
        bin_field: &str,
        cli_relative: &str,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let pkg_root = root
            .join(".bun")
            .join("install")
            .join("global")
            .join("node_modules")
            .join(pkg);
        let bin_dir = pkg_root.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create bin dir");
        let bin_exe = bin_dir.join(format!("{bin_name}.exe"));
        // Minimal MZ header so the file looks like a PE; the resolver should
        // never actually execute it.
        std::fs::write(&bin_exe, b"MZ\x00\x00bun-shim-placeholder").expect("write fake exe");
        let cli_js = pkg_root.join(cli_relative);
        if let Some(parent) = cli_js.parent() {
            std::fs::create_dir_all(parent).expect("create cli parent");
        }
        std::fs::write(&cli_js, "console.log('cli');\n").expect("write cli.js");
        let package_json = pkg_root.join("package.json");
        std::fs::write(&package_json, bin_field).expect("write package.json");
        (bin_exe, pkg_root, cli_js)
    }

    #[test]
    fn bun_pe_shim_in_dot_bun_install_rewrites_to_bun_runtime() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (_bin_exe, _pkg_root, cli_js) = fake_bun_install(
            temp.path(),
            "@anthropic-ai/claude-code",
            "claude",
            r#"{"bin":{"claude":"cli.js"}}"#,
            "cli.js",
        );
        let bun_bin_dir = temp.path().join(".bun").join("bin");
        std::fs::create_dir_all(&bun_bin_dir).expect("bun bin");
        let bun_exe = bun_bin_dir.join("bun.exe");
        std::fs::write(&bun_exe, b"MZ\x00\x00fake-bun").expect("bun.exe");
        // Place a sibling shim in `.bun/bin/` so the PATH lookup hits the
        // bun-managed PE first; resolver should reject it via the new logic.
        let shim = bun_bin_dir.join("claude.exe");
        std::fs::write(&shim, b"MZ\x00\x00fake-bun-shim").expect("claude shim");
        // PATH is irrelevant once the resolver finds claude.exe in .bun/bin,
        // but we still need to exercise the deeper `.bun/install` path. Use a
        // fully-qualified candidate by passing the deep path directly.

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bun_bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config(
            _bin_exe.display().to_string().as_str(),
            vec!["--help".to_string()],
            env,
        );

        assert_eq!(
            normalized.command,
            bun_exe.display().to_string(),
            "expected bun.exe to drive the rewritten command, got {:?}",
            normalized.command,
        );
        assert_eq!(
            normalized.args,
            vec![cli_js.display().to_string(), "--help".to_string()],
            "expected cli.js to be inserted as the first arg, got {:?}",
            normalized.args,
        );
    }

    #[test]
    fn bun_pe_shim_with_string_bin_field_resolves_entry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (bin_exe, _pkg_root, cli_js) = fake_bun_install(
            temp.path(),
            "claude-code",
            "claude",
            r#"{"bin":"cli.js"}"#,
            "cli.js",
        );
        let bun_bin_dir = temp.path().join(".bun").join("bin");
        std::fs::create_dir_all(&bun_bin_dir).expect("bun bin");
        let bun_exe = bun_bin_dir.join("bun.exe");
        std::fs::write(&bun_exe, b"MZ\x00").expect("bun.exe");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bun_bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config(bin_exe.display().to_string().as_str(), vec![], env);

        assert_eq!(normalized.command, bun_exe.display().to_string());
        assert_eq!(normalized.args, vec![cli_js.display().to_string()]);
    }

    #[test]
    fn bun_pe_shim_falls_back_to_node_when_bun_runtime_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (bin_exe, _pkg_root, cli_js) = fake_bun_install(
            temp.path(),
            "@anthropic-ai/claude-code",
            "claude",
            r#"{"bin":{"claude":"cli.js"}}"#,
            "cli.js",
        );
        let nodejs_dir = temp.path().join("nodejs");
        std::fs::create_dir_all(&nodejs_dir).expect("nodejs dir");
        let node_exe = nodejs_dir.join("node.exe");
        std::fs::write(&node_exe, b"MZ\x00").expect("node.exe");

        // PATH contains only nodejs/, no bun.exe anywhere.
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), nodejs_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        // Force USERPROFILE elsewhere so the fallback can't surprise us.
        env.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let normalized = normalized_config(bin_exe.display().to_string().as_str(), vec![], env);

        assert_eq!(
            normalized.command,
            node_exe.display().to_string(),
            "expected node.exe fallback when bun.exe is absent, got {:?}",
            normalized.command,
        );
        assert_eq!(normalized.args, vec![cli_js.display().to_string()]);
    }

    #[test]
    fn claude_placeholder_stub_resolves_to_cli_wrapper_instead_of_direct_exe() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (bin_exe, _pkg_root, cli_wrapper) = fake_bun_install(
            temp.path(),
            "@anthropic-ai/claude-code",
            "claude",
            r#"{"bin":{"claude":"bin/claude.exe"}}"#,
            "cli-wrapper.cjs",
        );
        std::fs::write(
            &bin_exe,
            "echo \"Error: claude native binary not installed.\" >&2\nexit 1\n",
        )
        .expect("write placeholder stub");
        let nodejs_dir = temp.path().join("nodejs");
        std::fs::create_dir_all(&nodejs_dir).expect("nodejs dir");
        let node_exe = nodejs_dir.join("node.exe");
        std::fs::write(&node_exe, b"MZ\x00").expect("node.exe");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), nodejs_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        env.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let normalized = normalized_config(
            bin_exe.display().to_string().as_str(),
            vec!["--version".to_string()],
            env,
        );

        assert_eq!(normalized.command, node_exe.display().to_string());
        assert_eq!(normalized.args.len(), 2);
        assert_eq!(
            PathBuf::from(&normalized.args[0]),
            cli_wrapper,
            "expected cli-wrapper.cjs as first arg"
        );
        assert_eq!(normalized.args[1], "--version");
    }

    #[test]
    fn claude_placeholder_stub_prefers_bun_runtime_when_node_is_absent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (bin_exe, _pkg_root, cli_wrapper) = fake_bun_install(
            temp.path(),
            "@anthropic-ai/claude-code",
            "claude",
            r#"{"bin":{"claude":"bin/claude.exe"}}"#,
            "cli-wrapper.cjs",
        );
        std::fs::write(
            &bin_exe,
            "echo \"Error: claude native binary not installed.\" >&2\nexit 1\n",
        )
        .expect("write placeholder stub");
        let bun_bin_dir = temp.path().join(".bun").join("bin");
        std::fs::create_dir_all(&bun_bin_dir).expect("bun bin");
        let bun_exe = bun_bin_dir.join("bun.exe");
        std::fs::write(&bun_exe, b"MZ\x00").expect("bun.exe");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bun_bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        env.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let normalized = normalized_config(
            bin_exe.display().to_string().as_str(),
            vec!["--version".to_string()],
            env,
        );

        assert_eq!(normalized.command, bun_exe.display().to_string());
        assert_eq!(
            normalized.args,
            vec![cli_wrapper.display().to_string(), "--version".to_string()]
        );
    }

    /// Build a *generic*, non-bun-managed npm-style install under `root` and
    /// return `(shim, bin_exe, cli_wrapper)`. Layout:
    /// `<root>/npm/{<name>, node_modules/<pkg>/{bin/<name>.exe, cli-wrapper.cjs,
    /// package.json}}` where `bin/<name>.exe` is a text placeholder stub. None
    /// of this lives under `.bun/install/global`, so `parse_bun_pe_shim` does
    /// not apply — resolution must flow through the generic shim guard.
    fn fake_generic_npm_install(root: &Path, pkg: &str, name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let npm_dir = root.join("npm");
        let pkg_root = npm_dir.join("node_modules").join(pkg);
        let bin_dir = pkg_root.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create pkg bin dir");
        let bin_exe = bin_dir.join(format!("{name}.exe"));
        std::fs::write(
            &bin_exe,
            "echo \"Error: claude native binary not installed.\" >&2\nexit 1\n",
        )
        .expect("write placeholder stub");
        let cli_wrapper = pkg_root.join("cli-wrapper.cjs");
        std::fs::write(&cli_wrapper, "console.log('wrapper');\n").expect("cli-wrapper");
        std::fs::write(
            pkg_root.join("package.json"),
            format!(r#"{{"bin":{{"{name}":"bin/{name}.exe"}}}}"#),
        )
        .expect("package.json");
        let shim = npm_dir.join(name);
        std::fs::write(
            &shim,
            format!("#!/bin/sh\nexec \"$basedir/node_modules/{pkg}/bin/{name}.exe\" \"$@\"\n"),
        )
        .expect("shim");
        (shim, bin_exe, cli_wrapper)
    }

    #[test]
    fn generic_npm_shell_shim_placeholder_redirects_to_cli_wrapper() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (_shim, _bin_exe, cli_wrapper) =
            fake_generic_npm_install(temp.path(), "@anthropic-ai/claude-code", "claude");
        let nodejs_dir = temp.path().join("nodejs");
        std::fs::create_dir_all(&nodejs_dir).expect("nodejs dir");
        let node_exe = nodejs_dir.join("node.exe");
        std::fs::write(&node_exe, b"MZ\x00").expect("node.exe");

        let mut env = HashMap::new();
        env.insert(
            "PATH".to_string(),
            format!(
                "{};{}",
                temp.path().join("npm").display(),
                nodejs_dir.display()
            ),
        );
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        env.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let normalized = normalized_config("claude", vec!["--version".to_string()], env);

        assert_eq!(
            normalized.command,
            node_exe.display().to_string(),
            "generic npm placeholder must resolve through a runtime, not the raw stub: {:?}",
            normalized.command,
        );
        assert_eq!(
            PathBuf::from(&normalized.args[0]),
            cli_wrapper,
            "expected cli-wrapper.cjs as first arg, got {:?}",
            normalized.args,
        );
        assert_eq!(normalized.args[1], "--version");
    }

    #[test]
    fn direct_generic_placeholder_exe_path_redirects_to_cli_wrapper() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (_shim, bin_exe, cli_wrapper) =
            fake_generic_npm_install(temp.path(), "@anthropic-ai/claude-code", "claude");
        let nodejs_dir = temp.path().join("nodejs");
        std::fs::create_dir_all(&nodejs_dir).expect("nodejs dir");
        let node_exe = nodejs_dir.join("node.exe");
        std::fs::write(&node_exe, b"MZ\x00").expect("node.exe");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), nodejs_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        env.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let normalized = normalized_config(bin_exe.display().to_string().as_str(), vec![], env);

        assert_eq!(normalized.command, node_exe.display().to_string());
        assert_eq!(PathBuf::from(&normalized.args[0]), cli_wrapper);
    }

    #[test]
    fn reject_non_pe_executable_flags_placeholder_stub_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stub = temp.path().join("claude.exe");
        std::fs::write(&stub, "native binary not installed\n").expect("stub");
        let real = temp.path().join("real.exe");
        std::fs::write(&real, b"MZ\x00\x00real-pe").expect("real");
        let script = temp.path().join("cli.cjs");
        std::fs::write(&script, "console.log('x')\n").expect("script");

        assert!(
            reject_non_pe_executable(stub.display().to_string().as_str()).is_some(),
            "non-PE .exe stub must be rejected",
        );
        assert!(
            reject_non_pe_executable(real.display().to_string().as_str()).is_none(),
            "real PE .exe must not be rejected",
        );
        assert!(
            reject_non_pe_executable(script.display().to_string().as_str()).is_none(),
            "non-exe file must not be rejected",
        );
        assert!(
            reject_non_pe_executable("bun").is_none(),
            "bare command must not be rejected",
        );
    }

    #[test]
    fn generic_npm_cmd_shim_placeholder_redirects_to_cli_wrapper() {
        let temp = tempfile::tempdir().expect("tempdir");
        // Drive resolution through a `.cmd` shim using the `%dp0%\` marker so the
        // parse_cmd_shim -> build_shim_target -> guard_shim_target exit (the
        // resolve_existing_path call site) is exercised, distinct from the
        // extensionless shell-shim exit covered above.
        let npm_dir = temp.path().join("npm");
        let pkg_root = npm_dir
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        let bin_dir = pkg_root.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let bin_exe = bin_dir.join("claude.exe");
        std::fs::write(
            &bin_exe,
            "echo \"Error: claude native binary not installed.\" >&2\nexit 1\n",
        )
        .expect("placeholder");
        let cli_wrapper = pkg_root.join("cli-wrapper.cjs");
        std::fs::write(&cli_wrapper, "console.log('w');\n").expect("cli-wrapper");
        std::fs::write(
            pkg_root.join("package.json"),
            r#"{"bin":{"claude":"bin/claude.exe"}}"#,
        )
        .expect("package.json");
        let cmd_shim = npm_dir.join("claude.cmd");
        std::fs::write(
            &cmd_shim,
            "@echo off\r\n\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\" %*\r\n",
        )
        .expect("cmd shim");

        let nodejs_dir = temp.path().join("nodejs");
        std::fs::create_dir_all(&nodejs_dir).expect("nodejs dir");
        let node_exe = nodejs_dir.join("node.exe");
        std::fs::write(&node_exe, b"MZ\x00").expect("node.exe");

        let mut env = HashMap::new();
        env.insert(
            "PATH".to_string(),
            format!("{};{}", npm_dir.display(), nodejs_dir.display()),
        );
        env.insert("PATHEXT".to_string(), ".CMD".to_string());
        env.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let normalized = normalized_config("claude", vec!["--version".to_string()], env);

        assert_eq!(
            normalized.command,
            node_exe.display().to_string(),
            "cmd-shim placeholder must redirect through a runtime, got {:?}",
            normalized.command,
        );
        assert_eq!(PathBuf::from(&normalized.args[0]), cli_wrapper);
        assert_eq!(normalized.args[1], "--version");
    }

    #[test]
    fn spawn_rejects_non_pe_placeholder_with_actionable_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stub = temp.path().join("claude.exe");
        std::fs::write(&stub, "Error: native binary not installed\n").expect("stub");

        let result = PtyHandle::spawn(SpawnConfig {
            command: stub.display().to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        });
        let msg = match result {
            Ok(_) => panic!("spawning a non-PE placeholder stub must fail"),
            Err(err) => err.to_string(),
        };
        assert!(
            msg.contains("not a valid Windows executable"),
            "expected actionable non-PE error, got: {msg}"
        );
    }

    #[test]
    fn placeholder_without_redirect_target_is_left_for_pre_spawn_net() {
        let temp = tempfile::tempdir().expect("tempdir");
        // Placeholder bin present, but NO cli-wrapper.cjs and NO *-win32-x64
        // native sibling: redirect must decline, leaving the raw stub for the
        // pre-spawn reject net to refuse rather than launching it.
        let pkg_root = temp
            .path()
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        let bin_dir = pkg_root.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let bin_exe = bin_dir.join("claude.exe");
        std::fs::write(&bin_exe, "Error: native binary not installed\n").expect("stub");
        std::fs::write(
            pkg_root.join("package.json"),
            r#"{"bin":{"claude":"bin/claude.exe"}}"#,
        )
        .expect("package.json");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        env.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let normalized = normalized_config(bin_exe.display().to_string().as_str(), vec![], env);
        assert_eq!(
            normalized.command,
            bin_exe.display().to_string(),
            "redirect should decline when no cli-wrapper/native exists",
        );
        assert!(
            reject_non_pe_executable(&normalized.command).is_some(),
            "raw placeholder stub must be caught by the pre-spawn net",
        );
    }

    #[test]
    fn bun_global_bin_shim_resolves_package_before_direct_exe() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (bin_exe, _pkg_root, cli_wrapper) = fake_bun_install(
            temp.path(),
            "@anthropic-ai/claude-code",
            "claude",
            r#"{"bin":{"claude":"bin/claude.exe"}}"#,
            "cli-wrapper.cjs",
        );
        std::fs::write(
            &bin_exe,
            "echo \"Error: claude native binary not installed.\" >&2\nexit 1\n",
        )
        .expect("write placeholder stub");
        let bun_bin_dir = temp.path().join(".bun").join("bin");
        std::fs::create_dir_all(&bun_bin_dir).expect("bun bin");
        let bun_global_shim = bun_bin_dir.join("claude.exe");
        std::fs::write(&bun_global_shim, b"MZ\x00\x00bun-global-shim").expect("global shim");
        let nodejs_dir = temp.path().join("nodejs");
        std::fs::create_dir_all(&nodejs_dir).expect("nodejs dir");
        let node_exe = nodejs_dir.join("node.exe");
        std::fs::write(&node_exe, b"MZ\x00").expect("node.exe");

        let mut env = HashMap::new();
        env.insert(
            "PATH".to_string(),
            std::env::join_paths([bun_bin_dir.as_path(), nodejs_dir.as_path()])
                .expect("join PATH")
                .to_string_lossy()
                .into_owned(),
        );
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        env.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let normalized = normalized_config("claude", vec!["--print".to_string()], env);

        assert_eq!(normalized.command, node_exe.display().to_string());
        assert_eq!(normalized.args.len(), 2);
        assert_eq!(
            PathBuf::from(&normalized.args[0]),
            cli_wrapper,
            "expected cli-wrapper.cjs as first arg"
        );
        assert_eq!(normalized.args[1], "--print");
    }

    #[test]
    fn bun_pe_shim_outside_dot_bun_install_is_not_rewritten() {
        // Regression guard: a `.exe` outside `.bun/install/global/node_modules/`
        // must keep the existing direct-execution behavior so non-bun installs
        // (Program Files, scoop, winget, etc.) are unaffected.
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let exe = bin_dir.join("claude.exe");
        std::fs::write(&exe, b"MZ\x00\x00not-a-bun-shim").expect("exe");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config("claude", vec!["--help".to_string()], env);

        assert_eq!(normalized.command, exe.display().to_string());
        assert_eq!(normalized.args, vec!["--help".to_string()]);
    }

    #[test]
    fn bun_pe_shim_missing_package_json_falls_back_to_exe() {
        // Defense-in-depth: if `package.json` is missing or unreadable, the
        // resolver must fall back to the existing `.exe` direct-execution
        // path so a damaged install does not regress further.
        let temp = tempfile::tempdir().expect("tempdir");
        let pkg_root = temp
            .path()
            .join(".bun")
            .join("install")
            .join("global")
            .join("node_modules")
            .join("claude-code");
        let bin_dir = pkg_root.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let bin_exe = bin_dir.join("claude.exe");
        std::fs::write(&bin_exe, b"MZ\x00\x00bun-shim").expect("exe");
        // Intentionally do NOT create package.json.

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalized_config(bin_exe.display().to_string().as_str(), vec![], env);

        assert_eq!(
            normalized.command,
            bin_exe.display().to_string(),
            "expected fallback to .exe direct execution when package.json is missing"
        );
        assert!(normalized.args.is_empty());
    }

    #[test]
    fn cmd_wrapper_omits_slash_s_flag() {
        // SPEC-1921 FR-082. `/s` makes CMD strip the quoting around the
        // executable path, which breaks `.cmd` invocations where the path
        // contains spaces (for example `C:\Program Files\nodejs\npx.cmd`).
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("Program Files").join("nodejs");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let cmd_path = bin_dir.join("npx.cmd");
        std::fs::write(&cmd_path, "@echo off\n").expect("cmd");

        let env: HashMap<String, String> = HashMap::new();
        let wrapped = spawn_wrapper(
            &cmd_path,
            &[
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
            ],
            &env,
            &[],
        )
        .expect("wrapper");

        assert!(
            !wrapped.1.iter().any(|a| a.eq_ignore_ascii_case("/s")),
            "cmd.exe wrapper must not include /s (FR-082), got argv {:?}",
            wrapped.1,
        );
        assert!(
            wrapped.1.iter().any(|a| a.eq_ignore_ascii_case("/d")),
            "wrapper should still include /d, got {:?}",
            wrapped.1,
        );
        assert!(
            wrapped.1.iter().any(|a| a.eq_ignore_ascii_case("/k")),
            "interactive wrapper should use /k so ConPTY input forwarding stays intact, got {:?}",
            wrapped.1,
        );
        let expected_expression = format!(
            "call \"{}\" --yes @anthropic-ai/claude-code@latest & exit",
            cmd_path.display()
        );
        assert_eq!(
            wrapped.2.as_str(),
            expected_expression.as_str(),
            "wrapper should preserve quoting for spaced shim paths and append `& exit`, got {:?}",
            wrapped,
        );
    }
}
