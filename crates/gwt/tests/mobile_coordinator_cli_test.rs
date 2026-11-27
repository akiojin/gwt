//! Issue #6476: the explicit coordinator process boundary, using private fixtures.
#![cfg(unix)]

use serde_json::{json, Value};
use std::{
    fs,
    future::Future,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    // test-hygiene: allow-wall-clock-deadline hang guard only; assertions use process and I/O completion
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("coordinator process did not respond")
}

struct Fixture {
    home: tempfile::TempDir,
    directory: PathBuf,
    config: Vec<u8>,
    certificate: reqwest::Certificate,
    reservation: Option<TcpListener>,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join(".gwt/mobile/coordinator");
        fs::create_dir_all(&directory).unwrap();
        for name in [".gwt", ".gwt/mobile", ".gwt/mobile/coordinator"] {
            fs::set_permissions(home.path().join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let issued = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
        for (name, bytes) in [
            ("cert.pem", issued.cert.pem()),
            ("key.pem", issued.signing_key.serialize_pem()),
        ] {
            fs::write(directory.join(name), bytes).unwrap();
            fs::set_permissions(directory.join(name), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = serde_json::to_vec(&json!({
            "schema_version": 1, "origin": "https://coordinator.example",
            "bind": reservation.local_addr().unwrap().to_string(),
            "tls_certificate": directory.join("cert.pem").canonicalize().unwrap(),
            "tls_private_key": directory.join("key.pem").canonicalize().unwrap(),
            "limits": {"registrations":64,"peers_per_host":8,"device_messages_per_second":32,
                       "device_burst":64,"host_messages_per_second":256,"host_burst":512}
        }))
        .unwrap();
        Self {
            home,
            directory,
            config,
            certificate: reqwest::Certificate::from_der(issued.cert.der()).unwrap(),
            reservation: Some(reservation),
        }
    }

    fn write_config(&self) {
        fs::write(self.directory.join("config.json"), &self.config).unwrap();
        fs::set_permissions(
            self.directory.join("config.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }

    fn command(&self) -> Command {
        let mut command =
            gwt_core::process::resolved_tokio_command(gwt_core::process::ProcessPlanRequest::new(
                env!("CARGO_BIN_EXE_gwt-mobile-coordinator"),
            ))
            .unwrap();
        command
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .current_dir(self.home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }

    async fn admin(&self, operation: &str) -> (Output, Value) {
        let mut child = self.command().spawn().unwrap();
        let request = json!({"schema_version":1,"command":{"operation":operation}});
        let mut input = child.stdin.take().unwrap();
        input
            .write_all(request.to_string().as_bytes())
            .await
            .unwrap();
        drop(input);
        let output = bounded(child.wait_with_output()).await.unwrap();
        let value = serde_json::from_slice(&output.stdout).unwrap();
        (output, value)
    }
}

#[tokio::test]
async fn explicit_serve_denies_http_and_releases_registry_on_both_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let mut fixture = Fixture::new();
        fixture.write_config();
        assert!(fixture.admin("initialize").await.0.status.success());
        drop(fixture.reservation.take());
        let mut child = fixture.command().arg("serve").spawn().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let ready: Value = serde_json::from_str(
            &bounded(stdout.next_line())
                .await
                .unwrap()
                .expect("readiness"),
        )
        .unwrap();
        assert_eq!(ready["ok"], true, "{ready}");
        assert_eq!(ready["operation"], "serve");
        assert_eq!(ready["status"], "listening");
        let address = ready["address"].as_str().unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(fixture.certificate.clone())
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        for path in ["/", "/ws", "/p/0123456789abcdef"] {
            let response = bounded(client.get(format!("https://{address}{path}")).send())
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
            assert!(response.bytes().await.unwrap().is_empty());
        }
        let (output, busy) = fixture.admin("status").await;
        assert!(!output.status.success());
        assert_eq!(busy["code"], "busy");
        // This PID belongs to the child created above, never a shared process.
        assert_eq!(unsafe { libc::kill(child.id().unwrap() as i32, signal) }, 0);
        let output = bounded(child.wait_with_output()).await.unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(fixture.admin("status").await.0.status.success());
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }
}

#[tokio::test]
async fn serve_rejects_missing_state_and_requires_an_explicit_exact_argument() {
    let fixture = Fixture::new();
    for config_present in [false, true] {
        if config_present {
            fixture.write_config();
        }
        let output = bounded(fixture.command().arg("serve").output())
            .await
            .unwrap();
        assert!(!output.status.success());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["code"], "serve_failed", "{response}");
        assert!(output.stderr.is_empty());
        assert!(!String::from_utf8_lossy(&output.stdout)
            .contains(fixture.home.path().to_str().unwrap()));
        assert!(!fixture.directory.join("registry.json").exists());
    }
    assert!(fixture.admin("initialize").await.0.status.success());
    assert!(fixture.admin("status").await.0.status.success());
    for args in [vec!["serve", "--bind", "127.0.0.1:1"], vec!["unknown"]] {
        let output = bounded(fixture.command().args(args).output())
            .await
            .unwrap();
        assert!(!output.status.success());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["code"], "invalid_request");
    }
    let output = bounded(fixture.command().arg("--help").output())
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("gwt-mobile-coordinator serve"));
}
