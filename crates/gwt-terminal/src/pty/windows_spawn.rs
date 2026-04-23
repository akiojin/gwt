use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::SpawnConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsSpawnTarget {
    command: String,
    args_prefix: Vec<String>,
}

pub(super) fn normalize_spawn_config(mut config: SpawnConfig) -> SpawnConfig {
    let resolved = resolve_spawn_target(&config.command, &config.env, &config.remove_env)
        .unwrap_or_else(|| WindowsSpawnTarget {
            command: config.command.clone(),
            args_prefix: Vec::new(),
        });

    match spawn_wrapper(
        Path::new(&resolved.command),
        &config.args,
        &config.env,
        &config.remove_env,
    ) {
        Some((command, args)) => {
            config.command = command;
            config.args = args;
        }
        None => {
            let mut args = resolved.args_prefix;
            args.extend(config.args);
            config.command = resolved.command;
            config.args = args;
        }
    }

    config
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
        return Some(WindowsSpawnTarget {
            command: candidate.display().to_string(),
            args_prefix: Vec::new(),
        });
    }

    if candidate.extension().is_none() {
        if candidate.exists() {
            if let Some(target) = parse_npm_shim(candidate) {
                return Some(target);
            }
        }
        for ext in windows_path_extensions(env, remove_env) {
            let with_ext = candidate.with_extension(ext.trim_start_matches('.'));
            if let Some(target) = resolve_existing_path(&with_ext) {
                return Some(target);
            }
        }
    }

    resolve_existing_path(candidate)
}

fn spawn_wrapper(
    resolved: &Path,
    forwarded_args: &[String],
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<(String, Vec<String>)> {
    let ext = resolved
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())?;
    if ext != "cmd" && ext != "bat" {
        return None;
    }

    let comspec =
        windows_env_value("ComSpec", env, remove_env).unwrap_or_else(|| OsString::from("cmd.exe"));

    // SPEC-1921 FR-082: Do NOT pass `/s`. `/s` forces CMD to strip the
    // quotes that surround the executable path, which breaks invocations
    // with whitespace in the path (e.g. `C:\Program Files\nodejs\npx.cmd`).
    // Without `/s`, CMD's default rule preserves the quotes when the
    // command line has the typical `"<exe>" <args>` shape we emit here.
    let expression = format!(
        "{} & exit",
        build_cmd_command_expression(&resolved.display().to_string(), forwarded_args)
    );
    Some((
        PathBuf::from(comspec).display().to_string(),
        vec!["/d".to_string(), "/k".to_string(), expression],
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

fn has_executable_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "exe" | "com"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use super::*;
    use crate::pty::PtyHandle;
    use crate::test_util::lock_pty_test;

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
                format!("{} --dangerously-skip-permissions & exit", cmd.display()),
            ]
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
            r#""C:\Program Files\nodejs\npx.cmd" --cwd "C:\Users\Test User\repo" "a&b" "arg ""quoted"" value""#
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
            "\"{}\" --yes @anthropic-ai/claude-code@latest & exit",
            npx_cmd.display()
        );
        assert!(
            normalized
                .args
                .iter()
                .any(|arg| arg == &expected_expression),
            "expected original package spec preserved inside cmd wrapper expression, got {:?}",
            normalized.args,
        );
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
            "\"{}\" --yes @anthropic-ai/claude-code@latest & exit",
            cmd_path.display()
        );
        assert_eq!(
            wrapped.1.last().map(String::as_str),
            Some(expected_expression.as_str()),
            "wrapper should preserve quoting for spaced shim paths and append `& exit`, got {:?}",
            wrapped.1,
        );
    }
}
