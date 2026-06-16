use std::{io::IsTerminal, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("-V" | "--version" | "version") => {
            println!("gwtd {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") => {
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
        None if std::io::stdin().is_terminal() => {
            print_help();
            return ExitCode::SUCCESS;
        }
        None => {}
        _ => {}
    }

    let code = match argv.get(1).map(String::as_str) {
        None => run_json_envelope_cli(&argv),
        Some(_) if is_allowed_argv_exception(&argv) => {
            let mut env = gwt::cli::DefaultCliEnv::new_for_hooks();
            gwt::cli::dispatch(&mut env, &argv)
        }
        _ => {
            eprint!("{}", json_only_argv_message(&argv));
            2
        }
    };
    ExitCode::from(code.clamp(0, 255) as u8)
}

fn print_help() {
    println!("gwtd {}", env!("CARGO_PKG_VERSION"));
    println!("Headless CLI for gwt agent, hook, and workflow automation.");
    println!();
    println!("Usage: gwtd < stdin-json-envelope");
    println!("       gwtd hook event <Event>   Managed hook transport exception");
    println!("       gwtd --help <command>   Show subcommands for a family");
    println!();
    println!("Commands:");
    println!("  issue       Manage GitHub Issues and SPEC sections");
    println!("  pr          Manage pull requests, reviews, checks, threads");
    println!("  actions     Fetch GitHub Actions run/job logs");
    println!("  board       Read/write the coordination Board (SPEC-1974)");
    println!("  hook        Dispatch Claude Code / Codex hook events");
    println!("  index       Manage the local search index");
    println!("  search      Semantic search over SPECs, Issues, files, memory");
    println!("  memory      Append reusable project memory");
    println!("  lessons     Legacy alias for memory add");
    println!("  discuss     gwt-discussion exit CLI (SPEC-1935)");
    println!("  discussion  Persist/update Git-managed discussion notes");
    println!("  plan        gwt-plan-spec exit CLI (SPEC-1935)");
    println!("  build       gwt-build-spec exit CLI (SPEC-1935)");
    println!("  register    gwt-register-spec exit CLI (SPEC-2784)");
    println!("  pane        Inspect and control live agent panes");
    println!("  workspace   Update Work current projection and summary journal");
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
        "search" => Some(format_search_help()),
        "memory" | "lessons" => Some(format_memory_help()),
        "discuss" => Some(format_discuss_help()),
        "discussion" => Some(format_discussion_help()),
        "plan" => Some(format_plan_help()),
        "build" => Some(format_build_help()),
        "register" => Some(format_register_help()),
        "pane" => Some(format_pane_help()),
        "workspace" => Some(format_workspace_help()),
        "update" => Some(format_update_help()),
        "daemon" => Some(format_daemon_help()),
        _ => None,
    }
}

fn format_workspace_help() -> String {
    [
        "workspace.* — Update Work current projection and summary journal via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"<work purpose>\",\"current_focus\":\"<current work>\"}}",
        "  JSON",
        "",
        "Operations:",
        "  workspace.update                       Set Work status fields and Agent purpose/focus",
        "  workspace.create | workspace.ensure    Create or ensure a Work assignment",
        "  workspace.join | workspace.candidates  Join/list Work candidates",
        "",
        "Key params:",
        "  purpose                                Short Agent/window title purpose",
        "  current_focus                          Current phase/activity",
        "  agent_session                          Defaults to GWT_SESSION_ID when omitted",
        "",
    ]
    .join("\n")
}

fn format_daemon_help() -> String {
    [
        "daemon.* — Long-running runtime daemon operations via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"daemon.status\",\"params\":{}}",
        "  JSON",
        "",
        "Operations:",
        "  daemon.start                            Bootstrap and serve the runtime daemon",
        "  daemon.status                           Probe the daemon endpoint",
        "  daemon.subscribe                         Subscribe to daemon broadcast channels",
        "",
        "Key params:",
        "  channels                                Required for daemon.subscribe",
        "",
        "Notes:",
        "  - Listens on a Unix domain socket per RuntimeScope (POSIX only today).",
        "  - Endpoint metadata is persisted under ~/.gwt/projects/<repo>/runtime/daemon/.",
        "  - SIGINT / SIGTERM trigger graceful shutdown + endpoint file removal.",
        "  - `status` reports `probe=ok uptime=<s>s channels=<n> connections=<n>` when the",
        "    daemon answers a `ClientFrame::Status` request within 1s, or `probe=failed:<reason>`",
        "    when the endpoint file is stale or unreachable.",
        "",
    ]
    .join("\n")
}

