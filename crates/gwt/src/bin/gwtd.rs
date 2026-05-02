use std::{path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("-V" | "--version" | "version") => {
            println!("gwtd {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") | None => {
            // SPEC-1942 T-204: `gwtd --help <family>` shows family-scoped help.
            if let Some(family) = argv.get(2).map(String::as_str) {
                if let Some(help_text) = family_help(family) {
                    print!("{help_text}");
                    return ExitCode::SUCCESS;
                }
            }
            print_help();
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    // SPEC-1942 T-204: `gwtd <family> --help` mirrors `gwtd --help <family>`.
    if matches!(argv.get(2).map(String::as_str), Some("-h" | "--help")) {
        if let Some(family) = argv.get(1).map(String::as_str) {
            if let Some(help_text) = family_help(family) {
                print!("{help_text}");
                return ExitCode::SUCCESS;
            }
        }
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
    println!("       gwtd --help <command>   Show subcommands for a family");
    println!();
    println!("Commands:");
    println!("  issue       Manage GitHub Issues and SPEC sections");
    println!("  pr          Manage pull requests, reviews, checks, threads");
    println!("  actions     Fetch GitHub Actions run/job logs");
    println!("  board       Read/write the coordination Board (SPEC-1974)");
    println!("  hook        Dispatch Claude Code / Codex hook events");
    println!("  index       Manage the local search index");
    println!("  discuss     gwt-discussion exit CLI (SPEC-1935)");
    println!("  plan        gwt-plan-spec exit CLI (SPEC-1935)");
    println!("  build       gwt-build-spec exit CLI (SPEC-1935)");
    println!("  update      Check / apply gwt updates");
    println!("  daemon      Long-running runtime daemon (SPEC-2077)");
}

/// SPEC-1942 T-204: render family-scoped help text. Returns `None` for
/// unknown families so the caller can fall back to the global help.
fn family_help(family: &str) -> Option<String> {
    match family {
        "issue" => Some(format_issue_help()),
        "pr" => Some(format_pr_help()),
        "actions" => Some(format_actions_help()),
        "board" => Some(format_board_help()),
        "hook" => Some(format_hook_help()),
        "index" => Some(format_index_help()),
        "discuss" => Some(format_discuss_help()),
        "plan" => Some(format_plan_help()),
        "build" => Some(format_build_help()),
        "update" => Some(format_update_help()),
        "daemon" => Some(format_daemon_help()),
        _ => None,
    }
}

fn format_daemon_help() -> String {
    [
        "gwtd daemon — Long-running runtime daemon (SPEC-2077).",
        "",
        "Usage: gwtd daemon <subcommand>",
        "",
        "Subcommands:",
        "  start                                   Bootstrap and serve the runtime daemon",
        "  status                                  Report whether a daemon is registered + probe its socket",
        "",
        "Notes:",
        "  - Listens on a Unix domain socket per RuntimeScope (POSIX only today).",
        "  - Endpoint metadata is persisted under ~/.gwt/projects/<repo>/runtime/daemon/.",
        "  - SIGINT / SIGTERM trigger graceful shutdown + endpoint file removal.",
        "  - `status` reports `probe=ok uptime=<s>s channels=<n>` when the daemon answers a",
        "    `ClientFrame::Status` request within 1s, or `probe=failed:<reason>` when the",
        "    endpoint file is stale or unreachable.",
        "",
    ]
    .join("\n")
}

fn format_issue_help() -> String {
    [
        "gwtd issue — Manage GitHub Issues and SPEC sections.",
        "",
        "Usage: gwtd issue <subcommand> [args]",
        "",
        "Subcommands:",
        "  spec <n>                              Print every section for an issue",
        "  spec <n> --section <name>             Print one section only",
        "  spec <n> --edit <name> -f <file>      Replace one section from file (- = stdin)",
        "  spec <n> --edit spec --json [-f <f>] [--replace]",
        "                                         Structured JSON update for the spec section",
        "  spec <n> --rename <title>             Update the issue title",
        "  spec list [--phase <p>] [--state open|closed]",
        "                                         List SPEC-labeled issues",
        "  spec create --title <t> -f <body> [--label <l>]*",
        "                                         Create a SPEC issue",
        "  spec create --json --title <t> [-f <f>] [--label <l>]*",
        "                                         Create a SPEC from structured JSON",
        "  spec pull [--all | <n>...]            Refresh cache from server",
        "  spec repair <n>                        Clear cache and re-fetch from server",
        "  view <n> [--refresh]                  Print a plain issue from cache or live",
        "  comments <n> [--refresh]              Print issue comments",
        "  linked-prs <n> [--refresh]            Print linked PR summaries",
        "  create --title <t> -f <body> [--label <l>]*",
        "                                         Create a plain issue",
        "  comment <n> -f <body>                 Create a plain issue comment",
        "",
    ]
    .join("\n")
}

fn format_pr_help() -> String {
    [
        "gwtd pr — Manage pull requests, reviews, checks, and threads.",
        "",
        "Usage: gwtd pr <subcommand> [args]",
        "",
        "Subcommands:",
        "  current                                Print the PR for the current branch",
        "  create --base <b> [--head <h>] --title <t> -f <body> [--label <l>]* [--draft]",
        "                                          Create a pull request",
        "  edit <n> [--title <t>] [-f <body>] [--add-label <l>]*",
        "                                          Update a pull request",
        "  view <n>                               Print a PR by number",
        "  comment <n> -f <body>                  Add a PR issue comment",
        "  reviews <n>                            Print PR review summaries",
        "  review-threads <n>                     Print review thread snapshots",
        "  review-threads reply-and-resolve <n> -f <body>",
        "                                          Reply to + resolve unresolved threads",
        "  checks <n>                             Print PR checks summary",
        "",
    ]
    .join("\n")
}

fn format_actions_help() -> String {
    [
        "gwtd actions — Fetch GitHub Actions run/job logs.",
        "",
        "Usage: gwtd actions <subcommand> [args]",
        "",
        "Subcommands:",
        "  logs --run <id>                        Print raw run logs",
        "  job-logs --job <id>                    Print raw job logs",
        "",
    ]
    .join("\n")
}

fn format_board_help() -> String {
    [
        "gwtd board — Read/write the coordination Board (SPEC-1974).",
        "",
        "Usage: gwtd board <subcommand> [args]",
        "",
        "Subcommands:",
        "  show [--json]                           Print the Board snapshot",
        "  post --kind <kind> (--body <text> | -f <file>)",
        "       [--parent <id>] [--topic <t>]* [--owner <n>]* [--target <id>]*",
        "                                          Append a Board entry",
        "",
        "Kinds: request, status, next, claim, impact, question, blocked, handoff, decision",
        "",
    ]
    .join("\n")
}

fn format_hook_help() -> String {
    [
        "gwtd hook — Dispatch Claude Code / Codex hook events (SPEC-1935).",
        "",
        "Usage: gwtd hook <name> [args]",
        "",
        "Hook names:",
        "  event <PreToolUse|...>                  Generic event dispatcher",
        "  runtime-state <event>                   Runtime state telemetry",
        "  coordination-event <event>              Coordination Board event",
        "  board-reminder                          Board reminder injection",
        "  workflow-policy                         PreToolUse workflow guard",
        "  block-bash-policy                       PreToolUse Bash policy guard",
        "  forward                                 Bridge to live event stream",
        "  skill-discussion-stop-check             Stop-check for gwt-discussion",
        "  skill-plan-spec-stop-check              Stop-check for gwt-plan-spec",
        "  skill-build-spec-stop-check             Stop-check for gwt-build-spec",
        "",
        "Stdin: hooks read JSON from stdin per the Claude Code hook contract.",
        "",
    ]
    .join("\n")
}

fn format_index_help() -> String {
    [
        "gwtd index — Manage the local search index.",
        "",
        "Usage: gwtd index <subcommand> [args]",
        "",
        "Subcommands:",
        "  status                                  Show index runtime + asset status",
        "  rebuild [--scope all|issues|specs|files|files-docs]",
        "                                          Rebuild a specific scope",
        "",
    ]
    .join("\n")
}

fn format_discuss_help() -> String {
    [
        "gwtd discuss — gwt-discussion exit CLI (SPEC-1935).",
        "",
        "Usage: gwtd discuss <action> --proposal <label>",
        "",
        "Actions:",
        "  resolve --proposal <label>              Mark a proposal chosen",
        "  park --proposal <label>                 Mark a proposal parked",
        "  reject --proposal <label>               Mark a proposal rejected",
        "  clear-next-question --proposal <label>  Clear the open question (Stop unblock)",
        "",
    ]
    .join("\n")
}

fn format_plan_help() -> String {
    [
        "gwtd plan — gwt-plan-spec exit CLI (SPEC-1935).",
        "",
        "Usage: gwtd plan <action> --spec <n> [...]",
        "",
        "Actions:",
        "  start --spec <n>                       Mark plan-spec started",
        "  phase --spec <n> --label <stage>       Mark a phase milestone",
        "  complete --spec <n>                    Mark plan-spec complete",
        "  abort --spec <n> [--reason <text>]     Abort plan-spec",
        "",
    ]
    .join("\n")
}

fn format_build_help() -> String {
    [
        "gwtd build — gwt-build-spec exit CLI (SPEC-1935).",
        "",
        "Usage: gwtd build <action> --spec <n> [...]",
        "",
        "Actions:",
        "  start --spec <n>                       Mark build-spec started",
        "  phase --spec <n> --label <stage>       Mark a phase milestone (red/green/...)",
        "  complete --spec <n>                    Mark build-spec complete",
        "  abort --spec <n> [--reason <text>]     Abort build-spec",
        "",
    ]
    .join("\n")
}

fn format_update_help() -> String {
    [
        "gwtd update — Check / apply gwt updates.",
        "",
        "Usage: gwtd update [--check]",
        "",
        "Flags:",
        "  --check                                 Only check, do not download/apply",
        "  (no flag)                               Check and prompt to apply",
        "",
    ]
    .join("\n")
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
    // Issue #2054: scan every remote (not just `origin`) and honour
    // `GWT_GITHUB_REPO` / `GWT_REMOTE` overrides so multi-remote repos
    // (local mirror + GitHub under a non-origin name) can still resolve.
    if let Some(slug) = std::env::var("GWT_GITHUB_REPO")
        .ok()
        .filter(|v| !v.is_empty())
    {
        if let Some(parsed) = parse_owner_repo(slug.trim_end_matches(".git")) {
            return Some(parsed);
        }
    }

    let remotes = load_remote_pairs();

    if let Some(name) = std::env::var("GWT_REMOTE").ok().filter(|v| !v.is_empty()) {
        if let Some((_, url)) = remotes.iter().find(|(remote_name, _)| remote_name == &name) {
            if let Some(parsed) = parse_github_remote_url(url) {
                return Some(parsed);
            }
        }
    }

    if let Some((_, url)) = remotes.iter().find(|(name, _)| name == "origin") {
        if let Some(parsed) = parse_github_remote_url(url) {
            return Some(parsed);
        }
    }

    remotes
        .iter()
        .find_map(|(_, url)| parse_github_remote_url(url))
}

fn load_remote_pairs() -> Vec<(String, String)> {
    let Ok(output) = gwt_core::process::hidden_command("git")
        .args(["remote", "-v"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let Some(url) = parts.next() else { continue };
        if !seen.insert(name.to_string()) {
            continue;
        }
        out.push((name.to_string(), url.to_string()));
    }
    out
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
