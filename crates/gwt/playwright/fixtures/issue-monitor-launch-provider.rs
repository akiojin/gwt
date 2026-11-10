// Offline Windows command fixture. Provider behavior is the existing recorder;
// gh/git never delegate to an installed provider or an external Git remote.
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn rest_issue(issue: &serde_json::Value) -> serde_json::Value {
    let mut issue = issue.clone();
    issue["state"] = serde_json::json!("open");
    issue["updated_at"] = issue["updatedAt"].clone();
    issue["html_url"] = issue["url"].clone();
    issue["user"] = serde_json::json!({"login":"fixture"});
    issue
}

fn rest_read(path: &str, issues: &serde_json::Value) -> Option<serde_json::Value> {
    let path = path.trim_start_matches('/').split('?').next()?;
    let route = path
        .strip_prefix("repos/fixture/launch-perf/")
        .or_else(|| path.strip_prefix("repos/{owner}/{repo}/"))?;
    let parts = route.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["issues"] => Some(serde_json::Value::Array(
            issues.as_array()?.iter().map(rest_issue).collect(),
        )),
        ["issues", number] => issues
            .as_array()?
            .iter()
            .find(|issue| issue["number"].as_u64() == number.parse().ok())
            .map(rest_issue),
        ["issues", number, "comments"] => {
            let issue = issues
                .as_array()?
                .iter()
                .find(|issue| issue["number"].as_u64() == number.parse().ok())?;
            Some(serde_json::Value::Array(
                issue["comments"]
                    .as_array()?
                    .iter()
                    .map(|comment| {
                        serde_json::json!({
                            "id":comment["databaseId"], "body":comment["body"],
                            "created_at":comment["createdAt"], "updated_at":comment["updatedAt"],
                            "user":{"login":"fixture"}
                        })
                    })
                    .collect(),
            ))
        }
        ["pulls"] | ["commits"] => Some(serde_json::json!([])),
        _ => None,
    }
}