fn format_issue_help() -> String {
    [
        "issue.* — Manage GitHub Issues and SPEC sections via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"issue.view\",\"params\":{\"number\":123}}",
        "  JSON",
        "",
        "Operations:",
        "  issue.view | issue.comments | issue.linked_prs",
        "  issue.create | issue.comment",
        "  issue.spec.read | issue.spec.section | issue.spec.edit",
        "  issue.spec.create | issue.spec.list | issue.spec.pull",
        "  issue.spec.repair | issue.spec.rename",
        "",
        "Key params:",
        "  number, title, section, body, labels, refresh",
        "  structured                             Treat issue.spec body as structured JSON",
        "  replace                                Replace structured SPEC section instead of merging",
        "  all, numbers                           Controls issue.spec.pull",
        "",
    ]
    .join("\n")
}

fn format_pr_help() -> String {
    [
        "pr.* — Manage pull requests, reviews, checks, and threads via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"pr.view\",\"params\":{\"number\":123}}",
        "  JSON",
        "",
        "Operations:",
        "  pr.current | pr.view | pr.checks | pr.reviews | pr.review_threads",
        "  pr.create | pr.edit | pr.comment | pr.review_threads.reply_and_resolve",
        "",
        "Key params:",
        "  number, base, head, title, body, labels, add_labels, draft",
        "",
    ]
    .join("\n")
}

fn format_actions_help() -> String {
    [
        "actions.* — Fetch GitHub Actions run/job logs via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"actions.logs\",\"params\":{\"run_id\":123}}",
        "  JSON",
        "",
        "Operations:",
        "  actions.logs                            Print raw run logs",
        "  actions.job_logs                        Print raw job logs",
        "",
        "Key params:",
        "  run_id, job_id",
        "",
    ]
    .join("\n")
}

fn format_board_help() -> String {
    [
        "board.* — Read/write the coordination Board (SPEC-1974) via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"status\",\"body\":\"Current state: ...\"}}",
        "  JSON",
        "",
        "Operations:",
        "  board.show                              Print the Board snapshot",
        "  board.post                              Append a Board entry",
        "",
        "Key params:",
        "  kind, body, title, topics, owners, targets, mentions, parent, broadcast",
        "  workspace, all                           board.show filters",
        "",
        "Note: board.post does not accept purpose/title_summary; update Agent title",
        "      through workspace.update params.purpose.",
        "",
        "Kinds: request, status, next, claim, impact, question, blocked, handoff, decision",
        "",
    ]
    .join("\n")
}

fn format_hook_help() -> String {
    [
        "gwtd hook — Managed hook transport exception (SPEC-1935).",
        "",
        "Usage: gwtd hook event <Event>",
        "",
        "Hook events:",
        "  PreToolUse, UserPromptSubmit, Stop, SessionStart, and provider-specific names",
        "",
        "Stdin: hooks read provider JSON from stdin; the event name is the only argv field.",
        "",
    ]
    .join("\n")
}

fn format_index_help() -> String {
    [
        "index.* — Manage the local search index via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"index.status\",\"params\":{}}",
        "  JSON",
        "",
        "Operations:",
        "  index.status                            Show index runtime and asset status",
        "  index.rebuild                           Rebuild a specific scope",
        "",
        "Key params:",
        "  scope                                   all|issues|specs|memory|discussions|board|files|files-docs",
        "                                          JSON also accepts files_docs",
        "",
    ]
    .join("\n")
}

fn format_memory_help() -> String {
    [
        "memory.* — Append reusable project memory via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"memory.add\",\"params\":{\"title\":\"...\",\"context\":\"...\",\"learning\":\"...\",\"future_action\":\"...\"}}",
        "  JSON",
        "",
        "Operation:",
        "  memory.add",
        "",
        "Key params:",
        "  date, type, title, context, learning, future_action",
        "",
    ]
    .join("\n")
}

fn format_discussion_help() -> String {
    [
        "discussion.* — Persist/update Git-managed discussion notes via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"discussion.update\",\"params\":{\"title\":\"...\",\"summary\":\"...\",\"next\":\"...\"}}",
        "  JSON",
        "",
        "Operation:",
        "  discussion.update",
        "",
        "Key params:",
        "  date, title, status, topics, related_specs, related_works",
        "  promoted_to, summary, decisions, open_questions, next",
        "",
    ]
    .join("\n")
}

fn format_discuss_help() -> String {
    [
        "discuss.* — gwt-discussion exit operations via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"discuss.resolve\",\"params\":{\"proposal\":\"A\"}}",
        "  JSON",
        "",
        "Operations:",
        "  discuss.resolve | discuss.park | discuss.reject",
        "  discuss.clear_next_question",
        "  discuss.goal_pending | discuss.goal_started",
        "  discuss.goal_failed | discuss.goal_skipped",
        "",
        "Key params:",
        "  proposal, condition, reason",
        "",
    ]
    .join("\n")
}

