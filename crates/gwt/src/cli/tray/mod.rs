//! SPEC #2920 — Tray-only browser front door.
//!
//! Replaces the wry/tao native WebView GUI route with a tray-resident
//! process that owns the embedded server and exposes an `Open` menu entry
//! to launch the default browser. This module is the runtime entry point
//! when `gwt` is invoked with no CLI verb (FrontDoorRoute::Tray).
//!
//! Phase 1 ships only the type and module skeletons; the actual event loop
//! and EmbeddedServer integration land in Phase 4. Until then, `run()`
//! returns `Err(TrayError::NotYetImplemented)` so we never silently take
//! over the GUI route.

use std::net::{IpAddr, Ipv4Addr};

pub mod autostart;
pub mod lock;
pub mod menu;
pub mod port;

/// SPEC #2920 FR-004: launch the OS default browser for the given URL.
/// Shared by the tray `Open` menu handler (main.rs event loop) and the
/// `gwt open` CLI (Phase 6). The launcher is detached so callers do
/// not block on the spawned process.
pub fn open_browser_for_url(url: &str) -> std::io::Result<()> {
    let child = if cfg!(target_os = "macos") {
        gwt_core::process::hidden_command("open").arg(url).spawn()?
    } else if cfg!(target_os = "windows") {
        // The empty "" before the URL is required by `start` so a URL
        // beginning with quoted text is not interpreted as a window
        // title.
        gwt_core::process::hidden_command("cmd")
            .args(["/C", "start", "", url])
            .spawn()?
    } else {
        gwt_core::process::hidden_command("xdg-open")
            .arg(url)
            .spawn()?
    };
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// CLI flags accepted by the tray-resident front door.
///
/// SPEC #2920 FR-013: `--no-tray` skips tray-icon creation (for CI /
/// Playwright). `--no-open` is preserved as a no-op for backward
/// compatibility — the tray menu `Open` action is what actually opens the
/// browser now, so the auto-open default is `false` regardless of this
/// flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayArgs {
    pub bind: IpAddr,
    pub port: Option<u16>,
    pub no_tray: bool,
    pub no_open: bool,
}

impl Default for TrayArgs {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: None,
            no_tray: false,
            no_open: false,
        }
    }
}

/// Errors surfaced by the tray-resident entry point.
#[derive(Debug, thiserror::Error)]
pub enum TrayError {
    #[error("tray-resident process is not yet implemented (SPEC #2920 Phase 4)")]
    NotYetImplemented,
}

/// SPEC #2920 Phase 4 partial — argv parse errors for the tray-resident
/// front door. `main()` renders the `Display` impl to stderr and exits 2,
/// matching the removed browser-server parser's contract.
#[derive(Debug, PartialEq, Eq)]
pub enum TrayArgParseError {
    MissingValue(String),
    InvalidIp(String),
    InvalidPort(String),
    UnknownFlag(String),
}

/// Canonical usage hint printed alongside any [`TrayArgParseError`]. Kept
/// as a constant so `main()`, the parser, and the README stay in sync.
pub const TRAY_USAGE_HINT: &str = "usage: gwt [--bind <ip>] [--port <n>] [--no-tray] [--no-open]";

impl std::fmt::Display for TrayArgParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingValue(flag) => {
                write!(f, "gwt: flag `{flag}` requires a value\n\n{TRAY_USAGE_HINT}")
            }
            Self::InvalidIp(value) => write!(
                f,
                "gwt: `--bind` got an invalid IP address: `{value}`\n\n{TRAY_USAGE_HINT}"
            ),
            Self::InvalidPort(value) => write!(
                f,
                "gwt: `--port` got an invalid port (expected 0..=65535): `{value}`\n\n{TRAY_USAGE_HINT}"
            ),
            Self::UnknownFlag(flag) => {
                write!(f, "gwt: unknown flag `{flag}`\n\n{TRAY_USAGE_HINT}")
            }
        }
    }
}

impl std::error::Error for TrayArgParseError {}