fn graphql_read(args: &[String], issues: &serde_json::Value) -> Option<serde_json::Value> {
    let field = |key: &str| {
        args.windows(2)
            .filter(|pair| ["-f", "--raw-field", "-F", "--field"].contains(&pair[0].as_str()))
            .find_map(|pair| pair[1].strip_prefix(key))
    };
    let query = field("query=")?;
    if !query.trim_start().starts_with("query(")
        || query
            .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
            .any(|word| word == "mutation")
        || !query.contains("timelineItems(")
        || field("owner=") != Some("fixture")
        || field("repo=") != Some("launch-perf")
    {
        return None;
    }
    let mut repository = serde_json::Map::new();
    for issue in issues.as_array()? {
        let number = issue["number"].as_u64()?;
        let alias = format!("i{number}");
        if query.contains(&format!("{alias}: issue(number: {number})")) {
            repository.insert(
                alias,
                serde_json::json!({
                    "timelineItems":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}
                }),
            );
        }
    }
    if repository.is_empty() {
        return None;
    }
    Some(serde_json::json!({"data":{
        "repository":repository,
        "rateLimit":{"cost":1,"remaining":5000,"resetAt":"2099-01-01T00:00:00Z","nodeCount":0}
    }}))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().unwrap();
    let name = exe.file_stem().unwrap().to_string_lossy();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    if name == "git" {
        if args
            .windows(3)
            .any(|a| a == ["remote", "get-url", "origin"])
        {
            println!("https://github.com/fixture/launch-perf.git");
            return;
        }
        let status = Command::new(std::env::var_os("GWT5596_REAL_GIT").unwrap())
            .args(args)
            .status()
            .unwrap();
        std::process::exit(status.code().unwrap_or(1));
    }
    if name == "gh" {
        let issues: serde_json::Value =
            serde_json::from_slice(&std::fs::read(home.join("github-issues.json")).unwrap())
                .unwrap();
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.join("gh-argv.jsonl"))
            .unwrap();
        writeln!(log, "{}", serde_json::json!(args)).unwrap();
        if args.first().map(String::as_str) == Some("--version") {
            println!("gh version 2.80.0");
            return;
        }
        if args.first().map(String::as_str) == Some("auth") {
            println!("fixture-only");
            return;
        }
        let result = match args.first().map(String::as_str) {
            Some("issue") if args.get(1).map(String::as_str) == Some("view") => {
                let number = args.get(2).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                issues
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|i| i["number"] == number)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null)
            }
            Some("issue") if args.get(1).map(String::as_str) == Some("list") => issues,
            Some("repo") if args.get(1).map(String::as_str) == Some("view") => {
                serde_json::json!({"nameWithOwner":"fixture/launch-perf","defaultBranchRef":{"name":"develop"}})
            }
            Some("pr") if args.get(1).map(String::as_str) == Some("list") => serde_json::json!([]),
            Some("api")
                if !args
                    .iter()
                    .any(|a| ["--method", "-X"].contains(&a.as_str())) =>
            {
                let result = if args.get(1).map(String::as_str) == Some("graphql") {
                    graphql_read(&args, &issues)
                } else {
                    args.get(1).and_then(|path| rest_read(path, &issues))
                };
                let Some(result) = result else {
                    eprintln!("unsupported offline gh read endpoint");
                    std::process::exit(64);
                };
                if args.iter().any(|arg| arg == "--include") {
                    print!("HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n");
                }
                result
            }
            _ => {
                eprintln!("unsupported offline gh command");
                std::process::exit(64);
            }
        };
        println!("{result}");
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("codex-cli 0.116.0");
        return;
    }
    // The configuration preflight runs before a provider Session exists.
    if args.windows(2).any(|pair| pair == ["features", "list"]) {
        println!("shell_tool\tstable\ttrue");
        return;
    }
    let session = std::env::var("GWT_SESSION_ID").unwrap();
    // Each PTY owns one receipt. Concurrent writes to a shared JSONL file can
    // interleave on Windows, which would corrupt the workload's own evidence.
    let pending = home.join(format!("provider-launch-{session}.tmp"));
    let receipt = home.join(format!("provider-launch-{session}.json"));
    std::fs::write(&pending, serde_json::json!({"pid":std::process::id(),"session_id":session,"argv":args,"cwd":std::env::current_dir().unwrap()}).to_string()).unwrap();
    std::fs::rename(pending, receipt).unwrap();
    println!("FIXTURE_WAITING_SESSION_START {session}");
    while !home.join(format!("ready-{session}")).exists() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut cli_success = true;
    let profile = home.join(format!("hook-profile-{session}.jsonl"));
    let mut hook_commands = Vec::new();
    for event in ["SessionStart", "UserPromptSubmit"] {
        let started = Instant::now();
        let mut child = Command::new(std::env::var_os("GWT_BIN_PATH").unwrap())
            .args(["hook", "event", event])
            .env("GWT_HOOK_PROFILE_PATH", &profile)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(serde_json::json!({"session_id":session,"hook_event_name":event,"source":"startup","cwd":std::env::current_dir().unwrap(),"prompt":args.last().unwrap()}).to_string().as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        cli_success &= output.status.success();
        // Protocol output can contain launch credentials. Persist only fixed
        // diagnostic categories and sizes, never stdout/stderr contents.
        let stderr = String::from_utf8_lossy(&output.stderr);
        hook_commands.push(serde_json::json!({
            "event": event, "exit_code": output.status.code(),
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
            "stdout_bytes": output.stdout.len(), "stderr_bytes": output.stderr.len(),
            "runtime_state_error": stderr.contains("runtime-state"),
            "deadline_error": stderr.contains("deadline"),
            "timeout_error": stderr.contains("timeout") || stderr.contains("timed out"),
            "fail_open": stderr.contains("fail-open"),
        }));
        std::fs::write(
            home.join(format!("hook-command-{session}.json")),
            serde_json::to_vec(&hook_commands).unwrap(),
        )
        .unwrap();
    }
    // Hook exit 0 is fail-open: the exact prompt event can still be journaled.
    // Forward flushes that existing event without submitting another prompt.
    let runtime = PathBuf::from(std::env::var_os("GWT_SESSION_RUNTIME_PATH").unwrap());
    assert_eq!(
        runtime.file_stem().unwrap().to_str(),
        Some(session.as_str())
    );
    let host = runtime.parent().unwrap();
    let journal = host
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hook-live")
        .join(host.file_name().unwrap())
        .join(runtime.file_name().unwrap());
    let drained = || {
        std::fs::read(&journal)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|state| {
                state["health"]["pending_runtime_state"] == false && state["pending"].is_null()
            })
    };
    let deadline = Instant::now() + Duration::from_secs(120);
    // A transient CLI error must not prevent replay of the original journal.
    // The E2E also requires the GUI's authenticated and matched prompt receipt.
    while !drained() && Instant::now() < deadline {
        let mut child = Command::new(std::env::var_os("GWT_BIN_PATH").unwrap())
            .args(["__internal", "daemon-hook", "forward"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                serde_json::json!({"session_id":session,"cwd":std::env::current_dir().unwrap()})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        cli_success &= child.wait_with_output().unwrap().status.success();
        std::thread::sleep(Duration::from_millis(100));
    }
    let journal_drained = drained();
    // Hook output may contain launch-scoped credentials; retain success only.
    let hook_pending = home.join(format!("hook-{session}.tmp"));
    std::fs::write(
        &hook_pending,
        serde_json::json!({"success":journal_drained,"cli_success":cli_success,"journal_drained":journal_drained}).to_string(),
    )
    .unwrap();
    std::fs::rename(hook_pending, home.join(format!("hook-{session}.json"))).unwrap();
    println!("FIXTURE_SESSION_START_COMPLETE {session}");
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