fn format_plan_help() -> String {
    [
        "plan.* — gwt-plan-spec state operations via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"plan.phase\",\"params\":{\"spec\":1942,\"label\":\"research\"}}",
        "  JSON",
        "",
        "Operations:",
        "  plan.start | plan.phase | plan.complete | plan.abort",
        "",
        "Key params:",
        "  spec, label, reason",
        "",
    ]
    .join("\n")
}

fn format_build_help() -> String {
    [
        "build.* — gwt-build-spec state operations via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"build.phase\",\"params\":{\"spec\":1942,\"label\":\"green\"}}",
        "  JSON",
        "",
        "Operations:",
        "  build.start | build.phase | build.complete | build.abort",
        "",
        "Key params:",
        "  spec, label, reason",
        "",
    ]
    .join("\n")
}

fn format_register_help() -> String {
    [
        "register.* — gwt-register-spec state operations via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"register.phase\",\"params\":{\"spec\":2784,\"label\":\"roundtrip\"}}",
        "  JSON",
        "",
        "Operations:",
        "  register.start | register.phase | register.complete | register.abort",
        "",
        "Key params:",
        "  spec, label, reason",
        "",
    ]
    .join("\n")
}

fn format_pane_help() -> String {
    [
        "pane.* — Inspect and control live agent panes via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"pane.read\",\"params\":{\"id\":\"pane-id\",\"lines\":200}}",
        "  JSON",
        "",
        "Operations:",
        "  pane.list | pane.read | pane.close | pane.stop | pane.send",
        "",
        "Key params:",
        "  id, lines, text",
        "",
        "Notes:",
        "  - Intended for gwt-launched agent panes with GWT_HOOK_FORWARD_URL set.",
        "  - GWT_PANE_WS_URL can override the derived WebSocket endpoint for tests.",
        "  - send is self-only: it targets the pane bound to GWT_SESSION_ID and",
        "    rejects panes owned by other sessions (SPEC-3050).",
        "",
    ]
    .join("\n")
}

/// User-facing top-level verbs eligible for did-you-mean suggestions. Keep in
/// sync with `print_help`; `suggestion_verbs_are_all_dispatchable` asserts
/// every entry is accepted by `gwt::cli::should_dispatch_cli`.
const SUGGESTION_VERBS: &[&str] = &[
    "issue",
    "pr",
    "actions",
    "board",
    "hook",
    "index",
    "search",
    "memory",
    "lessons",
    "discuss",
    "discussion",
    "plan",
    "build",
    "register",
    "pane",
    "workspace",
    "update",
    "daemon",
];

/// SPEC-1942 FR-109: render the stderr message for an unknown top-level verb.
fn unknown_command_message(unknown: &str) -> String {
    let mut message = format!("gwtd: unknown command '{unknown}'\n");
    if let Some(suggestion) = did_you_mean(unknown) {
        message.push_str(&format!("did you mean '{suggestion}'?\n"));
    }
    message.push('\n');
    message.push_str("Usage: gwtd < stdin-json-envelope\n");
    message.push_str("Run 'gwtd --help' for the full command list.\n");
    message
}

/// Suggest the closest known verb for a mistyped one (edit distance <= 2).
fn did_you_mean(input: &str) -> Option<&'static str> {
    if input.chars().count() < 3 {
        return None;
    }
    SUGGESTION_VERBS
        .iter()
        .map(|verb| (levenshtein(input, verb), *verb))
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, verb)| verb)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

fn format_search_help() -> String {
    [
        "search — Semantic search over SPECs, Issues, files, board, and memory via JSON envelope.",
        "",
        "Usage:",
        "  gwtd <<'JSON'",
        "  {\"schema_version\":1,\"operation\":\"search\",\"params\":{\"query\":\"agent title\",\"scopes\":[\"specs\",\"files\"],\"n_results\":8}}",
        "  JSON",
        "",
        "Operation:",
        "  search",
        "",
        "Key params:",
        "  query, scopes, match_mode, n_results",
        "  scopes                                  specs, issues, files, files_docs, memory, board, discussions",
        "",
        "Notes:",
        "  - Missing indexes are built automatically on first search (auto-build).",
        "  - Run from inside the target project; the repo is resolved from cwd.",
        "",
    ]
    .join("\n")
}

fn format_update_help() -> String {
    [
        "gwtd update — Application-owned updater.",
        "",
        "The updater replaces or exits the current process and is not used by LLM JSON envelope workflows.",
        "",
    ]
    .join("\n")
}

