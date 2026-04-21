use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

const DOCKER_GWT_BIN_PATH: &str = "/usr/local/bin/gwt";
const DOCKER_GWTD_BIN_PATH: &str = "/usr/local/bin/gwtd";
const DOCKER_HOST_GWT_BIN_NAME: &str = "gwt-linux";
const DOCKER_HOST_GWTD_BIN_NAME: &str = "gwtd-linux";
const DOCKER_GWT_OVERRIDE_HEADER: &str =
    "# Auto-generated docker-compose override for gwt bundle mounting";
const DOCKER_GWT_OVERRIDE_FILE_NAME: &str = "docker-compose.gwt.override.yml";
const DOCKER_USER_OVERRIDE_FILE_NAME: &str = "docker-compose.override.yml";

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerBundleMounts {
    host_gwt: PathBuf,
    host_gwtd: PathBuf,
}

pub(super) fn install_launch_gwt_bin_env(
    env_vars: &mut HashMap<String, String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    install_launch_gwt_bin_env_with_lookup(env_vars, runtime_target, &current_exe, |command| {
        which::which(command).ok()
    })
}

fn install_launch_gwt_bin_env_with_lookup(
    env_vars: &mut HashMap<String, String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<(), String> {
    let gwt_bin = match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Docker => DOCKER_GWT_BIN_PATH.to_string(),
        gwt_agent::LaunchRuntimeTarget::Host => {
            gwt::managed_assets::resolve_public_gwt_bin_with_lookup(current_exe, lookup)
                .to_string_lossy()
                .into_owned()
        }
    };
    match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Docker => {
            env_vars.insert(gwt_agent::session::GWT_BIN_PATH_ENV.to_string(), gwt_bin);
        }
        gwt_agent::LaunchRuntimeTarget::Host => {
            env_vars
                .entry(gwt_agent::session::GWT_BIN_PATH_ENV.to_string())
                .or_insert(gwt_bin);
        }
    }
    Ok(())
}

fn docker_bundle_mounts_for_gwt_home(gwt_home: &Path) -> DockerBundleMounts {
    let gwt_bin_dir = gwt_home.join("bin");
    DockerBundleMounts {
        host_gwt: gwt_bin_dir.join(DOCKER_HOST_GWT_BIN_NAME),
        host_gwtd: gwt_bin_dir.join(DOCKER_HOST_GWTD_BIN_NAME),
    }
}

#[cfg(test)]
fn docker_bundle_mounts_for_home(home: &Path) -> DockerBundleMounts {
    docker_bundle_mounts_for_gwt_home(&home.join(".gwt"))
}

fn docker_compose_mount_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn docker_bundle_override_content(service: &str, bundle: &DockerBundleMounts) -> String {
    let host_gwt = docker_compose_mount_path(&bundle.host_gwt);
    let host_gwtd = docker_compose_mount_path(&bundle.host_gwtd);
    format!(
        "{DOCKER_GWT_OVERRIDE_HEADER}\n\
         version: '3.8'\n\
         services:\n\
           {service}:\n\
             volumes:\n\
               - \"{host_gwt}:{DOCKER_GWT_BIN_PATH}:ro\"\n\
               - \"{host_gwtd}:{DOCKER_GWTD_BIN_PATH}:ro\"\n"
    )
}

pub(super) fn ensure_docker_gwt_binary_setup(
    repo_path: &Path,
    service: &str,
    target_arch: &str,
) -> Result<PathBuf, String> {
    let gwt_home = gwt_core::paths::gwt_home();
    ensure_docker_gwt_binary_setup_for_gwt_home(repo_path, service, &gwt_home, |bundle| {
        eprintln!(
            "Installing Linux gwt bundle for Docker at {} and {}",
            bundle.host_gwt.display(),
            bundle.host_gwtd.display()
        );
        let installed = gwt_core::update::UpdateManager::new().install_latest_docker_linux_bundle(
            target_arch,
            &bundle.host_gwt,
            &bundle.host_gwtd,
        )?;
        eprintln!(
            "Installed Linux gwt bundle v{} for Docker",
            installed.version
        );
        Ok(())
    })
}

pub(super) fn docker_compose_override_path(repo_path: &Path) -> PathBuf {
    repo_path.join(DOCKER_GWT_OVERRIDE_FILE_NAME)
}

