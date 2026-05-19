//! SPEC-1942 US-14 / FR-093〜FR-101: argv parser for `gwt serve` and the
//! equivalent `gwt --headless` alias.
//!
//! Unlike the other `cli/<family>.rs` modules, `serve` is **not** dispatched
//! through `run_cli` / `CliCommand`. Instead, `FrontDoorRoute::Headless` is
//! detected by `front_door_route` (both defined in the binary-only
//! `runtime_support` module) and `main()` calls [`parse`] on the same argv
//! slice to obtain [`ServeArgs`] before handing off to the headless boot
//! path. `should_dispatch_cli` is intentionally left unaware of `serve` so
//! the GUI/CLI dual-mode contract from SPEC-1942 US-1 stays untouched
//! (FR-101).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};

/// Arguments accepted by `gwt serve [...]` / `gwt --headless [...]`.
///
/// - `bind`: the IP address the embedded server binds to. Defaults to the
///   loopback address `127.0.0.1`. The user must pass `--bind <addr>` opt-in
///   to listen on any non-loopback address.
/// - `port`: TCP port. `0` means random ephemeral port (matches the existing
///   GUI behaviour at `crates/gwt/src/embedded_server.rs`). User-provided
///   values are kept verbatim.
/// - `open`: whether to spawn the system default browser at the served URL
///   after the server is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeArgs {
    pub bind: IpAddr,
    pub port: u16,
    pub open: bool,
}

impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            open: false,
        }
    }
}

/// Parser errors. They are intentionally narrow and string-free so callers can
/// translate them into user-facing diagnostics without leaking internal
/// formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeParseError {
    /// Caller passed an argv slice that does not start with the `serve` verb
    /// or the `--headless` alias.
    MissingVerb,
    /// `--bind` flag is present but the value is missing or empty.
    MissingBind,
    /// `--bind` value is not a valid IPv4 / IPv6 literal.
    InvalidBind(String),
    /// `--port` flag is present but the value is missing or empty.
    MissingPort,
    /// `--port` value is not a valid `u16`.
    InvalidPort(String),
    /// Unknown flag was passed. Carries the raw flag text.
    UnknownFlag(String),
}

impl fmt::Display for ServeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeParseError::MissingVerb => {
                write!(f, "expected `serve` or `--headless` as the first argument")
            }
            ServeParseError::MissingBind => write!(f, "--bind requires an IP address"),
            ServeParseError::InvalidBind(value) => {
                write!(f, "invalid --bind address: {value}")
            }
            ServeParseError::MissingPort => write!(f, "--port requires a TCP port number"),
            ServeParseError::InvalidPort(value) => {
                write!(f, "invalid --port number: {value}")
            }
            ServeParseError::UnknownFlag(flag) => {
                write!(f, "unknown flag for `gwt serve`: {flag}")
            }
        }
    }
}

impl std::error::Error for ServeParseError {}

/// Parse argv starting at the `serve` / `--headless` verb.
///
/// The caller is expected to pass `&argv[1..]` (i.e. the slice without the
/// program name). The first element must be either `serve` or `--headless`;
/// any other shape returns [`ServeParseError::MissingVerb`].
pub fn parse(args: &[String]) -> Result<ServeArgs, ServeParseError> {
    let mut it = args.iter();
    match it.next().map(String::as_str) {
        Some("serve") | Some("--headless") => {}
        _ => return Err(ServeParseError::MissingVerb),
    }

    let mut out = ServeArgs::default();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--bind" => {
                let value = it.next().ok_or(ServeParseError::MissingBind)?;
                if value.is_empty() {
                    return Err(ServeParseError::MissingBind);
                }
                out.bind = value
                    .parse::<IpAddr>()
                    .map_err(|_| ServeParseError::InvalidBind(value.clone()))?;
            }
            "--port" => {
                let value = it.next().ok_or(ServeParseError::MissingPort)?;
                if value.is_empty() {
                    return Err(ServeParseError::MissingPort);
                }
                out.port = value
                    .parse::<u16>()
                    .map_err(|_| ServeParseError::InvalidPort(value.clone()))?;
            }
            "--open" => {
                out.open = true;
            }
            other => return Err(ServeParseError::UnknownFlag(other.to_string())),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_serve_defaults_bind_to_loopback_and_port_to_zero_and_open_false() {
        let parsed = parse(&argv(&["serve"])).expect("serve parses with defaults");
        assert_eq!(parsed.bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(parsed.port, 0);
        assert!(!parsed.open);
    }

    #[test]
    fn parse_serve_accepts_bind_and_port_and_open() {
        let parsed = parse(&argv(&[
            "serve", "--bind", "0.0.0.0", "--port", "8787", "--open",
        ]))
        .expect("serve accepts flag bundle");
        assert_eq!(parsed.bind, IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        assert_eq!(parsed.port, 8787);
        assert!(parsed.open);
    }

    #[test]
    fn parse_dash_dash_headless_is_equivalent_to_serve() {
        let serve = parse(&argv(&["serve", "--port", "8787"])).expect("serve");
        let alias = parse(&argv(&["--headless", "--port", "8787"])).expect("--headless");
        assert_eq!(serve, alias);
    }

    #[test]
    fn parse_serve_rejects_invalid_bind() {
        let err =
            parse(&argv(&["serve", "--bind", "not-an-ip"])).expect_err("invalid bind must error");
        assert_eq!(err, ServeParseError::InvalidBind("not-an-ip".to_string()));
    }

    #[test]
    fn parse_serve_rejects_invalid_port() {
        let err = parse(&argv(&["serve", "--port", "1234567"])).expect_err("port must fit in u16");
        assert_eq!(err, ServeParseError::InvalidPort("1234567".to_string()));
    }

    #[test]
    fn parse_serve_rejects_missing_verb() {
        assert_eq!(
            parse(&argv(&["--port", "8787"])).expect_err("missing verb"),
            ServeParseError::MissingVerb
        );
        assert_eq!(
            parse(&argv(&[])).expect_err("empty argv"),
            ServeParseError::MissingVerb
        );
    }

    #[test]
    fn parse_serve_rejects_unknown_flag() {
        let err = parse(&argv(&["serve", "--frobnicate"])).expect_err("unknown flag must error");
        assert_eq!(
            err,
            ServeParseError::UnknownFlag("--frobnicate".to_string())
        );
    }

    #[test]
    fn parse_serve_accepts_ipv6_bind() {
        let parsed = parse(&argv(&["serve", "--bind", "::1"])).expect("ipv6 loopback");
        assert!(matches!(parsed.bind, IpAddr::V6(_)));
    }
}
