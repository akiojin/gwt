//! Embedded skill, command, and hook assets bundled at build time.
//!
//! gwt treats these files as opaque blobs: they are written to worktrees
//! as-is, and interpretation is the responsibility of Claude Code / Codex.

use include_dir::{include_dir, Dir};

/// All skill directories under `.claude/skills/`.
pub static CLAUDE_SKILLS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../.claude/skills");

/// All command files under `.claude/commands/`.
pub static CLAUDE_COMMANDS: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/../../.claude/commands");

/// All hook scripts under `.claude/hooks/scripts/`.
pub static CLAUDE_HOOKS: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/../../.claude/hooks/scripts");

/// All hook scripts under `.codex/hooks/scripts/`.
pub static CODEX_HOOKS: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/../../.codex/hooks/scripts");
