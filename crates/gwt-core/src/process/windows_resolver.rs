use std::{
    ffi::{OsStr, OsString},
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Platform whose executable lookup rules should be applied to a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessPlatform {
    Windows,
    Posix,
}

impl ProcessPlatform {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }
}

/// Complete input needed to resolve a subprocess without consulting caller state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessPlanRequest {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
    pub remove_env: Vec<OsString>,
    pub inherit_env: bool,
}

impl ProcessPlanRequest {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: PathBuf::from(program.as_ref()),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            remove_env: Vec::new(),
            inherit_env: true,
        }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    #[must_use]
    pub fn current_dir(mut self, cwd: impl AsRef<Path>) -> Self {
        self.cwd = Some(cwd.as_ref().to_path_buf());
        self
    }

    #[must_use]
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    #[must_use]
    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.remove_env.push(key.as_ref().to_os_string());
        self
    }

    #[must_use]
    pub fn inherit_env(mut self, inherit: bool) -> Self {
        self.inherit_env = inherit;
        self
    }
}

/// Spawn-ready process plan. `args` includes any resolver-owned runtime prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProcessPlan {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
    pub remove_env: Vec<OsString>,
    pub inherit_env: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessResolveFailureKind {
    NotFound,
    UnsafeExecutable,
}

/// Failure detected before a process reaches `CreateProcess`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessResolveFailure {
    pub kind: ProcessResolveFailureKind,
    pub candidate: Option<PathBuf>,
    pub message: String,
}

