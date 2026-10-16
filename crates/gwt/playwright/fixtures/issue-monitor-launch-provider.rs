// Offline Windows command fixture. Provider behavior is the existing recorder;
// gh/git never delegate to an installed provider or an external Git remote.
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

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
                serde_json::json!([])
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
    let mut child = Command::new(std::env::var_os("GWT_BIN_PATH").unwrap())
        .args(["hook", "event", "SessionStart"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(serde_json::json!({"session_id":session,"hook_event_name":"SessionStart","source":"startup","cwd":std::env::current_dir().unwrap()}).to_string().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    // Hook output may contain launch-scoped credentials; retain success only.
    std::fs::write(
        home.join(format!("hook-{session}.json")),
        serde_json::json!({"success":output.status.success()}).to_string(),
    )
    .unwrap();
    println!("FIXTURE_SESSION_START_COMPLETE {session}");
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
