//! Bootstrap and repair the shared project-index runtime under `~/.gwt/runtime`.

use std::fs;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{GwtError, Result};

const RUNNER_SOURCE: &str = include_str!("../runtime/chroma_index_runner.py");
const REQUIREMENTS_SOURCE: &str = include_str!("../runtime/project_index_requirements.txt");
const REQUIREMENTS_FILE: &str = "project_index_requirements.txt";
const PYTHON_VERSION_SNIPPET: &str =
    "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')";
const PROJECT_INDEX_RUNTIME_ERROR_PREFIX: &str = "[gwt-project-index-runtime]";
const PROJECT_INDEX_PYTHON_INSTALL_REQUIRED_PREFIX: &str = "[gwt-project-index-python-install]";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BootstrapPython {
    program: PathBuf,
    prefix_args: &'static [&'static str],
}

#[derive(Debug, Clone)]
struct PythonCandidate {
    executable: String,
    prefix_args: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectIndexRuntimeErrorKind {
    RuntimeUnavailable,
    PythonInstallRequired,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectIndexRuntimeReport {
    pub runner_updated: bool,
    pub requirements_updated: bool,
    pub venv_created: bool,
    pub venv_rebuilt: bool,
    pub dependencies_installed: bool,
}

pub fn ensure_project_index_runtime() -> Result<ProjectIndexRuntimeReport> {
    ensure_project_index_runtime_with(&crate::paths::gwt_home(), &RealProvisioner)
        .map_err(wrap_project_index_runtime_error)
}

trait RuntimeProvisioner {
    fn find_python(&self) -> Result<BootstrapPython>;
    fn create_venv(&self, python: &BootstrapPython, venv_dir: &Path) -> Result<()>;
    fn install_requirements(&self, venv_python: &Path, requirements: &Path) -> Result<()>;
    fn probe_chromadb(&self, venv_python: &Path) -> Result<()>;
}

struct RealProvisioner;

impl RuntimeProvisioner for RealProvisioner {
    fn find_python(&self) -> Result<BootstrapPython> {
        find_bootstrap_python().map_err(GwtError::Other)
    }

    fn create_venv(&self, python: &BootstrapPython, venv_dir: &Path) -> Result<()> {
        let mut command = Command::new(&python.program);
        command
            .args(python.prefix_args)
            .arg("-m")
            .arg("venv")
            .arg(venv_dir);
        run_checked(&mut command, "python -m venv")
    }

    fn install_requirements(&self, venv_python: &Path, requirements: &Path) -> Result<()> {
        run_checked(
            Command::new(venv_python)
                .arg("-m")
                .arg("pip")
                .arg("install")
                .arg("--disable-pip-version-check")
                .arg("-r")
                .arg(requirements),
            "pip install -r",
        )
    }

    fn probe_chromadb(&self, venv_python: &Path) -> Result<()> {
        run_checked(
            Command::new(venv_python).arg("-c").arg("import chromadb"),
            "python -c import chromadb",
        )
    }
}

fn ensure_project_index_runtime_with(
    gwt_home: &Path,
    provisioner: &impl RuntimeProvisioner,
) -> Result<ProjectIndexRuntimeReport> {
    let mut report = ProjectIndexRuntimeReport::default();
    let runtime_dir = crate::paths::gwt_runtime_dir_from(gwt_home);
    let runner_path = crate::paths::gwt_runtime_runner_path_from(gwt_home);
    let requirements_path = runtime_dir.join(REQUIREMENTS_FILE);
    let venv_dir = crate::paths::gwt_project_index_venv_dir_from(gwt_home);

    crate::paths::ensure_dir(&runtime_dir)?;
    report.runner_updated = write_if_changed(&runner_path, RUNNER_SOURCE)?;
    report.requirements_updated = write_if_changed(&requirements_path, REQUIREMENTS_SOURCE)?;

    let mut venv_python = venv_python_path(&venv_dir);
    let mut needs_install = report.requirements_updated;

    if !venv_python.exists() {
        let python = provisioner.find_python()?;
        provisioner.create_venv(&python, &venv_dir)?;
        report.venv_created = true;
        needs_install = true;
        venv_python = venv_python_path(&venv_dir);
    }

    if needs_install {
        provisioner.install_requirements(&venv_python, &requirements_path)?;
        report.dependencies_installed = true;
    }

    if let Err(first_probe_error) = provisioner.probe_chromadb(&venv_python) {
        if venv_dir.exists() {
            fs::remove_dir_all(&venv_dir)?;
        }
        let python = provisioner.find_python()?;
        provisioner.create_venv(&python, &venv_dir)?;
        venv_python = venv_python_path(&venv_dir);
        provisioner.install_requirements(&venv_python, &requirements_path)?;
        report.venv_rebuilt = true;
        report.dependencies_installed = true;
        provisioner
            .probe_chromadb(&venv_python)
            .map_err(|_| first_probe_error)?;
    }

    Ok(report)
}

fn venv_python_path(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python3")
    }
}

fn base_python_candidates() -> Vec<PythonCandidate> {
    #[cfg(windows)]
    {
        vec![
            PythonCandidate {
                executable: "python3.13".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.12".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.11".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.10".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.9".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "py".into(),
                prefix_args: &["-3"],
            },
            PythonCandidate {
                executable: "python".into(),
                prefix_args: &[],
            },
        ]
    }

    #[cfg(not(windows))]
    {
        vec![
            PythonCandidate {
                executable: "python3.13".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.12".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.11".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.10".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.9".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3".into(),
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python".into(),
                prefix_args: &[],
            },
        ]
    }
}

fn system_python_candidates() -> Vec<PythonCandidate> {
    let mut candidates = versioned_python_candidates_from_path_env();
    let mut seen: Vec<String> = candidates
        .iter()
        .map(|candidate| candidate.executable.clone())
        .collect();

    for candidate in base_python_candidates() {
        if seen.contains(&candidate.executable) {
            continue;
        }
        seen.push(candidate.executable.clone());
        candidates.push(candidate);
    }

    candidates
}

fn versioned_python_candidates_from_path_env() -> Vec<PythonCandidate> {
    std::env::var_os("PATH")
        .as_ref()
        .map(|path| versioned_python_candidates_from_paths(std::env::split_paths(path)))
        .unwrap_or_default()
}

fn versioned_python_candidates_from_paths<I>(paths: I) -> Vec<PythonCandidate>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut discovered: Vec<(u32, String)> = Vec::new();