impl fmt::Display for ProcessResolveFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProcessResolveFailure {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsProcessTarget {
    program: PathBuf,
    args_prefix: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    forward_request_args: bool,
}

pub const WINDOWS_CMD_WRAPPER_EXPRESSION_ENV: &str = "GWT_WINDOWS_CMD_WRAPPER_EXPRESSION";

/// Resolve a request using the current platform's process lookup rules.
pub fn resolve_process_plan(
    request: ProcessPlanRequest,
) -> Result<ResolvedProcessPlan, ProcessResolveFailure> {
    resolve_process_plan_for_platform(request, ProcessPlatform::current())
}

/// Resolve a request using explicit platform rules.
///
/// The explicit form keeps Windows PATH/PATHEXT and PE safety tests runnable on
/// every development host.
pub fn resolve_process_plan_for_platform(
    request: ProcessPlanRequest,
    platform: ProcessPlatform,
) -> Result<ResolvedProcessPlan, ProcessResolveFailure> {
    if platform == ProcessPlatform::Posix {
        return Ok(identity_plan(request));
    }

    let target = resolve_windows_target(&request)?;
    let mut args = target.args_prefix;
    if target.forward_request_args {
        args.extend(request.args.iter().cloned());
    }
    let mut env = request.env;
    env.extend(target.env);
    Ok(ResolvedProcessPlan {
        program: target.program,
        args,
        cwd: request.cwd,
        env,
        remove_env: request.remove_env,
        inherit_env: request.inherit_env,
    })
}

fn identity_plan(request: ProcessPlanRequest) -> ResolvedProcessPlan {
    ResolvedProcessPlan {
        program: request.program,
        args: request.args,
        cwd: request.cwd,
        env: request.env,
        remove_env: request.remove_env,
        inherit_env: request.inherit_env,
    }
}

fn resolve_windows_target(
    request: &ProcessPlanRequest,
) -> Result<WindowsProcessTarget, ProcessResolveFailure> {
    let program = request.program.as_path();
    if program.is_absolute() || has_path_separator(program) {
        return resolve_windows_candidate(program, request)?.ok_or_else(|| not_found(program));
    }

    if let Some(path) = effective_env_value(request, "PATH") {
        for directory in split_windows_paths(&path) {
            let candidate = directory.join(program);
            if let Some(target) = resolve_windows_candidate(&candidate, request)? {
                return Ok(target);
            }
        }
    }

    resolve_windows_candidate(program, request)?.ok_or_else(|| not_found(program))
}

fn resolve_windows_candidate(
    candidate: &Path,
    request: &ProcessPlanRequest,
) -> Result<Option<WindowsProcessTarget>, ProcessResolveFailure> {
    if has_native_extension(candidate) && candidate.is_file() {
        if let Some(target) = resolve_bun_shim(candidate, request)? {
            return validate_target(target, request).map(Some);
        }
        if is_native_placeholder(candidate) {
            if let Some(target) = redirect_native_placeholder(candidate, request)? {
                return validate_target(target, request).map(Some);
            }
            return Err(unsafe_placeholder(candidate));
        }
        validate_pe(candidate)?;
        return Ok(Some(direct_target(candidate)));
    }

    if candidate.extension().is_none() {
        if candidate.is_file() {
            if let Some(target) = parse_npm_shim(candidate, request) {
                return validate_target(target, request).map(Some);
            }
        }
        for extension in windows_path_extensions(request) {
            let with_extension = candidate.with_extension(extension.trim_start_matches('.'));
            if let Some(target) = resolve_windows_candidate(&with_extension, request)? {
                return Ok(Some(target));
            }
        }
    }

    if !candidate.is_file() {
        return Ok(None);
    }
    if let Some(target) = parse_npm_shim(candidate, request) {
        return validate_target(target, request).map(Some);
    }
    if is_cmd_shim(candidate) {
        return resolve_cmd_shim(candidate, request).map(Some);
    }
    Ok(Some(direct_target(candidate)))
}

fn validate_target(
    target: WindowsProcessTarget,
    request: &ProcessPlanRequest,
) -> Result<WindowsProcessTarget, ProcessResolveFailure> {
    if has_native_extension(&target.program) && target.program.is_file() {
        if is_native_placeholder(&target.program) {
            if let Some(redirected) = redirect_native_placeholder(&target.program, request)? {
                return validate_target(redirected, request);
            }
            return Err(unsafe_placeholder(&target.program));
        }
        validate_pe(&target.program)?;
    }
    Ok(target)
}

fn direct_target(program: &Path) -> WindowsProcessTarget {
    WindowsProcessTarget {
        program: program.to_path_buf(),
        args_prefix: Vec::new(),
        env: Vec::new(),
        forward_request_args: true,
    }
}

fn is_cmd_shim(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
}

fn resolve_cmd_shim(
    shim: &Path,
    request: &ProcessPlanRequest,
) -> Result<WindowsProcessTarget, ProcessResolveFailure> {
    let comspec =
        effective_env_value(request, "ComSpec").unwrap_or_else(|| OsString::from("cmd.exe"));
    let comspec_path = PathBuf::from(&comspec);
    if is_cmd_shim(&comspec_path) {
        return Err(unsafe_shell(&comspec_path));
    }

    let mut shell_request = request.clone();
    shell_request.program = comspec_path;
    shell_request.args.clear();
    let shell = resolve_windows_target(&shell_request)?;
    if !shell.args_prefix.is_empty() || !has_native_extension(&shell.program) {
        return Err(unsafe_shell(&shell.program));
    }

    let expression = build_cmd_command_expression(shim, &request.args);
    Ok(WindowsProcessTarget {
        program: shell.program,
        args_prefix: vec![
            OsString::from("/D"),
            OsString::from("/V:OFF"),
            OsString::from("/C"),
            OsString::from(format!("%{WINDOWS_CMD_WRAPPER_EXPRESSION_ENV}%")),
        ],
        env: vec![(
            OsString::from(WINDOWS_CMD_WRAPPER_EXPRESSION_ENV),
            OsString::from(expression),
        )],
        forward_request_args: false,
    })
}

fn build_cmd_command_expression(command: &Path, args: &[OsString]) -> String {
    let mut tokens = Vec::with_capacity(args.len() + 1);
    tokens.push(quote_cmd_token(&command.as_os_str().to_string_lossy()));
    tokens.extend(
        args.iter()
            .map(|arg| quote_cmd_token(&arg.to_string_lossy())),
    );
    tokens.join(" ")
}

fn quote_cmd_token(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn has_path_separator(path: &Path) -> bool {
    let raw = path.as_os_str().to_string_lossy();
    raw.contains('/') || raw.contains('\\')
}

fn effective_env_value(request: &ProcessPlanRequest, key: &str) -> Option<OsString> {
    if let Some((_, value)) = request
        .env
        .iter()
        .rev()
        .find(|(candidate, _)| os_eq_ignore_ascii_case(candidate, key))
    {
        return Some(value.clone());
    }
    if request
        .remove_env
        .iter()
        .any(|candidate| os_eq_ignore_ascii_case(candidate, key))
        || !request.inherit_env
    {
        return None;
    }
    std::env::vars_os()
        .find(|(candidate, _)| os_eq_ignore_ascii_case(candidate, key))
        .map(|(_, value)| value)
}

fn os_eq_ignore_ascii_case(value: &OsStr, expected: &str) -> bool {
    value
        .to_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn split_windows_paths(raw: &OsStr) -> Vec<PathBuf> {
    raw.to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn windows_path_extensions(request: &ProcessPlanRequest) -> Vec<String> {
    effective_env_value(request, "PATHEXT")
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn resolve_bun_shim(
    candidate: &Path,
    request: &ProcessPlanRequest,
) -> Result<Option<WindowsProcessTarget>, ProcessResolveFailure> {
    let Some(command_name) = candidate.file_stem().and_then(OsStr::to_str) else {
        return Ok(None);
    };

    if is_bun_managed_shim(candidate) {
        let Some(package_root) = locate_package_root(candidate) else {
            return Ok(None);
        };
        return resolve_bun_package_command(&package_root, command_name, false, request);
    }

    let Some(node_modules) = bun_global_node_modules_from_bin_shim(candidate) else {
        return Ok(None);
    };
    for package_root in collect_bun_global_package_roots(&node_modules) {
        if let Some(target) =
            resolve_bun_package_command(&package_root, command_name, true, request)?
        {
            return Ok(Some(target));
        }
    }
    Ok(None)
}

fn is_bun_managed_shim(candidate: &Path) -> bool {
    let components: Vec<String> = candidate
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str().map(str::to_ascii_lowercase),
            _ => None,
        })
        .collect();
    components.windows(4).any(|window| {
        window[0] == ".bun"
            && window[1] == "install"
            && window[2] == "global"
            && window[3] == "node_modules"
    })
}

fn bun_global_node_modules_from_bin_shim(candidate: &Path) -> Option<PathBuf> {
    let bin = candidate.parent()?;
    if !file_name_eq(bin, "bin") {
        return None;
    }
    let bun_root = bin.parent()?;
    if !file_name_eq(bun_root, ".bun") {
        return None;
    }
    Some(bun_root.join("install").join("global").join("node_modules"))
}

fn file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn locate_package_root(candidate: &Path) -> Option<PathBuf> {
    let mut current = candidate.parent()?;
    for _ in 0..5 {
        current = current.parent()?;
        if current.join("package.json").is_file() {
            return Some(current.to_path_buf());
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
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if name.starts_with('@') {
            if let Ok(scoped) = std::fs::read_dir(path) {
                roots.extend(
                    scoped
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| path.join("package.json").is_file()),
                );
            }
        } else if path.join("package.json").is_file() {
            roots.push(path);
        }
    }
    roots.sort();
    roots
}

fn resolve_bun_package_command(
    package_root: &Path,
    command_name: &str,
    require_named_bin: bool,
    request: &ProcessPlanRequest,
) -> Result<Option<WindowsProcessTarget>, ProcessResolveFailure> {
    let Some(relative) = package_bin_entry(package_root, command_name, require_named_bin) else {
        return Ok(None);
    };
    let entry = package_root.join(relative);
    if !entry.is_file() {
        return Ok(None);
    }

    if is_native_placeholder(&entry) {
        return redirect_package_placeholder(package_root, command_name, &entry, request)
            .map(Some)
            .ok_or_else(|| unsafe_placeholder(&entry));
    }

    let Some(runtime) = locate_script_runtime(request) else {
        return Ok(None);
    };
    Ok(Some(WindowsProcessTarget {
        program: runtime,
        args_prefix: vec![entry.into_os_string()],
        env: Vec::new(),
        forward_request_args: true,
    }))
}

fn package_bin_entry(
    package_root: &Path,
    command_name: &str,
    require_named_bin: bool,
) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(package_root.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    match json.get("bin")? {
        serde_json::Value::String(value) => {
            if require_named_bin
                && !package_root
                    .file_name()?
                    .to_str()?
                    .eq_ignore_ascii_case(command_name)
            {
                return None;
            }
            Some(PathBuf::from(value.as_str()))
        }
        serde_json::Value::Object(entries) => entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(command_name))
            .or_else(|| {
                (!require_named_bin && entries.len() == 1).then(|| entries.iter().next())?
            })
            .and_then(|(_, value)| value.as_str())
            .map(PathBuf::from),
        _ => None,
    }
}

fn redirect_native_placeholder(
    executable: &Path,
    request: &ProcessPlanRequest,
) -> Result<Option<WindowsProcessTarget>, ProcessResolveFailure> {
    let Some(command_name) = executable.file_stem().and_then(OsStr::to_str) else {
        return Ok(None);
    };
    let Some(package_root) = placeholder_package_root(executable) else {
        return Ok(None);
    };
    Ok(redirect_package_placeholder(
        &package_root,
        command_name,
        executable,
        request,
    ))
}

fn redirect_package_placeholder(
    package_root: &Path,
    command_name: &str,
    _placeholder: &Path,
    request: &ProcessPlanRequest,
) -> Option<WindowsProcessTarget> {
    let wrapper = package_root.join("cli-wrapper.cjs");
    if wrapper.is_file() {
        if let Some(runtime) = locate_script_runtime(request) {
            return Some(WindowsProcessTarget {
                program: runtime,
                args_prefix: vec![wrapper.into_os_string()],
                env: Vec::new(),
                forward_request_args: true,
            });
        }
    }

    let native = optional_windows_native_binary(package_root, command_name)?;
    Some(direct_target(&native))
}

fn placeholder_package_root(executable: &Path) -> Option<PathBuf> {
    let mut current = executable.parent()?;
    for _ in 0..4 {
        if current.join("package.json").is_file() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    executable.parent()?.parent().map(Path::to_path_buf)
}

fn optional_windows_native_binary(package_root: &Path, command_name: &str) -> Option<PathBuf> {
    let package_name = package_root.file_name()?.to_str()?;
    let native = package_root
        .parent()?
        .join(format!("{package_name}-win32-x64"))
        .join(format!("{command_name}.exe"));
    native.is_file().then_some(native)
}

fn locate_script_runtime(request: &ProcessPlanRequest) -> Option<PathBuf> {
    find_on_windows_path("bun.exe", request)
        .or_else(|| {
            effective_env_value(request, "USERPROFILE").and_then(|profile| {
                let candidate = PathBuf::from(profile)
                    .join(".bun")
                    .join("bin")
                    .join("bun.exe");
                candidate.is_file().then_some(candidate)
            })
        })
        .or_else(|| find_on_windows_path("node.exe", request))
}

fn find_on_windows_path(name: &str, request: &ProcessPlanRequest) -> Option<PathBuf> {
    let raw = effective_env_value(request, "PATH")?;
    split_windows_paths(&raw)
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn parse_npm_shim(candidate: &Path, request: &ProcessPlanRequest) -> Option<WindowsProcessTarget> {
    let content = std::fs::read_to_string(candidate).ok()?;
    let marker = match candidate
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("exe" | "com") => return None,
        Some("cmd" | "bat") => "%dp0%\\",
        _ => "$basedir/",
    };
    let paths = collect_marker_paths(&content, marker);
    build_npm_shim_target(candidate.parent()?, &paths, request)
}

fn collect_marker_paths(content: &str, marker: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut remaining = content;
    while let Some(index) = remaining.find(marker) {
        let tail = &remaining[index + marker.len()..];
        let end = tail.find(['"', '\r', '\n']).unwrap_or(tail.len());
        let value = tail[..end].trim();
        if !value.is_empty() {
            paths.push(value.to_string());
        }
        remaining = &tail[end..];
    }
    paths
}

fn build_npm_shim_target(
    base: &Path,
    paths: &[String],
    request: &ProcessPlanRequest,
) -> Option<WindowsProcessTarget> {
    let executable = paths.iter().find_map(|path| {
        let lower = path.to_ascii_lowercase();
        (lower.ends_with(".exe") || lower.ends_with(".com"))
            .then(|| base.join(windows_relative_path(path)))
    });
    let script = paths.iter().find_map(|path| {
        let lower = path.to_ascii_lowercase();
        (lower.ends_with(".js") || lower.ends_with(".cjs"))
            .then(|| base.join(windows_relative_path(path)))
    });

    match (executable, script) {
        (Some(runtime), Some(script)) if is_node_runtime(&runtime) => Some(WindowsProcessTarget {
            program: runtime
                .is_file()
                .then_some(runtime)
                .or_else(|| find_on_windows_path("node.exe", request))?,
            args_prefix: vec![script.into_os_string()],
            env: Vec::new(),
            forward_request_args: true,
        }),
        (Some(runtime), None) if is_node_runtime(&runtime) => None,
        (Some(executable), _) if executable.is_file() => Some(direct_target(&executable)),
        (_, Some(script)) => Some(WindowsProcessTarget {
            program: find_on_windows_path("node.exe", request)?,
            args_prefix: vec![script.into_os_string()],
            env: Vec::new(),
            forward_request_args: true,
        }),
        _ => None,
    }
}

fn windows_relative_path(raw: &str) -> PathBuf {
    raw.split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .collect()
}

fn is_node_runtime(path: &Path) -> bool {
    path.file_stem()
        .and_then(OsStr::to_str)
        .is_some_and(|stem| stem.eq_ignore_ascii_case("node"))
}

fn has_native_extension(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "exe" | "com"))
}

fn is_native_placeholder(path: &Path) -> bool {
    if !has_native_extension(path) {
        return false;
    }
    let Some(prefix) = read_prefix(path, 4096) else {
        return false;
    };
    String::from_utf8_lossy(&prefix)
        .to_ascii_lowercase()
        .contains("native binary not installed")
}

fn validate_pe(candidate: &Path) -> Result<(), ProcessResolveFailure> {
    if is_valid_pe_image(candidate) {
        return Ok(());
    }
    Err(ProcessResolveFailure {
        kind: ProcessResolveFailureKind::UnsafeExecutable,
        candidate: Some(candidate.to_path_buf()),
        message: format!(
            "'{}' is not a valid Windows executable (PE) image and was blocked before launch.",
            candidate.display()
        ),
    })
}

fn is_valid_pe_image(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
        return false;
    };
    if length < 0x40 {
        return false;
    }

