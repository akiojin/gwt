//! Match gwt's JavaScript verification runtime to its frontend CI job.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

pub(super) fn prepare(
    worktree: &Path,
    args: &mut [String],
    assignments: &mut Vec<(String, String)>,
    headed: bool,
) -> Result<(), String> {
    if !super::is_gwt_checkout(worktree) || !needs_node(worktree, args, headed) {
        return Ok(());
    }
    // Read the existing CI contract rather than maintain a second version pin.
    let policy = worktree.join(".github/workflows/test.yml");
    let major = ci_major(&policy)?;
    let path = assignments
        .iter()
        .rev()
        .find(|(key, _)| is_path(key))
        .map(|(_, value)| std::ffi::OsString::from(value))
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let mut directories: Vec<PathBuf> = std::env::split_paths(&path)
        .map(|directory| {
            if directory.is_absolute() {
                directory
            } else {
                worktree.join(directory)
            }
        })
        .collect();
    // Homebrew's versioned formula is keg-only. nvm also keeps inactive
    // versions outside PATH; neither requires mutating the caller's shell.
    for prefix in ["/opt/homebrew/opt", "/usr/local/opt"] {
        directories.push(Path::new(prefix).join(format!("node@{major}/bin")));
    }
    if let Some(nvm) = std::env::var_os("NVM_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".nvm")))
    {
        if let Ok(versions) = std::fs::read_dir(nvm.join("versions/node")) {
            directories.extend(versions.flatten().map(|entry| entry.path().join("bin")));
        }
    }
    let executable = if cfg!(windows) { "node.exe" } else { "node" };
    let selected = directories.iter().map(|bin| bin.join(executable)).find(|node| {
        if !node.is_file() { return false; }
        // A caller's preload must not turn this version probe into an E2E run.
        gwt_core::process::hidden_command(node).arg("--version")
            .env_remove("NODE_OPTIONS").stdin(Stdio::null()).output()
            .ok().filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .is_some_and(|version| version.trim().strip_prefix('v')
                .and_then(|version| version.split('.').next())
                .and_then(|major| major.parse::<u32>().ok()) == Some(major))
    }).ok_or_else(|| format!(
        "Node.js {major}.x is required by {} (test-frontend); no matching installed runtime was found. Install it with `nvm install {major}` or `brew install node@{major}` (Windows: install Node.js {major}.x), then retry verify.run. Other installations must expose their node directory on PATH. Host Node is not a fallback.", policy.display()
    ))?;
    let bin = selected.parent().expect("node has a bin directory");
    let child_path = std::env::join_paths(
        std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&path)),
    )
    .map_err(|error| error.to_string())?
    .into_string()
    .map_err(|_| "Node runtime PATH is not UTF-8".to_string())?;
    assignments.retain(|(key, _)| !is_path(key));
    assignments.push(("PATH".into(), child_path));
    // Absolute node commands must use the selected runtime as well. Wrappers
    // and /usr/bin/env shebangs resolve it from the command-local PATH.
    if Path::new(&args[0])
        .file_stem()
        .and_then(|stem| stem.to_str())
        == Some("node")
    {
        args[0] = selected
            .to_str()
            .ok_or("Node runtime path is not UTF-8")?
            .to_string();
    }
    Ok(())
}

fn is_path(key: &str) -> bool {
    if cfg!(windows) {
        key.eq_ignore_ascii_case("PATH")
    } else {
        key == "PATH"
    }
}

fn ci_major(policy: &Path) -> Result<u32, String> {
    let parse = || -> Option<u32> {
        let document: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(policy).ok()?).ok()?;
        let steps = document["jobs"]["test-frontend"]["steps"].as_sequence()?;
        let step = steps.iter().find(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("actions/setup-node@"))
        })?;
        let version = &step["with"]["node-version"];
        version
            .as_u64()
            .and_then(|major| u32::try_from(major).ok())
            .or_else(|| version.as_str()?.split('.').next()?.parse().ok())
            .filter(|major| *major > 0)
    };
    parse().ok_or_else(|| format!(
        "Cannot determine the verification Node.js major from {}: restore test-frontend's actions/setup-node node-version, then retry verify.run.", policy.display()
    ))
}