    for dir in paths {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            let Some(minor) = parse_versioned_python_candidate_name(&file_name) else {
                continue;
            };

            discovered.push((minor, format!("python3.{minor}")));
        }
    }

    discovered.sort_by(|left, right| right.0.cmp(&left.0));
    discovered.dedup_by(|left, right| left.1 == right.1);

    discovered
        .into_iter()
        .map(|(_, executable)| PythonCandidate {
            executable,
            prefix_args: &[],
        })
        .collect()
}

fn parse_versioned_python_candidate_name(name: &str) -> Option<u32> {
    let normalized = name.to_ascii_lowercase();
    let normalized = normalized.strip_suffix(".exe").unwrap_or(&normalized);
    let suffix = normalized.strip_prefix("python3.")?;
    if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<u32>().ok()
}

fn parse_python_version(version_str: &str) -> std::result::Result<(u32, u32), String> {
    let parts: Vec<&str> = version_str.trim().split('.').collect();
    if parts.len() < 2 {
        return Err(format!("Unexpected Python version format: {version_str}"));
    }

    let major = parts[0]
        .parse::<u32>()
        .map_err(|_| format!("Invalid Python major version: {}", parts[0]))?;
    let minor = parts[1]
        .parse::<u32>()
        .map_err(|_| format!("Invalid Python minor version: {}", parts[1]))?;
    Ok((major, minor))
}

fn supported_project_index_python_version(major: u32, minor: u32) -> bool {
    major == 3 && minor >= 9
}

fn is_windows_store_python_alias(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(part) => part.to_string_lossy().eq_ignore_ascii_case("WindowsApps"),
        _ => false,
    })
}

