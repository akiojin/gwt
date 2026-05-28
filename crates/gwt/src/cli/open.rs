//! SPEC #2920 FR-006 — `gwt open` CLI verb skeleton.
//!
//! Reads the tray-resident process lock file, extracts the embedded
//! server URL, and launches the OS default browser via the existing
//! `open_url_with_os_default` helper (currently in
//! `crates/gwt/src/app_runtime/mod.rs:2943`, planned for relocation to
//! `gwt-core` in Phase 4 / T-043).
//!
//! Phase 1 only declares the parser surface so the route table in
//! `cli.rs` can list `open` ahead of the runtime work in Phase 6.

use gwt_github::SpecOpsError;

use super::{CliEnv, CliParseError};

/// `gwt open` accepts no positional arguments today. Future Phase 6
/// follow-ups may add `--url <url>` for explicit overrides; the SPEC
/// keeps that as an `Out of Scope (v1)` item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenArgs;

/// Parse `gwtd open [...]` after the verb has already been stripped.
pub fn parse_args(args: &[String]) -> Result<OpenArgs, CliParseError> {
    if let Some(extra) = args.iter().find(|arg| !arg.is_empty()) {
        return Err(CliParseError::UnknownSubcommand(extra.clone()));
    }
    Ok(OpenArgs)
}

/// Run the `gwt open` verb. Phase 6 will implement the real logic; for
/// now we surface a clear error so callers cannot mistake an unimplemented
/// path for a no-op success.
pub fn run<E: CliEnv>(
    _env: &mut E,
    _args: OpenArgs,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    out.push_str("gwt open: not yet implemented (SPEC #2920 Phase 6)\n");
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_zero_arguments() {
        let parsed = parse_args(&[]).expect("empty argv parses");
        assert_eq!(parsed, OpenArgs);
    }

    #[test]
    fn parse_args_rejects_unknown_extras() {
        let argv = vec!["--no-such-flag".to_string()];
        let err = parse_args(&argv).expect_err("unknown flag must error");
        assert!(matches!(err, CliParseError::UnknownSubcommand(flag) if flag == "--no-such-flag"));
    }
}