fn run_json_envelope_cli(argv: &[String]) -> i32 {
    let repo_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some((owner, repo)) = resolve_repo_coordinates() {
        let mut env = gwt::cli::DefaultCliEnv::new(&owner, &repo, repo_path);
        return gwt::cli::dispatch(&mut env, argv);
    }

    let mut env = gwt::cli::DefaultCliEnv::new_for_hooks();
    gwt::cli::dispatch(&mut env, argv)
}

fn is_allowed_argv_exception(argv: &[String]) -> bool {
    matches!(argv.get(1).map(String::as_str), Some("__internal"))
        || matches!(
            (
                argv.get(1).map(String::as_str),
                argv.get(2).map(String::as_str),
                argv.get(3),
                argv.get(4),
            ),
            (Some("hook"), Some("event"), Some(_), None)
        )
        || matches!(
            (
                argv.get(1).map(String::as_str),
                argv.get(2).map(String::as_str),
                argv.get(3),
                argv.get(4),
                argv.get(5),
            ),
            (Some("hook"), Some("provider-event"), Some(_), Some(_), None)
        )
}

fn json_only_argv_message(argv: &[String]) -> String {
    let verb = argv.get(1).map(String::as_str).unwrap_or("");
    let mut message = String::new();
    if verb.is_empty() {
        message.push_str("gwtd expects a stdin JSON envelope.\n");
    } else if gwt::cli::should_dispatch_cli(argv) {
        message.push_str(&format!(
            "gwtd {verb}: legacy argv invocation is disabled; use stdin JSON envelope.\n"
        ));
    } else {
        message.push_str(&unknown_command_message(verb));
        return message;
    }
    message.push_str("Usage: gwtd < stdin JSON envelope\n");
    message.push_str(
        "Example: {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"<work purpose>\",\"current_focus\":\"<focus>\"}}\n",
    );
    message.push_str("Managed hook transport exception: gwtd hook event <Event>\n");
    message
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_you_mean_suggests_search_for_typo() {
        // The motivating misuse: `gwtd serach`-style typos and invented verbs
        // must point at the real `search` family (SPEC-1942 FR-109).
        assert_eq!(did_you_mean("serach"), Some("search"));
        assert_eq!(did_you_mean("baord"), Some("board"));
    }

    #[test]
    fn did_you_mean_rejects_unrelated_input() {
        assert_eq!(did_you_mean("frobnicate"), None);
        assert_eq!(did_you_mean(""), None);
        assert_eq!(did_you_mean("xy"), None);
    }

    #[test]
    fn unknown_command_message_includes_suggestion_and_help_pointer() {
        let message = unknown_command_message("serach");
        assert!(
            message.contains("unknown command 'serach'"),
            "missing unknown-command line: {message}"
        );
        assert!(
            message.contains("did you mean 'search'"),
            "missing did-you-mean line: {message}"
        );
        assert!(
            message.contains("gwtd --help"),
            "missing help pointer: {message}"
        );
    }

    #[test]
    fn unknown_command_message_without_suggestion_still_points_to_help() {
        let message = unknown_command_message("frobnicate");
        assert!(!message.contains("did you mean"), "unexpected: {message}");
        assert!(
            message.contains("gwtd --help"),
            "missing help pointer: {message}"
        );
        assert!(
            message.contains("Usage: gwtd < stdin-json-envelope"),
            "missing usage line: {message}"
        );
    }

    #[test]
    fn suggestion_verbs_are_all_dispatchable() {
        for verb in SUGGESTION_VERBS {
            assert!(
                gwt::cli::should_dispatch_cli(&["gwtd".to_string(), (*verb).to_string()]),
                "suggestion verb '{verb}' must be accepted by should_dispatch_cli"
            );
        }
    }

    #[test]
    fn format_search_help_documents_json_params() {
        let help = format_search_help();
        for expected in [
            r#""operation":"search""#,
            "query",
            "scopes",
            "files_docs",
            "match_mode",
            "n_results",
        ] {
            assert!(
                help.contains(expected),
                "search help must document JSON param {expected}. help:\n{help}",
            );
        }
    }

    #[test]
    fn family_help_resolves_search() {
        assert!(family_help("search").is_some());
    }

    #[test]
    fn format_board_help_documents_mentions_param() {
        let help = format_board_help();
        assert!(
            help.contains("mentions"),
            "board help must document mentions JSON param. help:\n{help}",
        );
    }

    #[test]
    fn format_board_help_documents_post_audience_params() {
        let help = format_board_help();
        for expected in ["targets", "owners", "topics", "parent"] {
            assert!(
                help.contains(expected),
                "board help must document {expected} JSON param. help:\n{help}",
            );
        }
    }
}
