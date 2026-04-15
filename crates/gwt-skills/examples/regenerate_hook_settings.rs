//! Ad-hoc helper to regenerate `.claude/settings.local.json` and
//! `.codex/hooks.json` against the current worktree using the updated
//! generators. Used to migrate the repo from the inline shell / literal
//! `gwt` form to the absolute-path self-dispatch form (SPEC #1942).
//!
//! Usage:
//!
//!   cargo build -p gwt && cargo run -p gwt-skills --example regenerate_hook_settings
//!
//! Run from the worktree root. The generator is idempotent — it
//! replaces gwt-managed entries and preserves user-defined hooks.
//!
//! The example sets `GWT_HOOK_BIN` to the gwt binary found in
//! `target/{debug,release}` so that the generated hook commands embed
//! the correct binary, not this example binary.

use std::path::Path;

fn main() -> std::io::Result<()> {
    // Locate the real gwt binary. Without this, `current_exe()` inside
    // the generator would return this example's own path, which is not the
    // binary Claude Code should dispatch to.
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
    eprintln!("using gwt at: {}", gwt_bin.display());
    std::env::set_var("GWT_HOOK_BIN", &gwt_bin);

    let worktree = Path::new(".");
    println!("regenerating .claude/settings.local.json …");
    gwt_skills::generate_settings_local(worktree)?;
    println!("regenerating .codex/hooks.json …");
    gwt_skills::generate_codex_hooks(worktree)?;
    println!("done");
    Ok(())
}
