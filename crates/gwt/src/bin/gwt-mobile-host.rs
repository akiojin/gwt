//! Offline PC credential import and explicit configuration inspection.

use gwt::mobile_host::admin::{run, AdminError};
use std::{
    io::Write,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!(
            "gwt-mobile-host: offline PC credential setup\n\
Usage: gwt-mobile-host < request.json\n\
Request: schema_version=1, command.operation=import or inspect.\n\
Both require binding (host_id, coordinator_origin) and project.\n\
Import also requires an absolute private source path without parent components.\n\
Maximum stdin: 16 KiB. No secrets, clock overrides or other arguments accepted.\n\
Storage: trusted HOME/.gwt/mobile/host, outside repositories. All three\n\
directories must already be owned 0700, files regular single-link 0600,\n\
without symlinks or macOS extended ACLs; ancestors must be non-replaceable.\n\
No directories, permissions or config.json selection are changed.\n\
Inspect reads the explicit identity in config.json; after an existing or\n\
uncertain import, configure the issuance receipt's public identity and inspect\n\
before deciding recovery. Never automatically retry, delete or reissue.\n\
No network, registration, enable, device authority or current registry\n\
revocation check is provided. Windows private storage is unsupported."
        );
        return ExitCode::SUCCESS;
    }
    let result = if args.is_empty() {
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
    } else {
        Err(AdminError::InvalidRequest)
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