    let mut dos_header = [0_u8; 0x40];
    if file.read_exact(&mut dos_header).is_err() || &dos_header[..2] != b"MZ" {
        return false;
    }
    let pe_offset = u64::from(u32::from_le_bytes(
        dos_header[0x3c..0x40].try_into().unwrap(),
    ));
    const COFF_HEADER_SIZE: u64 = 20;
    const SECTION_HEADER_SIZE: u64 = 40;
    let Some(coff_end) = pe_offset
        .checked_add(4)
        .and_then(|offset| offset.checked_add(COFF_HEADER_SIZE))
    else {
        return false;
    };
    if pe_offset < 0x40 || coff_end > length || file.seek(SeekFrom::Start(pe_offset)).is_err() {
        return false;
    }
    let mut signature = [0_u8; 4];
    if file.read_exact(&mut signature).is_err() || &signature != b"PE\0\0" {
        return false;
    }

    let mut coff = [0_u8; COFF_HEADER_SIZE as usize];
    if file.read_exact(&mut coff).is_err() {
        return false;
    }
    let machine = u16::from_le_bytes(coff[0..2].try_into().unwrap());
    let section_count = u64::from(u16::from_le_bytes(coff[2..4].try_into().unwrap()));
    let optional_size = u64::from(u16::from_le_bytes(coff[16..18].try_into().unwrap()));
    let characteristics = u16::from_le_bytes(coff[18..20].try_into().unwrap());
    if !matches!(machine, 0x014c | 0x01c4 | 0x8664 | 0xaa64 | 0xa641 | 0xa64e)
        || section_count == 0
        || characteristics & 0x0002 == 0
    {
        return false;
    }