fn needs_node(worktree: &Path, args: &[String], headed: bool) -> bool {
    let program = Path::new(&args[0])
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    if headed
        || matches!(
            program,
            "node" | "npm" | "npx" | "pnpm" | "yarn" | "corepack" | "bun" | "bunx" | "playwright"
        )
    {
        return true;
    }
    // Repository shell wrappers may launch Node indirectly (frontend unit
    // tests call run-node-tests-with-linkedom.sh, for example).
    matches!(program, "bash" | "sh" | "pwsh" | "powershell")
        && args.iter().skip(1).any(|arg| {
            matches!(
                Path::new(arg).extension().and_then(|ext| ext.to_str()),
                Some("sh" | "ps1")
            ) && std::fs::read_to_string(worktree.join(arg)).is_ok_and(|source| {
                ["node", "npm", "npx", "playwright"]
                    .iter()
                    .any(|name| source.contains(name))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout(root: &Path, major: u32) {
        std::fs::create_dir_all(root.join("crates/gwt")).unwrap();
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=['crates/gwt']\n",
        )
        .unwrap();
        std::fs::write(root.join("crates/gwt/Cargo.toml"), "[package]\nname='gwt'\n[features]\ntest-gh-guard=[]\n[[bin]]\nname='gwtd'\npath='src/bin/gwtd.rs'\n").unwrap();
        std::fs::write(root.join(".github/workflows/test.yml"), format!("jobs:\n  test-frontend:\n    steps:\n      - uses: actions/setup-node@v7\n        with:\n          node-version: {major}\n")).unwrap();
    }

    #[cfg(unix)]
    fn node(root: &Path, major: u32) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join(format!("node-{major}/bin"));
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("node");
        std::fs::write(&exe, format!("#!/bin/sh\necho v{major}.1.0\n")).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    // AC-1: an incompatible host entry must not win, including shell wrappers
    // nominated for headed measurement. Both spawn routes consume this result.
    #[cfg(unix)]
    #[test]
    fn ci_node_replaces_host_runtime_in_inherited_and_daemon_children() {
        let root = tempfile::tempdir().unwrap();
        checkout(root.path(), 22);
        let host = node(root.path(), 26);
        let ci = node(root.path(), 22);
        let path = std::env::join_paths([&host, &ci])
            .unwrap()
            .into_string()
            .unwrap();
        std::fs::create_dir_all(root.path().join("scripts")).unwrap();
        std::fs::write(root.path().join("scripts/unit.sh"), "node --version\n").unwrap();
        for (command, headed) in [
            ("node --version", false),
            ("bash scripts/unit.sh", false),
            ("bash wrapper.sh", true),
        ] {
            let mut args = super::super::split_command_line(command).unwrap();
            let mut env = vec![("PATH".into(), path.clone())];
            prepare(root.path(), &mut args, &mut env, headed).unwrap();
            let child_path = &env.last().unwrap().1;
            assert_eq!(std::env::split_paths(child_path).next().unwrap(), ci);
            let request = super::super::delegated_spawn_request(
                root.path(),
                &args,
                &env,
                false,
                None,
                root.path().join("out"),
                root.path().join("err"),
            );
            assert!(request.env.contains(&("PATH".into(), child_path.clone())));
            if command.starts_with("node ") {
                assert_eq!(args[0], ci.join("node").to_str().unwrap());
                let (code, _, output, _) = super::super::execute_command_with_isolation(
                    root.path(),
                    &format!("PATH='{path}' node --version"),
                    false,
                    None,
                    &super::super::VerificationHost::Inherit,
                    None,
                )
                .unwrap();
                assert_eq!(code, 0, "{output}");
                assert!(output.contains("v22.1.0"), "{output}");
            }
        }
    }

    #[test]
    fn missing_ci_node_fails_with_required_major_and_installation_help() {
        let root = tempfile::tempdir().unwrap();
        checkout(root.path(), 999);
        let mut args = vec!["node".into(), "--version".into()];
        let mut env = vec![("PATH".into(), String::new())];
        let error = prepare(root.path(), &mut args, &mut env, false).unwrap_err();
        assert!(error.contains("Node.js 999"), "{error}");
        assert!(error.contains("nvm install 999"), "{error}");
        assert!(error.contains("PATH"), "{error}");
    }

    #[test]
    fn unrelated_projects_and_rust_commands_keep_their_environment() {
        let root = tempfile::tempdir().unwrap();
        let mut args = vec!["node".into(), "--version".into()];
        let mut env = vec![("PATH".into(), "original".into())];
        prepare(root.path(), &mut args, &mut env, true).unwrap();
        checkout(root.path(), 999);
        args[0] = "cargo".into();
        prepare(root.path(), &mut args, &mut env, false).unwrap();
        assert_eq!(env, vec![("PATH".into(), "original".into())]);
    }
}
