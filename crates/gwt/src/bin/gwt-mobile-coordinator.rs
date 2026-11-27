//! Offline Host credential administration and explicit default-deny HTTPS startup.

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
            "gwt-mobile-coordinator: Host credential administration and HTTPS startup\n\
Usage: gwt-mobile-coordinator < request.json\n\
       gwt-mobile-coordinator serve\n\
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
inspection before another issuance. Explicit serve loads fixed config.json,\n\
private TLS files and an existing registry, then listens on the configured bind.\n\
All HTTP requests return 404; registration, signaling and mobile UI are unavailable.\n\
SIGINT/SIGTERM stop accepting and drain connections without a forced deadline.\n\
No automatic initialization or permission repair. Windows private storage is unsupported."
        );
        return ExitCode::SUCCESS;
    }
    if args.len() == 1 && args[0] == "serve" {
        return match serve() {
            Ok(()) => ExitCode::SUCCESS,
            Err(diagnostic) => {
                let _ = write_response(&serde_json::json!({
                    "ok":false, "code":"serve_failed", "diagnostic":diagnostic
                }));
                ExitCode::FAILURE
            }
        };
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
    if write_response(&response).is_err() || !ok {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn write_response(response: &serde_json::Value) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, response)?;
    writeln!(stdout)?;
    stdout.flush()
}

#[cfg(unix)]
fn serve() -> Result<(), String> {
    use gwt::mobile_coordinator::server::CoordinatorServer;
    use tokio::signal::unix::{signal, SignalKind};

    let home = gwt_config::home::user_home().ok_or_else(|| {
        "trusted user home is unavailable; repair HOME and retry startup".to_owned()
    })?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            "coordinator runtime is unavailable; repair host resources and retry startup"
        })?;
    runtime.block_on(async {
        // Install both handlers before binding or announcing readiness.
        let mut interrupt = signal(SignalKind::interrupt()).map_err(|_| {
            "coordinator signal setup failed; repair host resources and retry startup"
        })?;
        let mut terminate = signal(SignalKind::terminate()).map_err(|_| {
            "coordinator signal setup failed; repair host resources and retry startup"
        })?;
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                "trusted Unix time is unavailable; repair the host clock and retry startup"
            })?
            .as_secs();
        let server = CoordinatorServer::bind_from_home(&home, wall)
            .await
            .map_err(|error| error.to_string())?;
        let address = server.local_addr().map_err(|error| error.to_string())?;
        write_response(&serde_json::json!({
            "ok":true, "operation":"serve", "status":"listening", "address":address
        }))
        .map_err(|_| "coordinator output is unavailable; restore stdout and retry startup")?;
        server
            .serve_until(async move {
                tokio::select! {
                    _ = interrupt.recv() => {},
                    _ = terminate.recv() => {},
                }
            })
            .await
            .map_err(|error| error.to_string())
    })
}

#[cfg(not(unix))]
fn serve() -> Result<(), String> {
    Err(gwt::mobile_coordinator::ConfigError::UnsupportedPrivateStorage.to_string())
}
