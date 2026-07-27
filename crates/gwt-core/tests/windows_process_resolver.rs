use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use gwt_core::process::{
    resolve_process_plan_for_platform, ProcessPlanRequest, ProcessPlatform,
    ProcessResolveFailureKind,
};
use tempfile::TempDir;

struct BunClaudeFixture {
    _temp: TempDir,
    bun_bin: PathBuf,
    bun_exe: PathBuf,
    wrapper: PathBuf,
    placeholder: PathBuf,
    native: PathBuf,
}

impl BunClaudeFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create fixture root");
        let profile = temp.path().join("ユーザー 太郎");
        let bun_bin = profile.join(".bun").join("bin");
        let package = profile
            .join(".bun")
            .join("install")
            .join("global")
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        let package_bin = package.join("bin");
        let native = package
            .parent()
            .expect("scoped package parent")
            .join("claude-code-win32-x64")
            .join("claude.exe");

        fs::create_dir_all(&bun_bin).expect("create bun bin");
        fs::create_dir_all(&package_bin).expect("create package bin");
        fs::write(
            package.join("package.json"),
            r#"{"name":"@anthropic-ai/claude-code","bin":{"claude":"bin/claude.exe"}}"#,
        )
        .expect("write package manifest");

        let bun_exe = bun_bin.join("bun.exe");
        write_valid_pe(&bun_exe);
        write_valid_pe(&bun_bin.join("claude.exe"));

        let placeholder = package_bin.join("claude.exe");
        fs::write(
            &placeholder,
            b"echo Native binary not installed. Run the package postinstall.\r\n",
        )
        .expect("write placeholder");

        let wrapper = package.join("cli-wrapper.cjs");
        fs::write(&wrapper, b"console.log('claude wrapper');\n").expect("write wrapper");

        Self {
            _temp: temp,
            bun_bin,
            bun_exe,
            wrapper,
            placeholder,
            native,
        }
    }

    fn request(&self) -> ProcessPlanRequest {
        ProcessPlanRequest::new("claude")
            .arg("--version")
            .inherit_env(false)
            .env("PATH", windows_path(&[&self.bun_bin]))
            .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
    }
}

