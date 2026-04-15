use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const CLI_VERBS: &[&str] = &["issue", "pr", "actions", "board", "hook"];
const GWT_CLI_BIN_ENV: &str = "GWT_CLI_BIN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliDelegateInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
}

pub fn should_delegate_cli_argv(args: &[String]) -> bool {
    args.get(1)
        .map(|arg| CLI_VERBS.contains(&arg.as_str()))
        .unwrap_or(false)
}

pub fn build_cli_delegate_invocation_from(
    args: &[String],
    current_exe: &Path,
) -> io::Result<Option<CliDelegateInvocation>> {
    if !should_delegate_cli_argv(args) {
        return Ok(None);
    }

    Ok(Some(CliDelegateInvocation {
        program: resolve_canonical_cli_bin_from(current_exe)?,
        args: args.iter().skip(1).cloned().collect(),
    }))
}

pub fn run_cli_delegate_invocation(invocation: &CliDelegateInvocation) -> io::Result<i32> {
    let status = Command::new(&invocation.program)
        .args(&invocation.args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok(status.code().unwrap_or(1))
}

pub fn resolve_canonical_cli_bin_from(current_exe: &Path) -> io::Result<PathBuf> {
    if let Some(path) = configured_cli_bin() {
        return Ok(path);
    }

    if let Some(path) = resolve_binary_near(current_exe, &["gwt", "gwt.exe"]) {
        return Ok(path);
    }

    if let Ok(path) = which::which("gwt") {
        return Ok(path);
    }

    // Development compatibility while the workspace binary is still named
    // `gwt-tui`. The canonical product contract remains `gwt`.
    if let Some(path) = resolve_binary_near(current_exe, &["gwt-tui", "gwt-tui.exe"]) {
        return Ok(path);
    }

    if let Ok(path) = which::which("gwt-tui") {
        return Ok(path);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "failed to locate canonical gwt CLI binary",
    ))
}

fn configured_cli_bin() -> Option<PathBuf> {
    let value = std::env::var_os(GWT_CLI_BIN_ENV)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn resolve_binary_near(current_exe: &Path, names: &[&str]) -> Option<PathBuf> {
    let parent = current_exe.parent()?;
    for dir in parent.ancestors() {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
