//! Bootstrap and repair the shared project-index runtime under `~/.gwt/runtime`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{GwtError, Result};

const RUNNER_SOURCE: &str = include_str!("../runtime/chroma_index_runner.py");
const REQUIREMENTS_SOURCE: &str = include_str!("../runtime/project_index_requirements.txt");
const REQUIREMENTS_FILE: &str = "project_index_requirements.txt";
const PYTHON_VERSION_SNIPPET: &str =
    "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BootstrapPython {
    program: PathBuf,
    prefix_args: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct PythonCandidate {
    executable: &'static str,
    prefix_args: &'static [&'static str],
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

fn system_python_candidates() -> &'static [PythonCandidate] {
    #[cfg(windows)]
    {
        &[
            PythonCandidate {
                executable: "python3.13",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.12",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.11",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.10",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.9",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "py",
                prefix_args: &["-3"],
            },
            PythonCandidate {
                executable: "python",
                prefix_args: &[],
            },
        ]
    }

    #[cfg(not(windows))]
    {
        &[
            PythonCandidate {
                executable: "python3.13",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.12",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.11",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.10",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3.9",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python3",
                prefix_args: &[],
            },
            PythonCandidate {
                executable: "python",
                prefix_args: &[],
            },
        ]
    }
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
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    normalized.contains("\\appdata\\local\\microsoft\\windowsapps\\")
        && file_name.starts_with("python")
        && file_name.ends_with(".exe")
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
        .map_err(|e| format!("Failed to check Python version: {e}"))?;

    if !output.status.success() {
        return Err("Failed to determine Python version".into());
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
    for candidate in system_python_candidates() {
        let Some(path) = resolve(candidate.executable) else {
            continue;
        };
        if is_windows_store_python_alias(&path) {
            continue;
        }

        let Ok((major, minor, _version)) = probe(&path, candidate.prefix_args) else {
            continue;
        };
        if supported_project_index_python_version(major, minor) {
            return Ok(BootstrapPython {
                program: path,
                prefix_args: candidate.prefix_args,
            });
        }
    }

    Err(project_index_python_install_guidance())
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
        fail_probe_once: Cell<bool>,
        python: BootstrapPython,
    }

    impl FakeProvisioner {
        fn new(root: &Path) -> Self {
            let python = root.join("python3");
            fs::write(&python, "#!/bin/sh\n").unwrap();
            Self {
                calls: RefCell::new(Vec::new()),
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
    fn find_bootstrap_python_with_skips_windows_store_alias_and_uses_next_candidate() {
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
    fn find_bootstrap_python_with_returns_install_guidance_when_missing() {
        let error =
            find_bootstrap_python_with(|_| None, |_, _| unreachable!("no candidate should probe"))
                .expect_err("missing python should fail");

        assert!(error.contains("Python 3.9+"));
        assert!(error.contains("py -3"));
        assert!(error.contains("python"));
    }
}