    let optional_start = coff_end;
    let Some(section_table_start) = optional_start.checked_add(optional_size) else {
        return false;
    };
    let Some(section_table_end) = section_count
        .checked_mul(SECTION_HEADER_SIZE)
        .and_then(|size| section_table_start.checked_add(size))
    else {
        return false;
    };
    if section_table_end > length || file.seek(SeekFrom::Start(optional_start)).is_err() {
        return false;
    }

    let mut optional_magic = [0_u8; 2];
    if file.read_exact(&mut optional_magic).is_err() {
        return false;
    }
    let structurally_valid = match u16::from_le_bytes(optional_magic) {
        0x010b => optional_size >= 0x00e0,
        0x020b => optional_size >= 0x00f0,
        _ => false,
    };
    structurally_valid && windows_loader_accepts_binary(path)
}

#[cfg(windows)]
fn windows_loader_accepts_binary(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetBinaryTypeW};

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut binary_type = 0_u32;
    // GetBinaryTypeW asks the Windows loader to classify the file. Accept only
    // native 32/64-bit images; DOS/WOW/PIF types are intentionally excluded.
    unsafe { GetBinaryTypeW(PCWSTR(wide.as_ptr()), &mut binary_type) }.is_ok()
        && matches!(binary_type, 0 | 6)
}