fn windows_path(paths: &[&Path]) -> OsString {
    OsString::from(
        paths
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join(";"),
    )
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

#[test]
fn windows_resolver_redirects_unicode_bun_placeholder_to_cli_wrapper() {
    let fixture = BunClaudeFixture::new();

    let plan = resolve_process_plan_for_platform(fixture.request(), ProcessPlatform::Windows)
        .expect("placeholder must resolve to a safe launcher");

    assert_eq!(plan.program, fixture.bun_exe);
    assert_eq!(
        plan.args,
        vec![
            fixture.wrapper.into_os_string(),
            OsString::from("--version")
        ]
    );
}

#[test]
fn windows_resolver_finds_command_in_quoted_path_entry() {
    let temp = tempfile::tempdir().expect("create fixture root");
    let quoted_dir = temp.path().join("Program; Files").join("Bun");
    let executable = quoted_dir.join("claude.exe");
    write_valid_pe(&executable);

    let request = ProcessPlanRequest::new("claude")
        .inherit_env(false)
        .env("PATH", format!(r#""{}";C:\missing"#, quoted_dir.display()))
        .env("PATHEXT", ".EXE");
    let plan = resolve_process_plan_for_platform(request, ProcessPlatform::Windows)
        .expect("quoted Windows PATH entry must resolve");

    assert_eq!(plan.program, executable);
}

#[test]
fn windows_resolver_redirects_placeholder_to_optional_native() {
    let fixture = BunClaudeFixture::new();
    fs::remove_file(&fixture.wrapper).expect("remove wrapper");
    write_valid_pe(&fixture.native);

    let plan = resolve_process_plan_for_platform(fixture.request(), ProcessPlatform::Windows)
        .expect("placeholder must resolve to optional native binary");

    assert_eq!(plan.program, fixture.native);
    assert_eq!(plan.args, vec![OsString::from("--version")]);
}

#[test]
fn windows_resolver_rejects_placeholder_without_safe_target() {
    let fixture = BunClaudeFixture::new();
    fs::remove_file(&fixture.wrapper).expect("remove wrapper");

    let error = resolve_process_plan_for_platform(fixture.request(), ProcessPlatform::Windows)
        .expect_err("unsafe placeholder must fail before CreateProcess");

    assert_eq!(error.kind, ProcessResolveFailureKind::UnsafeExecutable);
    assert_eq!(
        error.candidate.as_deref(),
        Some(fixture.placeholder.as_path())
    );
    assert!(error.message.contains("native-binary placeholder"));
}

#[test]
fn windows_resolver_rejects_mz_only_corrupt_image() {
    let temp = tempfile::tempdir().expect("create fixture root");
    let corrupt = temp.path().join("claude.exe");
    fs::write(&corrupt, b"MZ not really a PE image").expect("write corrupt image");

    let request = ProcessPlanRequest::new(corrupt.as_os_str()).arg("--version");
    let error = resolve_process_plan_for_platform(request, ProcessPlatform::Windows)
        .expect_err("MZ-only image must fail before CreateProcess");

    assert_eq!(error.kind, ProcessResolveFailureKind::UnsafeExecutable);
    assert_eq!(error.candidate.as_deref(), Some(corrupt.as_path()));
}

#[test]
fn windows_resolver_rejects_signature_only_corrupt_image() {
    let temp = tempfile::tempdir().expect("create fixture root");
    let corrupt = temp.path().join("claude.exe");
    let mut bytes = vec![0_u8; 0x88];
    bytes[0..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&(0x80_u32).to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    fs::write(&corrupt, bytes).expect("write signature-only image");

    let request = ProcessPlanRequest::new(corrupt.as_os_str()).arg("--version");
    let error = resolve_process_plan_for_platform(request, ProcessPlatform::Windows)
        .expect_err("a PE signature without structural headers must be rejected");

    assert_eq!(error.kind, ProcessResolveFailureKind::UnsafeExecutable);
    assert_eq!(error.candidate.as_deref(), Some(corrupt.as_path()));
}

#[test]
fn windows_resolver_preserves_a_valid_pe_executable() {
    let temp = tempfile::tempdir().expect("create fixture root");
    let executable = temp.path().join("claude.exe");
    write_valid_pe(&executable);
    let request = ProcessPlanRequest::new("claude")
        .arg("--version")
        .inherit_env(false)
        .env("PATH", windows_path(&[temp.path()]))
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD");

    let plan = resolve_process_plan_for_platform(request, ProcessPlatform::Windows)
        .expect("valid PE must remain launchable");

    assert_eq!(plan.program, executable);
    assert_eq!(plan.args, vec![OsString::from("--version")]);
}

#[test]
fn windows_resolver_rewrites_an_npm_cmd_shim_to_runtime_and_script() {
    let temp = tempfile::tempdir().expect("create fixture root");
    let bin = temp.path().join("npm bin");
    let script = bin
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("cli.js");
    fs::create_dir_all(script.parent().expect("script parent")).expect("create script parent");
    fs::write(&script, b"console.log('claude');\n").expect("write script");
    let node = bin.join("node.exe");
    write_valid_pe(&node);
    fs::write(
        bin.join("claude.cmd"),
        r#"@"%dp0%\node.exe" "%dp0%\node_modules\@anthropic-ai\claude-code\cli.js" %*"#,
    )
    .expect("write npm cmd shim");
    let request = ProcessPlanRequest::new("claude")
        .arg("--version")
        .inherit_env(false)
        .env("PATH", windows_path(&[&bin]))
        .env("PATHEXT", ".CMD;.EXE");

    let plan = resolve_process_plan_for_platform(request, ProcessPlatform::Windows)
        .expect("npm cmd shim must resolve to node plus script");

    assert_eq!(plan.program, node);
    assert_eq!(
        plan.args,
        vec![script.into_os_string(), OsString::from("--version")]
    );
}

#[test]
fn windows_resolver_wraps_an_opaque_cmd_shim_with_validated_comspec() {
    let temp = tempfile::tempdir().expect("create fixture root");
    let bin = temp.path().join("npm bin");
    fs::create_dir_all(&bin).expect("create bin");
    let shim = bin.join("runner.cmd");
    fs::write(&shim, "@echo off\r\necho opaque shim\r\n").expect("write cmd shim");
    let comspec = temp.path().join("Windows").join("System32").join("cmd.exe");
    write_valid_pe(&comspec);
    let request = ProcessPlanRequest::new("runner")
        .args(["first value", "a&b"])
        .inherit_env(false)
        .env("PATH", windows_path(&[&bin]))
        .env("PATHEXT", ".CMD")
        .env("ComSpec", &comspec)
        .env(
            "gwt_windows_cmd_wrapper_expression",
            "caller-controlled expression",
        );

    let plan = resolve_process_plan_for_platform(request, ProcessPlatform::Windows)
        .expect("opaque cmd shims must become a spawn-ready ComSpec plan");

    assert_eq!(plan.program, comspec);
    assert_eq!(
        plan.args,
        vec![
            OsString::from("/D"),
            OsString::from("/V:OFF"),
            OsString::from("/C"),
            OsString::from("%GWT_WINDOWS_CMD_WRAPPER_EXPRESSION%"),
        ]
    );
    let wrapper_env = plan
        .env
        .iter()
        .filter(|(key, _)| {
            key.to_string_lossy()
                .eq_ignore_ascii_case("GWT_WINDOWS_CMD_WRAPPER_EXPRESSION")
        })
        .collect::<Vec<_>>();
    assert_eq!(wrapper_env.len(), 1, "{wrapper_env:?}");
    assert_eq!(
        wrapper_env[0],
        &(
            OsString::from("GWT_WINDOWS_CMD_WRAPPER_EXPRESSION"),
            OsString::from(format!("\"{}\" \"first value\" \"a&b\"", shim.display())),
        )
    );
}

#[test]
fn windows_resolver_redirects_an_npm_cmd_placeholder_to_wrapper() {
    let temp = tempfile::tempdir().expect("create fixture root");
    let npm_bin = temp.path().join("npm global");
    let package = npm_bin
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-code");
    let placeholder = package.join("bin").join("claude.exe");
    fs::create_dir_all(placeholder.parent().expect("placeholder parent"))
        .expect("create placeholder parent");
    fs::write(
        package.join("package.json"),
        r#"{"name":"@anthropic-ai/claude-code","bin":{"claude":"bin/claude.exe"}}"#,
    )
    .expect("write package manifest");
    fs::write(&placeholder, b"Error: native binary not installed\r\n").expect("write placeholder");
    let wrapper = package.join("cli-wrapper.cjs");
    fs::write(&wrapper, b"console.log('wrapper');\n").expect("write wrapper");
    fs::write(
        npm_bin.join("claude.cmd"),
        r#"@"%dp0%\node_modules\@anthropic-ai\claude-code\bin\claude.exe" %*"#,
    )
    .expect("write npm cmd shim");
    let runtime_bin = temp.path().join("runtime");
    let node = runtime_bin.join("node.exe");
    write_valid_pe(&node);
    let request = ProcessPlanRequest::new("claude")
        .arg("--version")
        .inherit_env(false)
        .env("PATH", windows_path(&[&npm_bin, &runtime_bin]))
        .env("PATHEXT", ".CMD;.EXE");

    let plan = resolve_process_plan_for_platform(request, ProcessPlatform::Windows)
        .expect("npm placeholder must redirect to a safe wrapper");

    assert_eq!(plan.program, node);
    assert_eq!(
        plan.args,
        vec![wrapper.into_os_string(), OsString::from("--version")]
    );
}

#[test]
fn non_windows_resolver_preserves_program_args_and_environment_contract() {
    let request = ProcessPlanRequest::new("claude")
        .arg("--version")
        .current_dir("workspace")
        .env("PATH", "custom-path")
        .env_remove("CLAUDE_CONFIG_DIR");

    let plan = resolve_process_plan_for_platform(request, ProcessPlatform::Posix)
        .expect("non-Windows resolution must remain an identity transform");

    assert_eq!(plan.program, PathBuf::from("claude"));
    assert_eq!(plan.args, vec![OsString::from("--version")]);
    assert_eq!(plan.cwd, Some(PathBuf::from("workspace")));
    assert_eq!(
        plan.env,
        vec![(OsString::from("PATH"), OsString::from("custom-path"))]
    );
    assert_eq!(plan.remove_env, vec![OsString::from("CLAUDE_CONFIG_DIR")]);
}