fn python_version(
    path: &Path,
    prefix_args: &[&str],
) -> std::result::Result<(u32, u32, String), String> {
    let output = Command::new(path)
        .args(prefix_args)
        .arg("-c")
        .arg(PYTHON_VERSION_SNIPPET)
        .output()
        .map_err(|e| format!("failed to execute version probe: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = match (stderr.is_empty(), stdout.is_empty()) {
            (true, true) => String::new(),
            (false, true) => stderr,
            (true, false) => stdout,
            (false, false) => format!("{stderr}; stdout: {stdout}"),
        };

        if detail.is_empty() {
            return Err(format!("version probe exited with {}", output.status));
        }

        return Err(format!(
            "version probe exited with {}: {detail}",
            output.status
        ));
    }

    let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let (major, minor) = parse_python_version(&version_str)?;
    Ok((major, minor, version_str))
}

fn project_index_python_install_guidance() -> String {
    "Project index runtime requires Python 3.9+ on PATH. Install Python and ensure `python` or `py -3` works before reopening gwt.".into()
}

fn find_bootstrap_python() -> std::result::Result<BootstrapPython, String> {
    find_bootstrap_python_with(|name| which::which(name).ok(), python_version)
}

fn find_bootstrap_python_with<Resolve, Probe>(
    resolve: Resolve,
    probe: Probe,
) -> std::result::Result<BootstrapPython, String>
where
    Resolve: Fn(&str) -> Option<PathBuf>,
    Probe: Fn(&Path, &[&str]) -> std::result::Result<(u32, u32, String), String>,
{
    let mut saw_candidate = false;
    let mut last_issue = None;

    for candidate in system_python_candidates() {
        let Some(path) = resolve(candidate.executable.as_str()) else {
            continue;
        };
        saw_candidate = true;

        let (major, minor, version) = match probe(&path, candidate.prefix_args) {
            Ok(version) => version,
            Err(error) => {
                let alias_note = if is_windows_store_python_alias(&path) {
                    " (launcher entrypoint)"
                } else {
                    ""
                };
                last_issue = Some(format!(
                    "{}{} could not report a usable Python version: {error}",
                    path.display(),
                    alias_note
                ));
                continue;
            }
        };
        if supported_project_index_python_version(major, minor) {
            return Ok(BootstrapPython {
                program: path,
                prefix_args: candidate.prefix_args,
            });
        }
        last_issue = Some(format!(
            "{} reported Python {version}; project index requires Python 3.9+",
            path.display()
        ));
    }

    if !saw_candidate {
        return Err(tag_project_index_runtime_error(
            ProjectIndexRuntimeErrorKind::PythonInstallRequired,
            project_index_python_install_guidance(),
        ));
    }

    let detail = format!(
        "No supported Python 3.9+ bootstrap candidate was usable. {}",
        last_issue.unwrap_or_else(project_index_python_install_guidance)
    );
    Err(tag_project_index_runtime_error(
        ProjectIndexRuntimeErrorKind::RuntimeUnavailable,
        detail,
    ))
}

pub fn project_index_runtime_error_kind(detail: &str) -> Option<ProjectIndexRuntimeErrorKind> {
    if detail.starts_with(PROJECT_INDEX_PYTHON_INSTALL_REQUIRED_PREFIX) {
        return Some(ProjectIndexRuntimeErrorKind::PythonInstallRequired);
    }
    if detail.starts_with(PROJECT_INDEX_RUNTIME_ERROR_PREFIX) {
        return Some(ProjectIndexRuntimeErrorKind::RuntimeUnavailable);
    }
    None
}

pub fn project_index_runtime_error_detail(detail: &str) -> &str {
    detail
        .strip_prefix(PROJECT_INDEX_PYTHON_INSTALL_REQUIRED_PREFIX)
        .or_else(|| detail.strip_prefix(PROJECT_INDEX_RUNTIME_ERROR_PREFIX))
        .unwrap_or(detail)
        .trim_start()
}

fn tag_project_index_runtime_error(
    kind: ProjectIndexRuntimeErrorKind,
    detail: impl AsRef<str>,
) -> String {
    let prefix = match kind {
        ProjectIndexRuntimeErrorKind::RuntimeUnavailable => PROJECT_INDEX_RUNTIME_ERROR_PREFIX,
        ProjectIndexRuntimeErrorKind::PythonInstallRequired => {
            PROJECT_INDEX_PYTHON_INSTALL_REQUIRED_PREFIX
        }
    };
    format!("{prefix} {}", detail.as_ref())
}

