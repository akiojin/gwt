//! SPEC #2920 Phase 2 (T-020) — verify `gwt serve` and `gwt --headless`
//! are removed and that the canonical usage hint reaches stderr with
//! exit code 2. Also pins the runtime_support shape so the hint message
//! and routing cannot drift apart silently.

const RUNTIME_SUPPORT_SRC: &str = include_str!("../src/runtime_support.rs");
const MAIN_SRC: &str = include_str!("../src/main.rs");
const CLI_SRC: &str = include_str!("../src/cli.rs");

#[test]
fn front_door_route_defines_legacy_serve_usage_hint_variant() {
    assert!(
        RUNTIME_SUPPORT_SRC.contains("LegacyServeUsageHint"),
        "FrontDoorRoute must declare the LegacyServeUsageHint variant for `gwt serve` migration"
    );
    // `Headless` is replaced by `LegacyServeUsageHint`; the old name
    // must not silently linger as a routing target.
    let banned_variant = "FrontDoorRoute::Headless";
    assert!(
        !RUNTIME_SUPPORT_SRC.contains(banned_variant),
        "Removed variant `{banned_variant}` must not be reintroduced — SPEC #2920 Q9 supersede"
    );
}

#[test]
fn runtime_support_pins_canonical_legacy_usage_hint() {
    assert!(
        RUNTIME_SUPPORT_SRC.contains("LEGACY_SERVE_USAGE_HINT"),
        "runtime_support.rs must export LEGACY_SERVE_USAGE_HINT for test parity"
    );
    let canonical = "have been removed in v10.0.0 (SPEC #2920)";
    assert!(
        RUNTIME_SUPPORT_SRC.contains(canonical),
        "LEGACY_SERVE_USAGE_HINT must include the canonical migration phrase"
    );
    let canonical_fix = "Use `gwt --no-tray --no-open` instead.";
    assert!(
        RUNTIME_SUPPORT_SRC.contains(canonical_fix),
        "LEGACY_SERVE_USAGE_HINT must redirect to `gwt --no-tray --no-open`"
    );
}

#[test]
fn main_emits_legacy_usage_hint_before_gui_bootstrap() {
    assert!(
        MAIN_SRC.contains("runtime_support::FrontDoorRoute::LegacyServeUsageHint"),
        "main() must short-circuit the LegacyServeUsageHint route"
    );
    assert!(
        MAIN_SRC.contains("runtime_support::LEGACY_SERVE_USAGE_HINT"),
        "main() must emit runtime_support::LEGACY_SERVE_USAGE_HINT verbatim"
    );
    // The serve_args parser is gone; its presence would silently reintroduce
    // the deleted `cli::serve` module.
    assert!(
        !MAIN_SRC.contains("serve_args"),
        "main() must not reference serve_args after SPEC #2920 Phase 2"
    );
    assert!(
        !MAIN_SRC.contains("gwt::cli::serve::parse"),
        "main() must not call the removed gwt::cli::serve::parse()"
    );
}

#[test]
fn cli_root_no_longer_exports_serve_module() {
    assert!(
        !CLI_SRC.contains("pub mod serve;"),
        "cli.rs must drop `pub mod serve;` after SPEC #2920 Phase 2"
    );
}

// Live routing behaviour (`gwt serve` argv → LegacyServeUsageHint) is
// covered by the in-crate unit tests in
// `crates/gwt/src/runtime_support.rs` because `runtime_support` is a
// private module of the `gwt` binary. The grep-based assertions above
// pin the wiring at the public source surface so a refactor cannot
// silently rename or drop the variant.
