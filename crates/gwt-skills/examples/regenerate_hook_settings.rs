//! Ad-hoc helper to regenerate `.claude/settings.local.json` and
//! `.codex/hooks.json` against the current worktree using the updated
//! generators. Used once to migrate the repo from the inline shell
//! form to the `gwt hook ...` CLI form (SPEC #1942 US-4).
//!
//! Usage:
//!
//!   cargo run -p gwt-skills --example regenerate_hook_settings
//!
//! Run from the worktree root. The generator is idempotent — it
//! replaces gwt-managed entries and preserves user-defined hooks.

use std::path::Path;

fn main() -> std::io::Result<()> {
    let worktree = Path::new(".");
    println!("regenerating .claude/settings.local.json …");
    gwt_skills::generate_settings_local(worktree)?;
    println!("regenerating .codex/hooks.json …");
    gwt_skills::generate_codex_hooks(worktree)?;
    println!("done");
    Ok(())
}