pub(super) fn docker_compose_user_override_path(repo_path: &Path) -> PathBuf {
    repo_path.join(DOCKER_USER_OVERRIDE_FILE_NAME)
}

pub(super) fn is_legacy_gwt_generated_override(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .is_ok_and(|content| content.starts_with(DOCKER_GWT_OVERRIDE_HEADER))
}

#[cfg(test)]
fn ensure_docker_gwt_binary_setup_for_home<F>(
    repo_path: &Path,
    service: &str,
    home: &Path,
    install_bundle: F,
) -> Result<PathBuf, String>
where
    F: FnMut(&DockerBundleMounts) -> Result<(), String>,
{
    let gwt_home = home.join(".gwt");
    ensure_docker_gwt_binary_setup_for_gwt_home(repo_path, service, &gwt_home, install_bundle)
}

fn ensure_docker_gwt_binary_setup_for_gwt_home<F>(
    repo_path: &Path,
    service: &str,
    gwt_home: &Path,
    mut install_bundle: F,
) -> Result<PathBuf, String>
where
    F: FnMut(&DockerBundleMounts) -> Result<(), String>,
{
    use std::fs;

    let bundle = docker_bundle_mounts_for_gwt_home(gwt_home);

    if !docker_bundle_binary_ready(&bundle.host_gwt)
        || !docker_bundle_binary_ready(&bundle.host_gwtd)
    {
        install_bundle(&bundle).map_err(|err| {
            format!(
                "Failed to install Linux gwt bundle for Docker: {err}\n\
                 Expected cached binaries at {} and {}",
                bundle.host_gwt.display(),
                bundle.host_gwtd.display()
            )
        })?;
    }

    if !docker_bundle_binary_ready(&bundle.host_gwt)
        || !docker_bundle_binary_ready(&bundle.host_gwtd)
    {
        return Err(format!(
            "Linux gwt bundle setup did not create expected Docker binaries at {} and {}",
            bundle.host_gwt.display(),
            bundle.host_gwtd.display()
        ));
    }

    let override_path = docker_compose_override_path(repo_path);
    let override_content = docker_bundle_override_content(service, &bundle);
    let rewrite_override = fs::read_to_string(&override_path)
        .map(|existing| existing != override_content)
        .unwrap_or(true);
    if rewrite_override {
        fs::write(&override_path, override_content).map_err(|err| {
            format!(
                "Failed to write generated Docker compose override: {err}\n\
                 Manually create {} with gwt/gwtd bundle mounts",
                override_path.display()
            )
        })?;
    }

    Ok(override_path)
}

