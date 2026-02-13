#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_master;
mod agent_tools;
mod app;
mod commands;
#[cfg_attr(test, allow(dead_code))]
mod mcp_handlers;
#[cfg_attr(test, allow(dead_code))]
mod mcp_ws_server;
mod menu;
mod state;

use state::AppState;
use std::io::Read;

fn main() {
    // Self-update helper mode: do not start GUI, just execute requested update action.
    if maybe_run_internal_mode() {
        return;
    }

    // Claude Code hooks call `/Applications/gwt.app/.../gwt-tauri hook <Event>` via `~/.claude/settings.json`.
    // In hook mode we must NOT start the GUI event loop; process stdin JSON and exit immediately.
    if handle_hook_cli() {
        return;
    }

    let app_state = AppState::new();

    let app = crate::app::build_app(tauri::Builder::default(), app_state)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(crate::app::handle_run_event);
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
                "mac_pkg" => gwt_core::update::InstallerKind::MacPkg,
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