fn wrap_project_index_runtime_error(err: GwtError) -> GwtError {
    let detail = err.to_string();
    if project_index_runtime_error_kind(&detail).is_some() {
        return GwtError::Other(detail);
    }
    GwtError::Other(tag_project_index_runtime_error(
        ProjectIndexRuntimeErrorKind::RuntimeUnavailable,
        detail,
    ))
}

fn write_if_changed(path: &Path, contents: &str) -> Result<bool> {
    match fs::read_to_string(path) {
        Ok(existing) if existing == contents => Ok(false),
        Ok(_) | Err(_) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, contents)?;
            Ok(true)
        }
    }
}

fn run_checked(command: &mut Command, label: &str) -> Result<()> {
    let output = command.output().map_err(GwtError::Io)?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(GwtError::Other(format!(
        "{label} failed with {}: {detail}",
        output.status
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::fs;

    use crate::paths::{gwt_project_index_venv_dir_from, gwt_runtime_runner_path_from};

    #[derive(Default)]
    struct FakeProvisioner {
        calls: RefCell<Vec<&'static str>>,
        create_venv_prefix_args: RefCell<Vec<&'static str>>,
        fail_probe_once: Cell<bool>,
        python: BootstrapPython,
    }

    impl FakeProvisioner {
        fn new(root: &Path) -> Self {
            let python = root.join("python3");
            fs::write(&python, "#!/bin/sh\n").unwrap();
            Self {
                calls: RefCell::new(Vec::new()),
                create_venv_prefix_args: RefCell::new(Vec::new()),
                fail_probe_once: Cell::new(false),
                python: BootstrapPython {
                    program: python,
                    prefix_args: &[],
                },
            }
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.borrow().clone()
        }
    }

    impl RuntimeProvisioner for FakeProvisioner {
        fn find_python(&self) -> Result<BootstrapPython> {
            self.calls.borrow_mut().push("find_python");
            Ok(self.python.clone())
        }

        fn create_venv(&self, _python: &BootstrapPython, venv_dir: &Path) -> Result<()> {
            self.calls.borrow_mut().push("create_venv");
            *self.create_venv_prefix_args.borrow_mut() = _python.prefix_args.to_vec();
            let venv_python = venv_python_path(venv_dir);
            fs::create_dir_all(venv_python.parent().unwrap()).unwrap();
            fs::write(venv_python, "#!/bin/sh\n").unwrap();
            Ok(())
        }

        fn install_requirements(&self, _venv_python: &Path, requirements: &Path) -> Result<()> {
            self.calls.borrow_mut().push("install_requirements");
            assert!(requirements.exists());
            Ok(())
        }

        fn probe_chromadb(&self, _venv_python: &Path) -> Result<()> {
            self.calls.borrow_mut().push("probe_chromadb");
            if self.fail_probe_once.replace(false) {
                return Err(GwtError::Other("probe failed".into()));
            }
            Ok(())
        }
    }

    #[test]
    fn ensure_project_index_runtime_writes_assets_and_creates_venv() {
        let root = tempfile::tempdir().unwrap();
        let gwt_home = root.path().join(".gwt");
        let provisioner = FakeProvisioner::new(root.path());

        let report = ensure_project_index_runtime_with(&gwt_home, &provisioner).unwrap();

        assert!(report.runner_updated);
        assert!(report.requirements_updated);
        assert!(report.venv_created);
        assert!(report.dependencies_installed);
        assert!(gwt_runtime_runner_path_from(&gwt_home).exists());
        assert!(venv_python_path(&gwt_project_index_venv_dir_from(&gwt_home)).exists());
        assert_eq!(
            provisioner.calls(),
            vec![
                "find_python",
                "create_venv",
                "install_requirements",
                "probe_chromadb"
            ]
        );
    }

    #[test]
    fn ensure_project_index_runtime_skips_reinstall_when_current() {
        let root = tempfile::tempdir().unwrap();
        let gwt_home = root.path().join(".gwt");
        let provisioner = FakeProvisioner::new(root.path());

        let first = ensure_project_index_runtime_with(&gwt_home, &provisioner).unwrap();
        assert!(first.dependencies_installed);

        provisioner.calls.borrow_mut().clear();

        let second = ensure_project_index_runtime_with(&gwt_home, &provisioner).unwrap();
        assert!(!second.runner_updated);
        assert!(!second.requirements_updated);
        assert!(!second.venv_created);
        assert!(!second.venv_rebuilt);
        assert!(!second.dependencies_installed);
        assert_eq!(provisioner.calls(), vec!["probe_chromadb"]);
    }

    #[test]
    fn ensure_project_index_runtime_rebuilds_broken_venv() {
        let root = tempfile::tempdir().unwrap();
        let gwt_home = root.path().join(".gwt");
        let provisioner = FakeProvisioner::new(root.path());

        let _ = ensure_project_index_runtime_with(&gwt_home, &provisioner).unwrap();
        provisioner.calls.borrow_mut().clear();
        provisioner.fail_probe_once.set(true);

        let report = ensure_project_index_runtime_with(&gwt_home, &provisioner).unwrap();
        assert!(report.venv_rebuilt);
        assert!(report.dependencies_installed);
        assert_eq!(
            provisioner.calls(),
            vec![
                "probe_chromadb",
                "find_python",
                "create_venv",
                "install_requirements",
                "probe_chromadb"
            ]
        );
    }

    #[test]
    fn find_bootstrap_python_with_uses_next_candidate_after_launcher_probe_failure() {
        let windows_store_alias =
            PathBuf::from(r"C:\Users\akiojin\AppData\Local\Microsoft\WindowsApps\python.exe");
        let real_python = PathBuf::from(r"C:\Python313\python.exe");

        let selected = find_bootstrap_python_with(
            |name| match name {
                "python3.13" => Some(windows_store_alias.clone()),
                "python3.12" => Some(real_python.clone()),
                _ => None,
            },
            |path, _| {
                if path == windows_store_alias.as_path() {
                    Err("windows store alias".into())
                } else {
                    Ok((3, 12, "3.12".into()))
                }
            },
        )
        .expect("bootstrap python");

        assert_eq!(selected.program, real_python);
    }

    #[test]
    fn find_bootstrap_python_with_accepts_working_windows_store_python() {
        let windows_store_python =
            PathBuf::from(r"C:\Users\akiojin\AppData\Local\Microsoft\WindowsApps\python.exe");

        let selected = find_bootstrap_python_with(
            |name| match name {
                "python3" => Some(windows_store_python.clone()),
                _ => None,
            },
            |_path, _| Ok((3, 13, "3.13".into())),
        )
        .expect("bootstrap python");

        assert_eq!(selected.program, windows_store_python);
    }

    #[test]
    fn find_bootstrap_python_with_falls_back_from_python_38_to_python_39() {
        let unsupported_python = PathBuf::from("/tmp/python3.8");
        let supported_python = PathBuf::from("/tmp/python3.9");

        let selected = find_bootstrap_python_with(
            |name| match name {
                "python3.13" => Some(unsupported_python.clone()),
                "python3.12" => Some(supported_python.clone()),
                _ => None,
            },
            |path, _| {
                if path == unsupported_python.as_path() {
                    Ok((3, 8, "3.8".into()))
                } else {
                    Ok((3, 9, "3.9".into()))
                }
            },
        )
        .expect("bootstrap python");

        assert_eq!(selected.program, supported_python);
    }

    #[test]
    fn find_bootstrap_python_with_returns_install_guidance_when_missing() {
        let error =
            find_bootstrap_python_with(|_| None, |_, _| unreachable!("no candidate should probe"))
                .expect_err("missing python should fail");

        assert_eq!(
            project_index_runtime_error_kind(&error),
            Some(ProjectIndexRuntimeErrorKind::PythonInstallRequired)
        );
        let detail = project_index_runtime_error_detail(&error);
        assert!(detail.contains("Python 3.9+"));
        assert!(detail.contains("py -3"));
        assert!(detail.contains("python"));
    }

    #[test]
    fn find_bootstrap_python_with_reports_unusable_candidates_without_install_guidance() {
        let broken_python = PathBuf::from("/tmp/python3");
        let error = find_bootstrap_python_with(
            |name| match name {
                "python3.13" => Some(broken_python.clone()),
                _ => None,
            },
            |_path, _| Err("launcher failure".into()),
        )
        .expect_err("broken candidate should fail");

        assert_eq!(
            project_index_runtime_error_kind(&error),
            Some(ProjectIndexRuntimeErrorKind::RuntimeUnavailable)
        );
        let detail = project_index_runtime_error_detail(&error);
        assert!(detail.contains("launcher failure"));
        assert!(!detail.contains("Install Python"));
    }

    #[test]
    fn ensure_project_index_runtime_passes_py_launcher_prefix_args_to_create_venv() {
        let root = tempfile::tempdir().unwrap();
        let gwt_home = root.path().join(".gwt");
        let provisioner = FakeProvisioner {
            python: BootstrapPython {
                program: root.path().join("py"),
                prefix_args: &["-3"],
            },
            ..FakeProvisioner::default()
        };
        fs::write(&provisioner.python.program, "#!/bin/sh\n").unwrap();

        let report = ensure_project_index_runtime_with(&gwt_home, &provisioner).unwrap();

        assert!(report.venv_created);
        assert_eq!(
            provisioner.create_venv_prefix_args.borrow().as_slice(),
            &["-3"]
        );
    }

    #[test]
    fn versioned_python_candidates_from_paths_discovers_future_python_minors() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("python3.15"), "").unwrap();
        fs::write(root.path().join("python3.14"), "").unwrap();

        let candidates = versioned_python_candidates_from_paths(vec![root.path().to_path_buf()]);

        assert_eq!(candidates[0].executable, "python3.15");
        assert_eq!(candidates[1].executable, "python3.14");
    }

    #[test]
    fn versioned_python_candidates_from_paths_ignores_non_python_binaries() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("python3-config"), "").unwrap();
        fs::write(root.path().join("python3.11"), "").unwrap();

        let candidates = versioned_python_candidates_from_paths(vec![root.path().to_path_buf()]);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].executable, "python3.11");
    }

    #[test]
    fn wrap_project_index_runtime_error_preserves_existing_python_install_marker() {
        let error = GwtError::Other(tag_project_index_runtime_error(
            ProjectIndexRuntimeErrorKind::PythonInstallRequired,
            project_index_python_install_guidance(),
        ));

        let wrapped = wrap_project_index_runtime_error(error);
        let wrapped = wrapped.to_string();
        assert_eq!(
            project_index_runtime_error_kind(&wrapped),
            Some(ProjectIndexRuntimeErrorKind::PythonInstallRequired)
        );
        let detail = project_index_runtime_error_detail(&wrapped);
        assert!(detail.contains("Python 3.9+"));
    }

    #[test]
    fn project_index_runtime_error_detail_strips_runtime_prefix() {
        let error = tag_project_index_runtime_error(
            ProjectIndexRuntimeErrorKind::RuntimeUnavailable,
            "pip install -r failed",
        );
        assert_eq!(
            project_index_runtime_error_detail(&error),
            "pip install -r failed"
        );
    }

    #[test]
    fn find_bootstrap_python_with_reports_supported_boundary_version() {
        let python39 = PathBuf::from("/tmp/python3.9");
        let selected = find_bootstrap_python_with(
            |name| match name {
                "python3.13" => Some(python39.clone()),
                _ => None,
            },
            |_path, _| Ok((3, 9, "3.9".into())),
        )
        .expect("python 3.9 should be accepted");

        assert_eq!(selected.program, python39);
    }

    #[test]
    fn find_bootstrap_python_with_returns_runtime_unavailable_for_too_old_python_only() {
        let python38 = PathBuf::from("/tmp/python3.8");
        let error = find_bootstrap_python_with(
            |name| match name {
                "python3.13" => Some(python38.clone()),
                _ => None,
            },
            |_path, _| Ok((3, 8, "3.8".into())),
        )
        .expect_err("python 3.8 should be rejected");

        assert_eq!(
            project_index_runtime_error_kind(&error),
            Some(ProjectIndexRuntimeErrorKind::RuntimeUnavailable)
        );
        let detail = project_index_runtime_error_detail(&error);
        assert!(detail.contains("3.8"));
    }
}