fn docker_bundle_binary_ready(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::PathBuf};

    use tempfile::tempdir;

    use super::{
        docker_bundle_mounts_for_home, docker_bundle_override_content,
        docker_compose_override_path, docker_compose_user_override_path,
        ensure_docker_gwt_binary_setup_for_home, install_launch_gwt_bin_env_with_lookup,
    };

    #[test]
    fn install_launch_gwt_bin_env_prefers_public_gwt_binary_for_host_sessions() {
        let current_exe = PathBuf::from(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let stable = PathBuf::from(r"C:\Users\Example\.bun\bin\gwt.exe");
        let mut env = HashMap::new();

        install_launch_gwt_bin_env_with_lookup(
            &mut env,
            gwt_agent::LaunchRuntimeTarget::Host,
            &current_exe,
            |command| {
                assert_eq!(command, "gwt");
                Some(stable.clone())
            },
        )
        .expect("install GWT_BIN_PATH");

        assert_eq!(
            env.get(gwt_agent::GWT_BIN_PATH_ENV).map(String::as_str),
            Some(stable.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn docker_bundle_override_content_mounts_front_door_and_daemon() {
        let home = PathBuf::from("/home/example");
        let bundle = docker_bundle_mounts_for_home(&home);
        let content = docker_bundle_override_content("app", &bundle);

        assert!(content.contains("/home/example/.gwt/bin/gwt-linux:/usr/local/bin/gwt:ro"));
        assert!(content.contains("/home/example/.gwt/bin/gwtd-linux:/usr/local/bin/gwtd:ro"));
        assert!(!content.contains("gwtd-linux:/usr/local/bin/gwt:ro"));
    }

    #[test]
    fn docker_binary_setup_installs_missing_bundle_before_writing_override() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let mut installer_calls = 0;

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), |bundle| {
            installer_calls += 1;
            fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent"))
                .expect("create bin dir");
            fs::write(&bundle.host_gwt, b"linux-gwt").expect("write gwt");
            fs::write(&bundle.host_gwtd, b"linux-gwtd").expect("write gwtd");
            Ok(())
        })
        .expect("docker setup");

        let bundle = docker_bundle_mounts_for_home(home.path());
        assert_eq!(installer_calls, 1);
        assert_eq!(fs::read(&bundle.host_gwt).expect("read gwt"), b"linux-gwt");
        assert_eq!(
            fs::read(&bundle.host_gwtd).expect("read gwtd"),
            b"linux-gwtd"
        );

        let override_content = fs::read_to_string(docker_compose_override_path(repo.path()))
            .expect("override content");
        assert!(override_content.contains("gwt-linux:/usr/local/bin/gwt:ro"));
        assert!(override_content.contains("gwtd-linux:/usr/local/bin/gwtd:ro"));
    }

    #[test]
    fn docker_binary_setup_repairs_directory_placeholders_before_writing_override() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        fs::create_dir_all(&bundle.host_gwt).expect("create gwt placeholder dir");
        fs::create_dir_all(&bundle.host_gwtd).expect("create gwtd placeholder dir");
        let mut installer_calls = 0;

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), |bundle| {
            installer_calls += 1;
            if bundle.host_gwt.is_dir() {
                fs::remove_dir_all(&bundle.host_gwt).expect("remove gwt placeholder");
            }
            if bundle.host_gwtd.is_dir() {
                fs::remove_dir_all(&bundle.host_gwtd).expect("remove gwtd placeholder");
            }
            fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent"))
                .expect("create bin dir");
            fs::write(&bundle.host_gwt, b"linux-gwt").expect("write gwt");
            fs::write(&bundle.host_gwtd, b"linux-gwtd").expect("write gwtd");
            Ok(())
        })
        .expect("docker setup");

        assert_eq!(installer_calls, 1);
        assert!(bundle.host_gwt.is_file());
        assert!(bundle.host_gwtd.is_file());
        assert!(docker_compose_override_path(repo.path()).is_file());
    }

    #[test]
    fn docker_binary_setup_skips_installer_when_bundle_exists() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent")).expect("create bin dir");
        fs::write(&bundle.host_gwt, b"existing-gwt").expect("write gwt");
        fs::write(&bundle.host_gwtd, b"existing-gwtd").expect("write gwtd");

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), |_| {
            panic!("installer should not run when both bundle binaries exist");
        })
        .expect("docker setup");

        assert_eq!(
            fs::read(&bundle.host_gwt).expect("read gwt"),
            b"existing-gwt"
        );
        assert_eq!(
            fs::read(&bundle.host_gwtd).expect("read gwtd"),
            b"existing-gwtd"
        );
        assert!(docker_compose_override_path(repo.path()).exists());
    }

    #[test]
    fn docker_binary_setup_preserves_existing_user_override_file() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        let user_override = docker_compose_user_override_path(repo.path());
        let user_override_content =
            "services:\n  app:\n    environment:\n      KEEP_ME: \"true\"\n";
        fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent")).expect("create bin dir");
        fs::write(&bundle.host_gwt, b"existing-gwt").expect("write gwt");
        fs::write(&bundle.host_gwtd, b"existing-gwtd").expect("write gwtd");
        fs::write(&user_override, user_override_content).expect("write user override");

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), |_| {
            panic!("installer should not run when both bundle binaries exist");
        })
        .expect("docker setup");

        assert_eq!(
            fs::read_to_string(&user_override).expect("read user override"),
            user_override_content
        );
        assert!(docker_compose_override_path(repo.path()).is_file());
    }

    #[test]
    fn docker_binary_setup_rewrites_override_for_selected_service() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent")).expect("create bin dir");
        fs::write(&bundle.host_gwt, b"existing-gwt").expect("write gwt");
        fs::write(&bundle.host_gwtd, b"existing-gwtd").expect("write gwtd");

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), |_| {
            panic!("installer should not run when both bundle binaries exist");
        })
        .expect("docker setup for app");

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "worker", home.path(), |_| {
            panic!("installer should not run when both bundle binaries exist");
        })
        .expect("docker setup for worker");

        let override_content = fs::read_to_string(docker_compose_override_path(repo.path()))
            .expect("override content");
        assert!(override_content.contains("worker:"));
        assert!(!override_content.contains("app:"));
    }
}
