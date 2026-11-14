use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = std::env::current_exe()
        .unwrap()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    if name == "gh" {
        if args.first().map(String::as_str) == Some("--version") {
            println!("gh version 2.80.0");
        } else if args.first().map(String::as_str) == Some("auth") {
            println!("fixture-only");
        } else if args.iter().any(|value| value == "view") {
            println!(
                "{}",
                serde_json::json!({
                    "number": 4964,
                    "title": "manual successor retry fixture",
                    "state": "OPEN",
                    "body": "## Acceptance Criteria\n- [ ] AC-1: fixture",
                    "labels": [],
                    "comments": [],
                    "url": "https://github.com/fixture/fixture/issues/4964",
                })
            );
        } else {
            println!("[]");
        }
        return;
    }
    if args
        .iter()
        .any(|value| value == "--version" || value == "-V")
    {
        println!("codex-cli 0.116.0");
        return;
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let session = std::env::var("GWT_SESSION_ID").unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("provider-launches.jsonl"))
        .unwrap();
    writeln!(
        log,
        "{}",
        serde_json::json!({"pid": std::process::id(), "session_id": session, "argv": args})
    )
    .unwrap();
    println!("FIXTURE_WAITING_SESSION_START {session}");
    while !home.join(format!("ready-{session}")).exists() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let input = serde_json::json!({
        "session_id": "12345678-1234-4234-8234-123456789abc",
        "hook_event_name": "SessionStart",
        "source": "startup",
        "cwd": std::env::current_dir().unwrap(),
    });
    let mut child = Command::new(std::env::var_os("GWT_BIN_PATH").unwrap())
        .args(["hook", "event", "SessionStart"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    std::fs::write(
        home.join(format!("hook-{session}.json")),
        serde_json::json!({
            "success": output.status.success(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
        })
        .to_string(),
    )
    .unwrap();
    println!("FIXTURE_SESSION_START_COMPLETE {session}");
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