/// SPEC #2920 Phase 4 partial — parse the GUI route's argv into a
/// [`TrayArgs`]. The full Tray route (FrontDoorRoute::Tray) lands later;
/// for now this powers the existing GUI route so VPN-reachable hosts can
/// run `gwt --bind 0.0.0.0 --port <n>` without falling back to SSH local
/// port forwarding.
///
/// Accepted flags:
/// - `--bind <ip>`: parsed via `IpAddr::from_str`. Defaults to
///   `127.0.0.1` (matches the documented trust boundary).
/// - `--port <n>`: parsed via `u16::from_str`. Omission remains `None` so
///   stable-port policy can distinguish it from explicit `--port 0`.
/// - `--no-tray` / `--no-open`: recognised so the README hint does not
///   fail today, but their behaviour stays out of scope for this slice.
///
/// Unknown long flags are rejected. One legacy positional (e.g. `gwt .`)
/// is tolerated and ignored — the GUI route already uses `current_dir()`
/// for project discovery, so positional paths are inert today.
pub fn parse_tray_argv(argv: &[String]) -> Result<TrayArgs, TrayArgParseError> {
    use std::str::FromStr;

    let mut args = TrayArgs::default();
    let mut positional_consumed = false;
    let mut iter = argv.iter().skip(1);
    while let Some(token) = iter.next() {
        match token.as_str() {
            "--bind" => {
                let value = iter
                    .next()
                    .ok_or_else(|| TrayArgParseError::MissingValue("--bind".to_string()))?;
                args.bind = IpAddr::from_str(value)
                    .map_err(|_| TrayArgParseError::InvalidIp(value.clone()))?;
            }
            "--port" => {
                let value = iter
                    .next()
                    .ok_or_else(|| TrayArgParseError::MissingValue("--port".to_string()))?;
                args.port = Some(
                    u16::from_str(value)
                        .map_err(|_| TrayArgParseError::InvalidPort(value.clone()))?,
                );
            }
            "--no-tray" => args.no_tray = true,
            "--no-open" => args.no_open = true,
            flag if flag.starts_with("--") => {
                return Err(TrayArgParseError::UnknownFlag(flag.to_string()));
            }
            positional if !positional_consumed => {
                // Legacy `gwt .` / `gwt /some/path` shape — see
                // `front_door_route_keeps_gui_launch_for_empty_and_repo_path_argv`.
                let _ = positional;
                positional_consumed = true;
            }
            extra => {
                return Err(TrayArgParseError::UnknownFlag(extra.to_string()));
            }
        }
    }
    Ok(args)
}

