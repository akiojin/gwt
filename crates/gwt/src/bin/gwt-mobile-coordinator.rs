//! Offline Host credential administration only. No listener is provided yet.

use gwt::mobile_coordinator::admin::{run, AdminError};
use std::{
    io::Write,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!(
            "gwt-mobile-coordinator: offline Host credential administration\n\
Usage: gwt-mobile-coordinator < request.json\n\
Storage: trusted HOME/.gwt/mobile/coordinator; all three directories must already\n\
be owned 0700, without symlinks or macOS extended ACLs, outside repositories.\n\
No directories or permissions are changed automatically. Keep ancestors trusted.\n\
Request: {{\"schema_version\":1,\"command\":{{\"operation\":\"status\"}}}}\n\
Operations: initialize, status, issue (origin, host_id, projects, expires_at),\n\
rotate (origin, credential_id, expires_at), revoke (credential_id).\n\
IDs are non-nil UUIDs, projects are canonical 16-digit lowercase hex keys,\n\
origin is HTTPS, expires_at is future Unix seconds. Maximum input: 16 KiB.\n\
Receipts contain public metadata and credential file paths only.\n\
Stop the coordinator before administration; uncertain commits require status\n\
inspection before another issuance. No serve, network, Host import or mobile\n\
connection is available. Windows private storage is unsupported."
        );
        return ExitCode::SUCCESS;
    }
    let result = if !args.is_empty() {
        Err(AdminError::InvalidRequest)
    } else {
        gwt_config::home::user_home()
            .ok_or(AdminError::UnsafeHome)
            .and_then(|home| {
                run(std::io::stdin().lock(), &home, || {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|now| now.as_secs())
                        .map_err(|_| AdminError::ClockUnavailable)
                })
            })
    };
    let ok = result.is_ok();
    let response = result.unwrap_or_else(|error| error.response());
    let mut stdout = std::io::stdout().lock();
    if serde_json::to_writer(&mut stdout, &response).is_err() || writeln!(stdout).is_err() || !ok {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
