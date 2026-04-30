use std::{path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("-V" | "--version" | "version") => {
            println!("gwtd {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") | None => {
            print_help();
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    if !gwt::cli::should_dispatch_cli(&argv) {
        eprintln!(
            "gwtd: unknown command '{}'",
            argv.get(1).unwrap_or(&String::new())
        );
        print_help();
        return ExitCode::from(2);
    }

    let code = match argv.get(1).map(String::as_str) {
        Some("issue" | "pr" | "actions") => run_repo_backed_cli(&argv),
        _ => {
            let mut env = gwt::cli::DefaultCliEnv::new_for_hooks();
            gwt::cli::dispatch(&mut env, &argv)
        }
    };
    ExitCode::from(code.clamp(0, 255) as u8)
}

fn print_help() {
    println!("gwtd {}", env!("CARGO_PKG_VERSION"));
    println!("Headless CLI for gwt agent, hook, and workflow automation.");
    println!();
    println!("Usage: gwtd <command> [args]");
    println!();
    println!("Commands: issue, pr, actions, board, hook, discuss, plan, build, index, update");
}

fn run_repo_backed_cli(argv: &[String]) -> i32 {
    let repo_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some((owner, repo)) = resolve_repo_coordinates() else {
        eprintln!(
            "gwtd {}: could not resolve GitHub owner/repo from the current git remote",
            argv.get(1).map(String::as_str).unwrap_or("issue")
        );
        return 2;
    };
    let mut env = gwt::cli::DefaultCliEnv::new(&owner, &repo, repo_path);
    gwt::cli::dispatch(&mut env, argv)
}

fn resolve_repo_coordinates() -> Option<(String, String)> {
    let output = gwt_core::process::hidden_command("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_github_remote_url(String::from_utf8_lossy(&output.stdout).trim())
}

fn parse_github_remote_url(url: &str) -> Option<(String, String)> {
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        return parse_owner_repo(rest.trim_end_matches(".git"));
    }
    for prefix in ["https://github.com/", "http://github.com/"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            return parse_owner_repo(rest.trim_end_matches(".git").trim_end_matches('/'));
        }
    }
    None
}

fn parse_owner_repo(value: &str) -> Option<(String, String)> {
    let mut parts = value.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}
