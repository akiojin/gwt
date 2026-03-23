#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::result_large_err)]

mod agent_logger;
#[allow(dead_code)]
mod agent_tools;
mod app;
mod assistant_engine;
mod assistant_monitor;
mod assistant_tools;
mod commands;
mod consultation;
mod menu;
mod pty_skills;
mod session_watcher;
mod single_instance;
mod state;
mod tool_helpers;

use std::{io::Read, sync::Arc};

#[cfg(any(test, target_os = "macos"))]
use std::path::{Path, PathBuf};

use state::AppState;

#[cfg(any(test, target_os = "macos"))]
const LEGACY_WEBKIT_LOCAL_STORAGE_RESET_SENTINEL: &str = "webkit-localstorage-reset-issue-1720-v1";

fn main() {
    // Self-update helper mode: do not start GUI, just execute requested update action.
    if maybe_run_internal_mode() {
        return;
    }

    // Claude Code hooks invoke `gwt-tauri hook <Event>` (plugin hook forwarding or legacy settings hook).
    // In hook mode we must NOT start the GUI event loop; process stdin JSON and exit immediately.
    if handle_hook_cli() {
        return;
    }

    // Initialize logging before anything else so all tracing output is captured.
    let settings = gwt_core::config::Settings::load_global().unwrap_or_default();
    let log_config = gwt_core::logging::LogConfig {
        log_dir: settings.log_dir.clone().unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".gwt").join("logs"))
                .unwrap_or_else(|| std::path::PathBuf::from(".gwt/logs"))
        }),
        workspace: "default".to_string(),
        debug: settings.debug,
        retention_days: settings.log_retention_days,
        profiling: settings.profiling,
    };
    let _profiling_guard = gwt_core::logging::init_logger(&log_config);

    gwt_core::logging::log_flow_start("startup", "app_init");

    let single_instance_guard = match crate::single_instance::try_acquire_single_instance() {
        Ok(crate::single_instance::AcquireOutcome::Acquired(guard)) => Arc::new(guard),
        Ok(crate::single_instance::AcquireOutcome::AlreadyRunning(running)) => {
            if let Err(err) = crate::single_instance::notify_existing_instance_focus() {
                eprintln!(
                    "gwt is already running (pid={:?}, focus_port={:?}); failed to focus existing instance: {err}",
                    running.pid,
                    running.focus_port
                );
            }
            return;
        }
        Err(err) => {
            eprintln!("failed to initialize single-instance lock: {err}");
            return;
        }
    };

    #[cfg(target_os = "macos")]
    maybe_reset_legacy_webkit_local_storage();

    let app_state = AppState::new();

    let app = crate::app::build_app(
        tauri::Builder::default(),
        app_state,
        Some(single_instance_guard),
    )
    .build(tauri::generate_context!())
    .expect("error while building tauri application");

    gwt_core::logging::log_flow_success("startup", "app_init");

    app.run(crate::app::handle_run_event);
}

#[cfg(target_os = "macos")]
fn maybe_reset_legacy_webkit_local_storage() {
    let Some(home_dir) = dirs::home_dir() else {
        return;
    };
    match reset_legacy_webkit_local_storage(&home_dir) {
        Ok(removed_targets) if !removed_targets.is_empty() => {
            tracing::info!(
                category = "startup_migration",
                removed_targets = removed_targets.len(),
                "Reset legacy WebKit LocalStorage to avoid startup crash"
            );
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(
                category = "startup_migration",
                error = %err,
                "Failed to reset legacy WebKit LocalStorage"
            );
        }
    }
}

#[cfg(any(test, target_os = "macos"))]
fn webkit_local_storage_reset_sentinel(home_dir: &Path) -> PathBuf {
    home_dir
        .join(".gwt")
        .join("runtime")
        .join(LEGACY_WEBKIT_LOCAL_STORAGE_RESET_SENTINEL)
}

#[cfg(any(test, target_os = "macos"))]
fn webkit_local_storage_targets(home_dir: &Path) -> Vec<PathBuf> {
    let website_data_root = home_dir
        .join("Library")
        .join("WebKit")
        .join("com.akiojin.gwt")
        .join("WebsiteData");

    let mut targets = Vec::new();
    let top_level_local_storage = website_data_root.join("LocalStorage");
    if top_level_local_storage.exists() {
        targets.push(top_level_local_storage);
    }

    let default_root = website_data_root.join("Default");
    let Ok(origin_dirs) = std::fs::read_dir(default_root) else {
        targets.sort();
        targets.dedup();
        return targets;
    };

    for origin_dir in origin_dirs.flatten() {
        let direct_local_storage = origin_dir.path().join("LocalStorage");
        if direct_local_storage.exists() {
            targets.push(direct_local_storage);
        }
        let Ok(origin_children) = std::fs::read_dir(origin_dir.path()) else {
            continue;
        };
        for origin_child in origin_children.flatten() {
            let origin_child_path = origin_child.path();
            if origin_child_path.file_name().and_then(|name| name.to_str()) == Some("LocalStorage")
            {
                targets.push(origin_child_path);
                continue;
            }

            let nested_local_storage = origin_child_path.join("LocalStorage");
            if nested_local_storage.exists() {
                targets.push(nested_local_storage);
            }
        }
    }

    targets.sort();
    targets.dedup();
    targets
}

