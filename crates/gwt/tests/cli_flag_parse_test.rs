//! SPEC #2920 Phase 4 partial (T-025) — integration test for the
//! tray-route argv parser that restores `--bind` / `--port`. The unit
//! tests in `crates/gwt/src/cli/tray/mod.rs` cover individual cases; this
//! integration test pins the public surface (`parse_tray_argv`,
//! `TrayArgs`, `TrayArgParseError`) so removing or renaming any of them
//! is a visible change.

use std::net::{IpAddr, Ipv4Addr};

use gwt::cli::tray::{parse_tray_argv, TrayArgParseError, TrayArgs};

#[test]
fn parse_tray_argv_matches_default_with_no_flags() {
    let args = parse_tray_argv(&[String::from("gwt")]).expect("empty argv parses");
    assert_eq!(args, TrayArgs::default());
}

#[test]
fn parse_tray_argv_distinguishes_omitted_port_from_explicit_zero() {
    let omitted = parse_tray_argv(&[String::from("gwt")]).expect("empty argv parses");
    let explicit_zero = parse_tray_argv(&[
        String::from("gwt"),
        String::from("--port"),
        String::from("0"),
    ])
    .expect("explicit ephemeral port parses");

    assert_ne!(
        omitted.port, explicit_zero.port,
        "omitted --port must remain distinguishable from explicit --port 0"
    );
}

#[test]
fn parse_tray_argv_restores_external_bind_and_fixed_port() {
    // Exact shape a VPN-reachable remote host should be able to run after
    // SPEC #2920 Phase 4 partial lands. The browser URL emitted to stderr
    // becomes `http://0.0.0.0:60745/` once main.rs forwards these values
    // to `EmbeddedServer::start_with_bind`.
    let argv: Vec<String> = ["gwt", "--bind", "0.0.0.0", "--port", "60745"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let args = parse_tray_argv(&argv).expect("external bind parses");
    assert_eq!(args.bind, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    assert_eq!(args.port, Some(60745));
}

#[test]
fn parse_tray_argv_surfaces_invalid_input_as_typed_errors() {
    let bad_ip: Vec<String> = ["gwt", "--bind", "not-an-ip"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let err = parse_tray_argv(&bad_ip).expect_err("invalid IP must error");
    assert!(
        matches!(err, TrayArgParseError::InvalidIp(ref v) if v == "not-an-ip"),
        "unexpected error: {err:?}"
    );

    let bad_port: Vec<String> = ["gwt", "--port", "99999"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let err = parse_tray_argv(&bad_port).expect_err("port out of range must error");
    assert!(
        matches!(err, TrayArgParseError::InvalidPort(ref v) if v == "99999"),
        "unexpected error: {err:?}"
    );
}
