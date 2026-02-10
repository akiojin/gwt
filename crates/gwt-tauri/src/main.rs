#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod commands;
mod menu;
mod state;

use state::AppState;

fn main() {
    if maybe_run_internal_mode() {
        return;
    }

    let app_state = AppState::new();

    let app = crate::app::build_app(tauri::Builder::default(), app_state)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(crate::app::handle_run_event);
}

fn maybe_run_internal_mode() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] != "__internal" {
        return false;
    }

    match args[2].as_str() {
        "apply-update" | "apply_update" => {
            let mut target: Option<String> = None;
            let mut source: Option<String> = None;
            let mut args_file: Option<String> = None;

            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
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
        other => {
            eprintln!("Unknown internal mode: {other}");
            std::process::exit(1);
        }
    }
}