#[cfg(any(test, target_os = "macos"))]
fn reset_legacy_webkit_local_storage(home_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let sentinel = webkit_local_storage_reset_sentinel(home_dir);
    if sentinel.exists() {
        return Ok(Vec::new());
    }

    let targets = webkit_local_storage_targets(home_dir);
    for target in &targets {
        if target.is_dir() {
            std::fs::remove_dir_all(target)
                .map_err(|err| format!("failed to remove {}: {err}", target.display()))?;
        } else if target.is_file() {
            std::fs::remove_file(target)
                .map_err(|err| format!("failed to remove {}: {err}", target.display()))?;
        }
    }

    if let Some(parent) = sentinel.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(
        &sentinel,
        format!(
            "migration={}\npackage_version={}\n",
            LEGACY_WEBKIT_LOCAL_STORAGE_RESET_SENTINEL,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .map_err(|err| format!("failed to write {}: {err}", sentinel.display()))?;

    Ok(targets)
}

fn handle_hook_cli() -> bool {
    let mut args = std::env::args();
    let _exe = args.next();

    let Some(subcommand) = args.next() else {
        return false;
    };
    if subcommand != "hook" {
        return false;
    }

    // Missing event name is treated as a no-op to avoid noisy hook errors.
    let Some(event) = args.next() else {
        return true;
    };

    let mut payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut payload);

    // Best-effort: hook errors should not block Claude Code.
    let _ = gwt_core::config::process_claude_hook_event(&event, &payload);
    true
}

fn maybe_run_internal_mode() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] != "__internal" {
        return false;
    }

    match args[2].as_str() {
        "apply-update" | "apply_update" => {
            let mut old_pid: Option<u32> = None;
            let mut target: Option<String> = None;
            let mut source: Option<String> = None;
            let mut args_file: Option<String> = None;

            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--old-pid" => {
                        i += 1;
                        old_pid = args.get(i).and_then(|s| s.parse::<u32>().ok());
                    }
                    "--target" => {
                        i += 1;
                        target = args.get(i).cloned();
                    }
                    "--source" => {
                        i += 1;
                        source = args.get(i).cloned();
                    }
                    "--args-file" => {
                        i += 1;
                        args_file = args.get(i).cloned();
                    }
                    _ => {}
                }
                i += 1;
            }

            let Some(old_pid) = old_pid else {
                eprintln!("Missing --old-pid");
                std::process::exit(1);
            };
            let Some(target) = target else {
                eprintln!("Missing --target");
                std::process::exit(1);
            };
            let Some(source) = source else {
                eprintln!("Missing --source");
                std::process::exit(1);
            };
            let Some(args_file) = args_file else {
                eprintln!("Missing --args-file");
                std::process::exit(1);
            };

            let res = gwt_core::update::internal_apply_update(
                old_pid,
                std::path::Path::new(&target),
                std::path::Path::new(&source),
                std::path::Path::new(&args_file),
            );
            if let Err(err) = res {
                eprintln!("{err}");
                std::process::exit(1);
            }
            true
        }
        "run-installer" | "run_installer" => {
            let mut old_pid: Option<u32> = None;
            let mut target: Option<String> = None;
            let mut installer: Option<String> = None;
            let mut installer_kind: Option<String> = None;
            let mut args_file: Option<String> = None;

            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--old-pid" => {
                        i += 1;
                        old_pid = args.get(i).and_then(|s| s.parse::<u32>().ok());
                    }
                    "--target" => {
                        i += 1;
                        target = args.get(i).cloned();
                    }
                    "--installer" => {
                        i += 1;
                        installer = args.get(i).cloned();
                    }
                    "--installer-kind" => {
                        i += 1;
                        installer_kind = args.get(i).cloned();
                    }
                    "--args-file" => {
                        i += 1;
                        args_file = args.get(i).cloned();
                    }
                    _ => {}
                }
                i += 1;
            }

            let Some(old_pid) = old_pid else {
                eprintln!("Missing --old-pid");
                std::process::exit(1);
            };
            let Some(target) = target else {
                eprintln!("Missing --target");
                std::process::exit(1);
            };
            let Some(installer) = installer else {
                eprintln!("Missing --installer");
                std::process::exit(1);
            };
            let Some(installer_kind) = installer_kind else {
                eprintln!("Missing --installer-kind");
                std::process::exit(1);
            };
            let Some(args_file) = args_file else {
                eprintln!("Missing --args-file");
                std::process::exit(1);
            };

            let kind = match installer_kind.as_str() {
                "mac_dmg" => gwt_core::update::InstallerKind::MacDmg,
                "windows_msi" => gwt_core::update::InstallerKind::WindowsMsi,
                other => {
                    eprintln!("Unknown --installer-kind: {other}");
                    std::process::exit(1);
                }
            };

            let res = gwt_core::update::internal_run_installer(
                old_pid,
                std::path::Path::new(&target),
                std::path::Path::new(&installer),
                kind,
                std::path::Path::new(&args_file),
            );
            if let Err(err) = res {
                eprintln!("{err}");
                std::process::exit(1);
            }
            true
        }
        other => {
            eprintln!("Unknown internal mode: {other}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn webkit_local_storage_targets_collects_top_level_and_nested_dirs() {
        let temp = tempdir().unwrap();
        let home = temp.path();
        let top_level = home
            .join("Library")
            .join("WebKit")
            .join("com.akiojin.gwt")
            .join("WebsiteData")
            .join("LocalStorage");
        let nested = home
            .join("Library")
            .join("WebKit")
            .join("com.akiojin.gwt")
            .join("WebsiteData")
            .join("Default")
            .join("origin-a")
            .join("site-a")
            .join("LocalStorage");
        std::fs::create_dir_all(&top_level).unwrap();
        std::fs::create_dir_all(&nested).unwrap();

        let targets = webkit_local_storage_targets(home);

        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&top_level));
        assert!(targets.contains(&nested));
    }

    #[test]
    fn webkit_local_storage_targets_collects_direct_origin_local_storage() {
        let temp = tempdir().unwrap();
        let home = temp.path();
        let direct_origin_local_storage = home
            .join("Library")
            .join("WebKit")
            .join("com.akiojin.gwt")
            .join("WebsiteData")
            .join("Default")
            .join("origin-a")
            .join("LocalStorage");
        std::fs::create_dir_all(&direct_origin_local_storage).unwrap();

        let targets = webkit_local_storage_targets(home);

        assert_eq!(targets.len(), 1);
        assert!(targets.contains(&direct_origin_local_storage));
    }

    #[test]
    fn reset_legacy_webkit_local_storage_removes_targets_and_writes_sentinel() {
        let temp = tempdir().unwrap();
        let home = temp.path();
        let top_level = home
            .join("Library")
            .join("WebKit")
            .join("com.akiojin.gwt")
            .join("WebsiteData")
            .join("LocalStorage");
        let nested = home
            .join("Library")
            .join("WebKit")
            .join("com.akiojin.gwt")
            .join("WebsiteData")
            .join("Default")
            .join("origin-a")
            .join("site-a")
            .join("LocalStorage");
        std::fs::create_dir_all(&top_level).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(top_level.join("https___tauri.local_0.localstorage"), "").unwrap();
        std::fs::write(nested.join("localstorage.sqlite3"), "legacy").unwrap();

        let removed_targets = reset_legacy_webkit_local_storage(home).unwrap();
        let sentinel = webkit_local_storage_reset_sentinel(home);

        assert_eq!(removed_targets.len(), 2);
        assert!(!top_level.exists());
        assert!(!nested.exists());
        assert!(sentinel.exists());

        let sentinel_body = std::fs::read_to_string(sentinel).unwrap();
        assert!(sentinel_body.contains(LEGACY_WEBKIT_LOCAL_STORAGE_RESET_SENTINEL));
    }

    #[test]
    fn reset_legacy_webkit_local_storage_skips_when_sentinel_exists() {
        let temp = tempdir().unwrap();
        let home = temp.path();
        let top_level = home
            .join("Library")
            .join("WebKit")
            .join("com.akiojin.gwt")
            .join("WebsiteData")
            .join("LocalStorage");
        std::fs::create_dir_all(&top_level).unwrap();

        let sentinel = webkit_local_storage_reset_sentinel(home);
        std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        std::fs::write(&sentinel, "already-migrated").unwrap();

        let removed_targets = reset_legacy_webkit_local_storage(home).unwrap();

        assert!(removed_targets.is_empty());
        assert!(top_level.exists());
    }
}
