fn main() {
    emit_build_stamp();

    #[cfg(all(target_os = "windows", not(test)))]
    embed_windows_resources();
}

/// Stamp the commit and the build time into the binary (SPEC #4249 FR-002).
///
/// A merged fix is not a running fix: the PM reported PR #4246 as live while
/// the installed app predated it by a day. `release.status` reads these two
/// values back so that gap is observable instead of guessed from a file mtime.
///
/// Both are emitted unconditionally — empty when git is unavailable (a source
/// tarball, a vendored build) — so the crate compiles the same way everywhere
/// and the reader treats empty as unknown.
fn emit_build_stamp() {
    let workspace_root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("..");

    let commit = git_output(&workspace_root, &["rev-parse", "HEAD"]).unwrap_or_default();
    println!("cargo:rustc-env=GWT_BUILD_COMMIT={commit}");

    let coverage = std::env::var("CARGO_LLVM_COV").as_deref() == Ok("1");
    let epoch = build_epoch(&workspace_root, coverage, std::time::SystemTime::now());
    println!("cargo:rustc-env=GWT_BUILD_EPOCH={epoch}");
    println!("cargo:rerun-if-env-changed=CARGO_LLVM_COV");
    println!("cargo:rerun-if-env-changed=GWT_COVERAGE_REVISION");

    // CI regenerates this from actual HEAD immediately before Cargo. Watching
    // that revision avoids fresh-checkout metadata invalidating identical
    // coverage objects. Never cache or manually pin the caller's revision env.
    if coverage
        && !commit.is_empty()
        && std::env::var("GWT_COVERAGE_REVISION").as_deref() == Ok(commit.as_str())
    {
        return;
    }

    // Without these the stamp survives a rebuild and starts lying, which is
    // the exact failure this feature exists to report. HEAD covers a checkout;
    // the branch ref covers a commit on the current branch. A packed ref has
    // no file to watch, so it is skipped rather than watched as a missing path
    // (that would rerun the script on every single build).
    for path in ["HEAD", &current_branch_ref(&workspace_root)] {
        if path.is_empty() {
            continue;
        }
        let Some(resolved) = git_output(&workspace_root, &["rev-parse", "--git-path", path]) else {
            continue;
        };
        let resolved = workspace_root.join(resolved);
        if resolved.exists() {
            println!("cargo:rerun-if-changed={}", resolved.display());
        }
    }
}

fn build_epoch(
    workspace_root: &std::path::Path,
    coverage: bool,
    build_time: std::time::SystemTime,
) -> String {
    // Coverage shards must produce identical objects before their profiles can
    // be combined. Ordinary builds retain the real build-time release stamp.
    if coverage {
        return git_output(workspace_root, &["show", "-s", "--format=%ct", "HEAD"])
            .unwrap_or_default();
    }
    build_time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs().to_string())
        .unwrap_or_default()
}

/// `refs/heads/<branch>` for the current branch, or empty when detached.
fn current_branch_ref(workspace_root: &std::path::Path) -> String {
    git_output(workspace_root, &["symbolic-ref", "--quiet", "HEAD"]).unwrap_or_default()
}

/// Trimmed stdout of a successful `git` call, or `None`.
fn git_output(workspace_root: &std::path::Path, args: &[&str]) -> Option<String> {
    // Build scripts run at compile time with a console already attached (no
    // GUI window-flash risk) and cannot depend on gwt-core's hidden_command.
    #[allow(clippy::disallowed_methods)]
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instrumented_artifacts_share_the_source_commit_epoch() {
        let root = std::path::Path::new(".");
        let expected = git_output(root, &["show", "-s", "--format=%ct", "HEAD"]).unwrap();
        assert_eq!(build_epoch(root, true, std::time::UNIX_EPOCH), expected);
        assert_eq!(
            build_epoch(
                root,
                true,
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(123)
            ),
            expected
        );
    }

    #[test]
    fn ordinary_builds_keep_the_actual_build_time() {
        let build_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(123);
        assert_eq!(
            build_epoch(std::path::Path::new("."), false, build_time),
            "123"
        );
    }
}

#[cfg(all(target_os = "windows", not(test)))]
fn embed_windows_resources() {
    println!("cargo:rerun-if-changed=../../assets/icons/icon.ico");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/icons/icon.ico");
    resource.set("FileDescription", "GWT");
    resource.set("ProductName", "GWT");
    resource.set("CompanyName", "GWT Contributors");
    resource.set("LegalCopyright", "Copyright (c) GWT Contributors");
    // #3018: without an embedded manifest declaring requestedExecutionLevel,
    // UAC installer detection treats keyword-named copies of these binaries
    // (e.g. the self-update helper) as legacy installers and CreateProcess
    // fails with ERROR_ELEVATION_REQUIRED (os error 740).
    // rfd's custom approval buttons use TaskDialogIndirect, which requires
    // selecting Common Controls v6 before the executable is loaded.
    resource.set_manifest(
        r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
        <requestedPrivileges>
            <requestedExecutionLevel level="asInvoker" uiAccess="false" />
        </requestedPrivileges>
    </security>
</trustInfo>
<dependency>
    <dependentAssembly>
        <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls"
            version="6.0.0.0" processorArchitecture="*"
            publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
</dependency>
</assembly>"#,
    );

    if let Err(error) = resource.compile() {
        panic!("failed to compile Windows resources: {error}");
    }
}