/// Entry point invoked by `main.rs` after `FrontDoorRoute::Tray` is
/// resolved. Phase 4 will replace the placeholder with the real event
/// loop + EmbeddedServer bootstrap.
pub fn run(_args: TrayArgs) -> Result<i32, TrayError> {
    Err(TrayError::NotYetImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_args_default_binds_loopback_and_omits_explicit_port() {
        let args = TrayArgs::default();
        assert_eq!(args.bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(args.port, None);
        assert!(!args.no_tray);
        assert!(!args.no_open);
    }

    #[test]
    fn tray_run_is_not_yet_implemented_in_phase_1() {
        // SPEC #2920 Phase 1 only ships skeletons; real bootstrap lands
        // in Phase 4 alongside the WebView removal.
        let err = run(TrayArgs::default()).expect_err("placeholder must error");
        assert!(matches!(err, TrayError::NotYetImplemented));
    }

    // SPEC #2920 Phase 4 partial — `--bind`/`--port` restore on the GUI
    // (tray-resident) route. These cover the argv parser; full Tray route
    // takeover and `--no-tray`/`--no-open` behaviour stay TODO.

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parse_tray_argv_default_returns_loopback_defaults() {
        let args = parse_tray_argv(&argv(&["gwt"])).expect("empty argv parses");
        assert_eq!(args, TrayArgs::default());
    }

    #[test]
    fn parse_tray_argv_accepts_bind_and_port() {
        let args = parse_tray_argv(&argv(&["gwt", "--bind", "0.0.0.0", "--port", "60745"]))
            .expect("bind+port parses");
        assert_eq!(args.bind, "0.0.0.0".parse::<IpAddr>().unwrap());
        assert_eq!(args.port, Some(60745));
        assert!(!args.no_tray);
        assert!(!args.no_open);
    }

    #[test]
    fn parse_tray_argv_accepts_loopback_explicitly() {
        let args = parse_tray_argv(&argv(&["gwt", "--bind", "127.0.0.1", "--port", "8787"]))
            .expect("explicit loopback parses");
        assert_eq!(args.bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(args.port, Some(8787));
    }

    #[test]
    fn parse_tray_argv_accepts_ipv6_unspecified() {
        let args =
            parse_tray_argv(&argv(&["gwt", "--bind", "::"])).expect("ipv6 unspecified parses");
        assert_eq!(args.bind, "::".parse::<IpAddr>().unwrap());
        assert_eq!(args.port, None);
    }

    #[test]
    fn parse_tray_argv_rejects_missing_bind_value() {
        let err = parse_tray_argv(&argv(&["gwt", "--bind"]))
            .expect_err("missing --bind value must error");
        assert!(matches!(err, TrayArgParseError::MissingValue(ref f) if f == "--bind"));
    }

    #[test]
    fn parse_tray_argv_rejects_missing_port_value() {
        let err = parse_tray_argv(&argv(&["gwt", "--port"]))
            .expect_err("missing --port value must error");
        assert!(matches!(err, TrayArgParseError::MissingValue(ref f) if f == "--port"));
    }

    #[test]
    fn parse_tray_argv_rejects_invalid_ip() {
        let err = parse_tray_argv(&argv(&["gwt", "--bind", "not-an-ip"]))
            .expect_err("invalid IP must error");
        assert!(matches!(err, TrayArgParseError::InvalidIp(ref v) if v == "not-an-ip"));
    }

    #[test]
    fn parse_tray_argv_rejects_invalid_port() {
        let err = parse_tray_argv(&argv(&["gwt", "--port", "99999"]))
            .expect_err("port out of range must error");
        assert!(matches!(err, TrayArgParseError::InvalidPort(ref v) if v == "99999"));
    }

    #[test]
    fn parse_tray_argv_rejects_unknown_flag() {
        let err = parse_tray_argv(&argv(&["gwt", "--no-such-flag"]))
            .expect_err("unknown flag must error");
        assert!(matches!(err, TrayArgParseError::UnknownFlag(ref f) if f == "--no-such-flag"));
    }

    #[test]
    fn parse_tray_argv_accepts_no_tray_and_no_open_as_noop_flags() {
        // SPEC #2920 Phase 4 partial: the flags are recognised so the
        // README hint `gwt --no-tray --no-open` does not error today.
        // Their behaviour (skipping tray icon / suppressing auto-open) is
        // out of scope for this slice and lands in the full Tray route
        // takeover.
        let args = parse_tray_argv(&argv(&["gwt", "--no-tray", "--no-open"])).expect("flags parse");
        assert!(args.no_tray);
        assert!(args.no_open);
        assert_eq!(args.bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(args.port, None);
    }

    #[test]
    fn parse_tray_argv_accepts_legacy_positional_path() {
        // Existing GUI launches accept `gwt .` and `gwt /some/path` — the
        // path is ignored by the GUI route (which uses `current_dir()`).
        // The parser must tolerate one positional so we do not break the
        // existing argv shape pinned by `front_door_route_keeps_gui_launch_for_empty_and_repo_path_argv`.
        let args = parse_tray_argv(&argv(&["gwt", "/some/path", "--port", "8787"]))
            .expect("positional + flags parse");
        assert_eq!(args.port, Some(8787));
    }

    #[test]
    fn parse_tray_argv_display_message_mentions_usage() {
        let err = parse_tray_argv(&argv(&["gwt", "--bind", "not-an-ip"]))
            .expect_err("invalid IP must error");
        let rendered = format!("{err}");
        assert!(
            rendered.contains("not-an-ip"),
            "error must echo the bad value: {rendered}"
        );
        assert!(
            rendered.contains("usage:") || rendered.contains("--bind"),
            "error must surface a usage hint: {rendered}"
        );
    }
}