#[cfg(not(windows))]
fn windows_loader_accepts_binary(_path: &Path) -> bool {
    true
}

fn read_prefix(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut buffer = vec![0_u8; limit];
    let length = file.read(&mut buffer).ok()?;
    buffer.truncate(length);
    Some(buffer)
}

fn unsafe_placeholder(candidate: &Path) -> ProcessResolveFailure {
    ProcessResolveFailure {
        kind: ProcessResolveFailureKind::UnsafeExecutable,
        candidate: Some(candidate.to_path_buf()),
        message: format!(
            "'{}' is a native-binary placeholder without a safe wrapper or optional native binary. Reinstall the package so its postinstall completes.",
            candidate.display()
        ),
    }
}

fn unsafe_shell(candidate: &Path) -> ProcessResolveFailure {
    ProcessResolveFailure {
        kind: ProcessResolveFailureKind::UnsafeExecutable,
        candidate: Some(candidate.to_path_buf()),
        message: format!(
            "ComSpec '{}' did not resolve to a native Windows executable and was blocked before launch.",
            candidate.display()
        ),
    }
}

fn not_found(program: &Path) -> ProcessResolveFailure {
    ProcessResolveFailure {
        kind: ProcessResolveFailureKind::NotFound,
        candidate: None,
        message: format!(
            "'{}' could not be resolved with the effective Windows PATH and PATHEXT.",
            program.display()
        ),
    }
}
