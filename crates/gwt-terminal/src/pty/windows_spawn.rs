use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::Path,
};

use gwt_core::process::{
    resolve_process_plan_for_platform, ProcessPlanRequest, ProcessPlatform, ProcessResolveFailure,
    ProcessResolveFailureKind, ResolvedProcessPlan, WINDOWS_CMD_WRAPPER_EXPRESSION_ENV,
};

use super::SpawnConfig;

pub(super) fn normalize_spawn_config(
    mut config: SpawnConfig,
) -> Result<SpawnConfig, ProcessResolveFailure> {
    config.command = normalize_command_token(&config.command);
    if let Some(cwd) = config.cwd.as_ref() {
        config.cwd = Some(gwt_core::paths::normalize_windows_child_process_path(cwd));
    }

    let plan = resolve_process_plan_for_platform(
        process_plan_request(
            &config.command,
            &config.args,
            config.cwd.as_deref(),
            &config.env,
            &config.remove_env,
        ),
        ProcessPlatform::Windows,
    )?;
    let pty_wrapper = pty_wrapper_from_resolved_plan(&plan);
    apply_resolved_plan(&mut config, plan);

    if let Some((args, expression)) = pty_wrapper {
        config.args = args;
        config
            .env
            .insert(WINDOWS_CMD_WRAPPER_EXPRESSION_ENV.to_string(), expression);
    }

    Ok(config)
}

