//! Ad-hoc helper to regenerate `.claude/settings.local.json` and
//! `.codex/hooks.json` against the current worktree using the updated
//! generators. Used to migrate the repo from the inline shell / literal
//! `gwt` form to the absolute-path self-dispatch form (SPEC #1942).
//!
//! Usage:
//!
//!   cargo build -p gwt && cargo run -p gwt-skills --example regenerate_hook_settings
//!
//! An optional argument selects which `.codex/hooks.json` copies to write:
//! `worktree-local`, `workspace-home`, or `both` (default, matching the
//! non-launch materialization contract).
//!
//! Run from the worktree root. The generator is idempotent — it
//! replaces gwt-managed entries and preserves user-defined hooks.
//!
//! The example sets `GWT_HOOK_BIN` to the gwt binary found in
//! `target/{debug,release}` so that the generated hook commands embed
//! the correct binary, not this example binary. Set `GWT_HOOK_BIN`
//! yourself to override it — `GWT_HOOK_BIN=gwtd` produces the portable
//! form a version-controlled `.codex/hooks.json` needs (#3474), since a
//! machine-local absolute path must never be committed.

use std::path::Path;

fn main() -> std::io::Result<()> {
    // Locate the real gwt binary. Without this, `current_exe()` inside
    // the generator would return this example's own path, which is not the
    // binary Claude Code should dispatch to.
    if std::env::var_os("GWT_HOOK_BIN").is_none_or(|value| value.is_empty()) {
        let candidates = [
            "target/debug/gwt",
            "target/release/gwt",
            "target/debug/gwt.exe",
            "target/release/gwt.exe",
        ];
        let gwt_bin = candidates
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())
            .and_then(|p| p.canonicalize().ok())
            .expect(
                "gwt binary not found in target/debug or target/release; \
                 run `cargo build -p gwt` first",
            );
        std::env::set_var("GWT_HOOK_BIN", &gwt_bin);
    }
    eprintln!(
        "using gwt at: {}",
        std::env::var("GWT_HOOK_BIN").unwrap_or_default()
    );

    // Defaults to both Codex discovery locations, matching the non-launch
    // materialization contract (`gwt::managed_assets::MANAGED_CODEX_HOOK_DISCOVERY_MODE`).
    let mode = match std::env::args().nth(1) {
        Some(value) => gwt_skills::CodexHookDiscoveryMode::from_cli_value(&value)
            .expect("codex hook discovery mode: worktree-local | workspace-home | both"),
        None => gwt_skills::CodexHookDiscoveryMode::Both,
    };

    let worktree = Path::new(".");
    println!("regenerating .claude/settings.local.json …");
    gwt_skills::generate_settings_local(worktree)?;
    println!("regenerating .codex/hooks.json ({}) …", mode.as_cli_value());
    gwt_skills::generate_codex_hooks_for_mode(worktree, mode)?;
    println!("done");
    Ok(())
}
