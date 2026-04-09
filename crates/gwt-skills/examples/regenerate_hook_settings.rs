//! Ad-hoc helper to regenerate `.claude/settings.local.json` and
//! `.codex/hooks.json` against the current worktree using the updated
//! generators. Used to migrate the repo from the inline shell / literal
//! `gwt` form to the absolute-path self-dispatch form (SPEC #1942).
//!
//! Usage:
//!
//!   cargo build -p gwt-tui && cargo run -p gwt-skills --example regenerate_hook_settings
//!
//! Run from the worktree root. The generator is idempotent — it
//! replaces gwt-managed entries and preserves user-defined hooks.
//!
//! The example sets `GWT_HOOK_BIN` to the gwt-tui binary found in
//! `target/{debug,release}` so that the generated hook commands embed
//! the correct binary, not this example binary.

use std::path::Path;

fn main() -> std::io::Result<()> {
    // Locate the real gwt-tui binary. Without this, `current_exe()` inside
    // the generator would return this example's own path, which is not the
    // binary Claude Code should dispatch to.
    let candidates = [
        "target/debug/gwt-tui",
        "target/release/gwt-tui",
        "target/debug/gwt-tui.exe",
        "target/release/gwt-tui.exe",
    ];
    let gwt_tui = candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
        .and_then(|p| p.canonicalize().ok())
        .expect(
            "gwt-tui binary not found in target/debug or target/release; \
             run `cargo build -p gwt-tui` first",
        );
    eprintln!("using gwt-tui at: {}", gwt_tui.display());
    std::env::set_var("GWT_HOOK_BIN", &gwt_tui);

    let worktree = Path::new(".");
    println!("regenerating .claude/settings.local.json …");
    gwt_skills::generate_settings_local(worktree)?;
    println!("regenerating .codex/hooks.json …");
    gwt_skills::generate_codex_hooks(worktree)?;
    println!("done");
    Ok(())
}