pub(super) fn normalize_host_shell_command(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Result<(String, Vec<String>), ProcessResolveFailure> {
    let command = normalize_command_token(command);
    let plan = resolve_process_plan_for_platform(
        process_plan_request(&command, args, None, env, remove_env),
        ProcessPlatform::Windows,
    )?;
    Ok((
        plan.program.to_string_lossy().into_owned(),
        os_args_to_strings(&plan.args),
    ))
}

fn process_plan_request(
    command: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> ProcessPlanRequest {
    let mut request = ProcessPlanRequest::new(command).args(args);
    if let Some(cwd) = cwd {
        request = request.current_dir(cwd);
    }
    for (key, value) in env {
        request = request.env(key, value);
    }
    for key in remove_env {
        request = request.env_remove(key);
    }
    request
}

fn apply_resolved_plan(config: &mut SpawnConfig, plan: ResolvedProcessPlan) {
    config.command = plan.program.to_string_lossy().into_owned();
    config.args = os_args_to_strings(&plan.args);
    config.cwd = plan.cwd;
    config.env = plan
        .env
        .into_iter()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect();
    config.remove_env = plan
        .remove_env
        .into_iter()
        .map(|key| key.to_string_lossy().into_owned())
        .collect();
}

fn os_args_to_strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
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

fn pty_wrapper_from_resolved_plan(plan: &ResolvedProcessPlan) -> Option<(Vec<String>, String)> {
    let [slash_d, slash_v, slash_c, expression_ref] = plan.args.as_slice() else {
        return None;
    };
    let expected_ref = format!("%{WINDOWS_CMD_WRAPPER_EXPRESSION_ENV}%");
    if !slash_d.eq_ignore_ascii_case("/d")
        || !slash_v.eq_ignore_ascii_case("/v:off")
        || !slash_c.eq_ignore_ascii_case("/c")
        || expression_ref != OsStr::new(&expected_ref)
    {
        return None;
    }

    let expression = plan
        .env
        .iter()
        .rev()
        .find(|(key, _)| key.eq_ignore_ascii_case(WINDOWS_CMD_WRAPPER_EXPRESSION_ENV))
        .map(|(_, value)| format!("{} & exit", value.to_string_lossy()))?;
    Some((
        vec![
            "/d".to_string(),
            "/v:off".to_string(),
            "/k".to_string(),
            expected_ref,
        ],
        expression,
    ))
}

pub(super) fn reject_non_pe_executable(command: &str) -> Option<String> {
    resolve_process_plan_for_platform(
        ProcessPlanRequest::new(normalize_command_token(command)),
        ProcessPlatform::Windows,
    )
    .err()
    .filter(|failure| failure.kind == ProcessResolveFailureKind::UnsafeExecutable)
    .map(|failure| failure.message)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::PathBuf};

    use super::*;
    #[cfg(windows)]
    use crate::pty::PtyHandle;

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
        .expect("normalize spawn config")
    }

    fn write_valid_pe(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create PE parent");
        }
        #[cfg(windows)]
        {
            fs::copy(
                std::env::current_exe().expect("current test executable"),
                path,
            )
            .expect("copy real PE fixture");
            return;
        }
        #[cfg(not(windows))]
        {
            let mut bytes = vec![0_u8; 0x1b0];
            bytes[0..2].copy_from_slice(b"MZ");
            bytes[0x3c..0x40].copy_from_slice(&(0x80_u32).to_le_bytes());
            bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
            bytes[0x84..0x86].copy_from_slice(&0x8664_u16.to_le_bytes());
            bytes[0x86..0x88].copy_from_slice(&1_u16.to_le_bytes());
            bytes[0x94..0x96].copy_from_slice(&0x00f0_u16.to_le_bytes());
            bytes[0x96..0x98].copy_from_slice(&0x0022_u16.to_le_bytes());
            bytes[0x98..0x9a].copy_from_slice(&0x020b_u16.to_le_bytes());
            fs::write(path, bytes).expect("write PE fixture");
        }
    }

    fn windows_path(paths: &[&Path]) -> String {
        paths
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join(";")
    }

    #[test]
    fn shared_resolver_failure_is_returned_before_pty_mapping() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stub = temp.path().join("claude.exe");
        fs::write(&stub, "Error: native binary not installed\n").expect("stub");

        let result = normalize_spawn_config(SpawnConfig {
            command: stub.display().to_string(),
            args: vec!["--version".to_string()],
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        });

        let error = match result {
            Ok(_) => panic!("unsafe executable must fail before PTY mapping"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("native-binary placeholder"),
            "expected the shared resolver diagnostic, got: {error}"
        );
    }

    #[test]
    fn strips_windows_verbatim_cwd_before_spawn() {
        let temp = tempfile::tempdir().expect("tempdir");
        let command = temp.path().join("tool.exe");
        write_valid_pe(&command);
        let normalized = normalize_spawn_config(SpawnConfig {
            command: command.display().to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(PathBuf::from(
                r"Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\20260525-0919",
            )),
        })
        .expect("normalize spawn config");

        assert_eq!(
            normalized.cwd,
            Some(PathBuf::from(r"E:\gwt\work\20260525-0919"))
        );
    }

    #[test]
    fn wraps_cmd_shims_with_comspec() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        let cmd = bin_dir.join("claude.cmd");
        fs::write(&cmd, "@echo off\r\n").expect("cmd");
        let comspec = temp.path().join("Windows").join("System32").join("cmd.exe");
        write_valid_pe(&comspec);

        let env = HashMap::from([
            ("PATH".to_string(), bin_dir.display().to_string()),
            ("PATHEXT".to_string(), ".CMD".to_string()),
            ("ComSpec".to_string(), comspec.display().to_string()),
        ]);
        let normalized = normalized_config(
            "claude",
            vec!["--dangerously-skip-permissions".to_string()],
            env,
        );

        assert_eq!(normalized.command, comspec.display().to_string());
        assert_eq!(
            normalized.args,
            vec![
                "/d".to_string(),
                "/v:off".to_string(),
                "/k".to_string(),
                format!("%{WINDOWS_CMD_WRAPPER_EXPRESSION_ENV}%"),
            ]
        );
        assert_eq!(
            normalized.env.get(WINDOWS_CMD_WRAPPER_EXPRESSION_ENV),
            Some(&format!(
                "\"{}\" \"--dangerously-skip-permissions\" & exit",
                cmd.display()
            ))
        );
    }

    #[test]
    fn rejects_unsafe_comspec_added_by_the_pty_wrapper() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::write(bin_dir.join("claude.cmd"), "@echo off\r\n").expect("cmd shim");
        let unsafe_comspec = temp.path().join("cmd.exe");
        fs::write(&unsafe_comspec, "not a PE image\n").expect("unsafe ComSpec");

        let env = HashMap::from([
            ("PATH".to_string(), bin_dir.display().to_string()),
            ("PATHEXT".to_string(), ".CMD".to_string()),
            ("ComSpec".to_string(), unsafe_comspec.display().to_string()),
        ]);
        let result = normalize_spawn_config(SpawnConfig {
            command: "claude".to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        });

        assert!(
            result.is_err(),
            "PTY wrapper must resolve and validate ComSpec"
        );
    }

    #[test]
    fn prefers_valid_exe_before_extensionless_shim() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::write(bin_dir.join("codex"), "#!/bin/sh\n").expect("shim");
        let exe = bin_dir.join("codex.exe");
        write_valid_pe(&exe);

        let env = HashMap::from([
            ("PATH".to_string(), bin_dir.display().to_string()),
            ("PATHEXT".to_string(), ".EXE;.CMD".to_string()),
        ]);
        let normalized = normalized_config("codex", vec!["--no-alt-screen".to_string()], env);

        assert_eq!(normalized.command, exe.display().to_string());
        assert_eq!(normalized.args, vec!["--no-alt-screen".to_string()]);
    }

    #[test]
    fn preserves_resolved_program_prefix_and_process_contract() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let script = bin_dir
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin")
            .join("codex.js");
        fs::create_dir_all(script.parent().expect("script parent")).expect("node modules");
        let node_exe = bin_dir.join("node.exe");
        write_valid_pe(&node_exe);
        fs::write(&script, "console.log('codex');\n").expect("script");
        fs::write(
            bin_dir.join("codex"),
            "#!/bin/sh\nexec \"$basedir/node.exe\" \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\n",
        )
        .expect("shim");

        let env = HashMap::from([
            ("PATH".to_string(), bin_dir.display().to_string()),
            ("PATHEXT".to_string(), ".EXE;.CMD".to_string()),
            ("GWT_TEST_VALUE".to_string(), "preserved".to_string()),
        ]);
        let normalized = normalize_spawn_config(SpawnConfig {
            command: "codex".to_string(),
            args: vec!["--no-alt-screen".to_string()],
            cols: 100,
            rows: 30,
            env,
            remove_env: vec!["GWT_REMOVED_VALUE".to_string()],
            cwd: None,
        })
        .expect("normalize spawn config");

        assert_eq!(normalized.command, node_exe.display().to_string());
        assert_eq!(
            normalized.args,
            vec![script.display().to_string(), "--no-alt-screen".to_string()]
        );
        assert_eq!(
            normalized.env.get("GWT_TEST_VALUE"),
            Some(&"preserved".to_string())
        );
        assert_eq!(normalized.remove_env, vec!["GWT_REMOVED_VALUE".to_string()]);
        assert_eq!((normalized.cols, normalized.rows), (100, 30));
    }

    fn bun_claude_fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let bun_bin = root.join("ユーザー 太郎").join(".bun").join("bin");
        let package = root
            .join("ユーザー 太郎")
            .join(".bun")
            .join("install")
            .join("global")
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        let package_bin = package.join("bin");
        fs::create_dir_all(&bun_bin).expect("bun bin");
        fs::create_dir_all(&package_bin).expect("package bin");
        fs::write(
            package.join("package.json"),
            r#"{"bin":{"claude":"bin/claude.exe"}}"#,
        )
        .expect("package json");
        let bun = bun_bin.join("bun.exe");
        write_valid_pe(&bun);
        write_valid_pe(&bun_bin.join("claude.exe"));
        fs::write(
            package_bin.join("claude.exe"),
            "Error: native binary not installed\n",
        )
        .expect("placeholder");
        let wrapper = package.join("cli-wrapper.cjs");
        fs::write(&wrapper, "console.log('wrapper');\n").expect("wrapper");
        (bun_bin, bun, wrapper)
    }

    #[test]
    fn pty_and_host_shell_consume_the_same_shared_plan() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (bun_bin, bun, wrapper) = bun_claude_fixture(temp.path());
        let env = HashMap::from([
            ("PATH".to_string(), windows_path(&[&bun_bin])),
            ("PATHEXT".to_string(), ".EXE;.CMD".to_string()),
        ]);

        let pty = normalized_config("claude", vec!["--version".to_string()], env.clone());
        let host = normalize_host_shell_command("claude", &["--version".to_string()], &env, &[])
            .expect("normalize host-shell command");

        assert_eq!(pty.command, bun.display().to_string());
        assert_eq!(
            pty.args,
            vec![wrapper.display().to_string(), "--version".to_string()]
        );
        assert_eq!(host, (pty.command, pty.args));
    }

    #[test]
    fn reject_non_pe_executable_delegates_to_shared_pe_validation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stub = temp.path().join("claude.exe");
        fs::write(&stub, "native binary not installed\n").expect("stub");
        let corrupt = temp.path().join("corrupt.exe");
        fs::write(&corrupt, b"MZ not really a PE image").expect("corrupt");
        let real = temp.path().join("real.exe");
        write_valid_pe(&real);

        let placeholder_error = reject_non_pe_executable(stub.to_string_lossy().as_ref())
            .expect("placeholder must be rejected");
        assert!(
            placeholder_error.contains("native-binary placeholder without a safe wrapper"),
            "{placeholder_error}"
        );
        assert!(reject_non_pe_executable(corrupt.to_string_lossy().as_ref()).is_some());
        assert!(reject_non_pe_executable(real.to_string_lossy().as_ref()).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn spawn_rejects_non_pe_placeholder_with_actionable_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stub = temp.path().join("claude.exe");
        fs::write(&stub, "Error: native binary not installed\n").expect("stub");

        let result = PtyHandle::spawn(SpawnConfig {
            command: stub.display().to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        });
        let message = match result {
            Ok(_) => panic!("spawning a non-PE placeholder stub must fail"),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains("native-binary placeholder without a safe wrapper"),
            "{message}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn pty_spawns_a_real_cmd_shim_from_a_spaced_path_with_quoted_arguments() {
        use std::time::Duration;

        use crate::test_util::{answer_cursor_position_query, lock_pty_test, read_until_contains};

        let _lock = lock_pty_test();
        let temp = tempfile::tempdir().expect("tempdir");
        let bin = temp.path().join("Program Files").join("npm bin");
        fs::create_dir_all(&bin).expect("create cmd shim directory");
        let shim = bin.join("npx.cmd");
        fs::write(
            &shim,
            "@echo off\r\necho GWT_ARG1:\"%~1\"\r\necho GWT_ARG2:\"%~2\"\r\necho GWT_ARG3:\"%~3\"\r\n",
        )
        .expect("write executable cmd shim");
        let comspec = std::env::var("ComSpec").expect("Windows ComSpec");
        let env = HashMap::from([
            ("PATH".to_string(), bin.display().to_string()),
            ("PATHEXT".to_string(), ".CMD".to_string()),
            ("ComSpec".to_string(), comspec),
        ]);

        let handle = PtyHandle::spawn(SpawnConfig {
            command: "npx".to_string(),
            args: vec![
                "a&b".to_string(),
                "%PATH%".to_string(),
                "a!GWT_UNDEFINED!b".to_string(),
            ],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        })
        .expect("spawn real cmd shim through PTY");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("PTY reader");
        let output = read_until_contains(reader, Duration::from_secs(5), "GWT_ARG3")
            .expect("read cmd shim output");
        let output = String::from_utf8_lossy(&output);

        assert!(output.contains("GWT_ARG1:\"a&b\""), "{output:?}");
        assert!(output.contains("GWT_ARG2:\"%PATH%\""), "{output:?}");
        assert!(
            output.contains("GWT_ARG3:\"a!GWT_UNDEFINED!b\""),
            "{output:?}"
        );
    }
}
