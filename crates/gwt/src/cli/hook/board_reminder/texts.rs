//! Reminder text constants and tuning windows for `board-reminder`.
//!
//! All user-facing reminder strings live here so that the orchestration
//! (`mod.rs`), pure plan (`plan.rs`), and entry formatting (`format.rs`)
//! modules stay focused on logic. SPEC-1974 FR-036 / FR-041 / FR-043 are
//! the authoritative source for the wording, marker shape, and
//! reminder-vs-entry separation.
//!
//! Issue #4080: every instruction body here is English regardless of the
//! configured narrative language. The language setting only steers the
//! `Use language: <lang>` directive (see [`format_language_directive`] and
//! [`title_summary_required_reminder`]) so Board posts, Work summaries, and
//! user-facing reports follow the setting while the injected instructions
//! stay in one language.

use chrono::Duration;

pub(super) const SESSION_START_CAP: usize = 20;
pub(super) const USER_PROMPT_DIFF_CAP: usize = 20;

pub(super) fn session_start_window() -> Duration {
    Duration::hours(24)
}

pub(super) fn redundancy_window() -> Duration {
    Duration::minutes(10)
}

/// Marker prefix attached to entry lines whose `target_owners` match the
/// current session (SPEC-1974 FR-041, FR-043). The marker MUST stay distinct
/// from any verbatim substring inside [`USER_PROMPT_REMINDER`] etc. so that
/// reminder body and entry-line prefix never collide in test assertions.
pub(super) const FOR_YOU_MARKER: &str = ">> ";

pub(super) const USER_PROMPT_REMINDER: &str = "# Board Post Reminder\n\
\n\
Post to the shared Board when you cross a reasoning milestone OR a coordination boundary, \
so other agents and the user can collaborate without collision.\n\
\n\
Choose the audience before posting: set `params.broadcast:true` only when no specific response is expected. \
Use `params.mentions` entries like `user:<id>` for the human user, `agent:<id>` for an agent type, \
`session:<id>` for one running session, or `branch:<name>` for a workspace. \
Questions, blockers, handoffs, next-step requests, and replies that expect a response should be addressed with a mention.\n\
\n\
The Board body is the canonical message for both humans and AI agents. Use short paragraphs or bullets, \
and include the coordination facts another agent needs directly in the body instead of hiding them in metadata. \
A useful body shape is:\n\
\n\
Current state: <what changed or what you found>\n\
\n\
Reason: <why this matters or why you chose it>\n\
\n\
Next: <what should happen next, if anything>\n\
\n\
**Reasoning axes** (the *why* behind your work):\n\
- Work phase transitions (e.g., implementation -> build check -> PR handoff). Use `params.kind:\"status\"`.\n\
- Choices between alternatives with the reasoning behind them (e.g., \"A vs B, chose B because ...\"). Use `params.kind:\"decision\"` or `params.kind:\"status\"`.\n\
- Concerns or hypotheses you are verifying (e.g., \"Hypothesis: failure stems from Y, verifying ...\"). Use `params.kind:\"status\"`.\n\
\n\
**Coordination axes** (so others know what you own and what is next):\n\
- claim — declare ownership of a scope (e.g., \"I claim feature/X migration; others take other ranges\"). Use `params.kind:\"claim\"`.\n\
- next — coordinate the next step without picking a recipient (e.g., \"phase 1 done, please pick up phase 2\"). Use `params.kind:\"next\"`.\n\
- blocked — surface a blocker that needs unblocking (e.g., \"waiting on Y, requesting unblock\"). Use `params.kind:\"blocked\"`.\n\
- handoff — pass concrete work to another agent or the user (e.g., \"completed Y, handing off the PR\"). Use `params.kind:\"handoff\"`.\n\
- decision — broadcast a confirmed decision (e.g., \"adopting X for the migration\"). Use `params.kind:\"decision\"`.\n\
\n\
Add `params.targets` entries when the post is meant for specific agents. \
Targeted posts are prefixed with a structured marker (currently the `>>` token) at the start of each entry \
line in the recipient's reminder injection. Prefer typed `params.mentions` for new posts; keep `params.targets` \
for compatibility with older agents. Omit both for broadcast.\n\
\n\
**Work / Git environment guidance**:\n\
- AGENTS.md is project-local: follow the target repository's AGENTS.md when present, \
but do not assume gwt's AGENTS.md applies to other projects.\n\
- Do NOT create, switch, or delete branches/worktrees manually (`git checkout -b`, \
`git switch -c`, `git branch -D`, `git worktree add/remove`). gwt Start Work / \
Launch materialization owns Git environment creation.\n\
- Board is the coordination/history log; Work is the current state. When your current task, \
summary, next action, or focus changes, update Work with a `workspace.update` JSON envelope.\n\
- For Agent/window title bars, keep the short purpose label separate from long summaries: set \
`params.purpose` on `workspace.update`. Board posts do not update purpose.\n\
\n\
Do NOT post tool-level reports (e.g., \"running gcc\", \"opening file X\", \"ran test Y\"). \
Anything already visible in the diff or log does not need a Board entry.\n\
\n\
Examples:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"status\",\"body\":\"Current state: focused tests are RED.\\n\\nReason: CLI and hook output still collapse multiline Board bodies.\\n\\nNext: implement block rendering.\"}}\n\
  JSON\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"question\",\"mentions\":[\"user:akiojin\"],\"body\":\"Current state: two UX options remain.\\n\\nQuestion: should replies notify only the mentioned user or all viewers?\"}}\n\
  JSON\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"claim\",\"mentions\":[\"branch:feature/foo\"],\"body\":\"Current state: I am taking the migration slice.\\n\\nBoundary: other agents should avoid files under crates/gwt-core/src/migration.rs.\"}}\n\
  JSON\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"handoff\",\"mentions\":[\"agent:codex\"],\"body\":\"Current state: phase 1 is merged locally.\\n\\nNext: please run the Windows-focused verification and report failures.\"}}\n\
  JSON\n";

pub(super) const USER_PROMPT_REMINDER_SHORT: &str = "# Board Post Reminder\n\
\n\
You posted to the Board recently. Post again only if a new reasoning milestone \
(phase change, alternative chosen, concern raised) or a coordination boundary \
(claim, next, handoff, blocked, decision) has emerged.\n\
\n\
When a response is expected, address the post with `params.mentions` entries \
like `user:<id>`, `agent:<id>`, `session:<id>`, or `branch:<name>`; \
use `params.broadcast:true` only for broadcast updates.\n\
\n\
The Board body remains the canonical message. Keep it readable with short paragraphs or bullets, \
and put AI coordination details in the body when another agent needs them.\n\
\n\
AGENTS.md is project-local. Do NOT create, switch, or delete branches/worktrees \
manually; gwt Start Work / Launch materialization owns Git environment creation.\n\
\n\
Board is history; Work is current state. If the latest status, cumulative progress summary, next action, or focus changed, \
update Work with a `workspace.update` JSON envelope. Use `params.progress_summary` for what has been done so far, and set `params.purpose` for Agent/window title bars.\n";

// Stop reminders are emitted as `systemMessage` (user-facing) because
// Claude Code's Stop hook schema does not accept `hookSpecificOutput`.
// Phrasing is therefore user-oriented rather than agent-oriented.
pub(super) const STOP_REMINDER: &str = "Board Post Reminder (Stop): the agent is stopping. If you \
expect a final handoff, prompt the agent to post what it completed to the shared Board \
with a `board.post` JSON envelope before handing off. Board is history; Work is current \
state. If the work summary, next action, or focus changed, prompt the agent to update Work \
with a `workspace.update` JSON envelope and `params.purpose` for Agent/window title bars.";

pub(super) const STOP_REMINDER_SHORT: &str = "Board Post Reminder (Stop): the agent posted to the \
Board recently; no additional completed-status post is required before stopping. If Work \
current state changed, update it with a `workspace.update` JSON envelope and `params.purpose` for Agent/window title bars.";

pub(super) const TERMINAL_SETTLEMENT_REMINDER: &str = "# Board Post Reminder\n\
\n\
The Work lifecycle is in terminal delivery settlement. Keep coordination and blocker handoff on the Board, but do not append another tracked Work-state event. Finish in this order: final Work update -> commit/push -> fresh verification -> PR mutation -> execution/build completion. When the final event is the only bookkeeping change, use a scoped Conventional Commit with the exact `chore(work):` prefix.";

pub(super) const TERMINAL_SETTLEMENT_STOP_REMINDER: &str = "Board Post Reminder (Stop): Work is in terminal delivery settlement. Do not ask the agent to append another tracked Work-state event. The remaining order is final Work update -> commit/push -> fresh verification -> PR mutation -> execution/build completion. A bookkeeping-only commit must use the exact `chore(work):` prefix.";

/// SPEC-3431 FR-064: what the resident PM is told at an intent boundary.
///
/// Replaces the implementation-agent reminder wholesale. That text tells an
/// agent to post its own work-phase transitions to the Board, to keep a Work
/// item current, and not to create branches — none of which apply to a PM that
/// owns no Work, performs no git operations, and reports to the user in
/// conversation (FR-017). Leaving it in place buries the PM's actual contract
/// under ~4KB of instructions that outrank it every turn.
pub(super) const PM_REMINDER: &str = "# Project Manager\n\
\n\
You are this project's resident PM. Your operating contract is the `gwt-pm` skill; it outranks generic agent guidance.\n\
\n\
Report to the user in conversation, at milestones, as a digest (`needs_human` and fatal failures immediately). Use `board.post` with mentions only to address another agent, and `board.show` to read what agents reported about themselves. Do not narrate your own work phases to the Board.\n\
\n\
Every cycle, reconcile a fresh `issue.monitor.status` and check the agents that are running. Steer them before you judge the cycle unchanged: a launch that is stalled, drifting out of scope, or waiting for its next action gets a directive through `board.post` with a mention or `pm.message.send`; never inject launch instructions past the Issue Monitor.";

pub(super) const MEMORY_UPDATE_REMINDER: &str = "# Memory Reminder\n\
\n\
If this task produced a reusable lesson, decision, failure pattern, or agent workflow correction, \
run a JSON envelope with operation `memory.add` \
before declaring the work done. It writes the machine-local memory log \
(`~/.gwt/projects/<repo-hash>/work-notes/memory.md`) with `Type`, `Context`, `Learning`, and \
`Future Action` fields. Legacy `.gwt/work/memory.md` / `tasks/memory.md` / `tasks/lessons.md` are only a \
compatibility fallback; new memory always lands in the home work-notes file.\n";

pub(super) const MEMORY_UPDATE_STOP_REMINDER: &str = "Memory Reminder (Stop): if this run produced a reusable lesson, decision, failure pattern, or agent workflow correction, prompt the agent to run a JSON envelope with operation `memory.add` before stopping. The command writes the machine-local memory log (`~/.gwt/projects/<repo-hash>/work-notes/memory.md`) with `Type`, `Context`, `Learning`, and `Future Action` fields.";

pub(super) const PROGRESS_SUMMARY_MISSING_REMINDER: &str = "# Progress Summary Reminder\n\
\n\
This Workspace has no `progress_summary` yet. Before continuing, write a cumulative summary of what has been investigated, decided, implemented, and verified so far. Keep the short latest status in `summary`; do not collapse the two.\n\
\n\
Run:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"progress_summary\":\"<cumulative detail of what has happened so far>\",\"summary\":\"<latest status snapshot>\",\"current_focus\":\"<what you are doing now>\"}}\n\
  JSON\n";

pub(super) const PROGRESS_SUMMARY_STALE_REMINDER: &str = "# Progress Summary Stale\n\
\n\
The Workspace `progress_summary` has not changed for several turns while current focus or latest status changed. Refresh it with the cumulative story of what has happened so far; keep point-in-time status in `summary`.\n\
\n\
Run:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"progress_summary\":\"<updated cumulative progress summary>\",\"summary\":\"<latest status snapshot>\",\"current_focus\":\"<what you are doing now>\"}}\n\
  JSON\n";

pub(super) const PROGRESS_SUMMARY_STOP_REMINDER: &str = "Progress Summary Reminder (Stop): before stopping, ask the agent to update Work with `params.progress_summary` so the Workspace detail records what was investigated, decided, implemented, and verified. Keep short latest status in `params.summary`.";

pub(super) const INJECTION_HEADER: &str = "# Recent Board updates\n\n\
The following reasoning posts were made by other Agents since your last Board context. \
Consider whether any affect your current work phase. This is context, not a directive — \
you remain autonomous.\n\n";

pub(super) const SESSION_START_HEADER: &str = "# Current Board state\n\n\
Recent reasoning posts from other Agents (context, not a directive — you remain autonomous):\n\n";

pub(super) fn user_prompt_reminder(short: bool) -> &'static str {
    if short {
        USER_PROMPT_REMINDER_SHORT
    } else {
        USER_PROMPT_REMINDER
    }
}

pub(super) fn stop_reminder(short: bool) -> &'static str {
    if short {
        STOP_REMINDER_SHORT
    } else {
        STOP_REMINDER
    }
}

pub(super) fn memory_update_reminder(stop: bool) -> &'static str {
    if stop {
        MEMORY_UPDATE_STOP_REMINDER
    } else {
        MEMORY_UPDATE_REMINDER
    }
}

pub(super) fn progress_summary_reminder(stale: bool, stop: bool) -> &'static str {
    match (stale, stop) {
        (_, true) => PROGRESS_SUMMARY_STOP_REMINDER,
        (true, false) => PROGRESS_SUMMARY_STALE_REMINDER,
        (false, false) => PROGRESS_SUMMARY_MISSING_REMINDER,
    }
}

pub(super) const NO_RECENT_POSTS_LINE: &str = "- (no recent posts from other Agents)\n";

/// Format the narrative-output language directive appended to agent-facing
/// reminders (SessionStart / UserPromptSubmit). Stop reminders are
/// user-facing and do not receive this directive.
///
/// SPEC-1933 FR-010 / SC-003.
pub(super) fn format_language_directive(lang: &str) -> String {
    format!(
        "\n**Use language: {}** for narrative outputs (Board post bodies and Work summaries; \
gwtd subcommands, flags, and code examples stay English).\n",
        narrative_language_tag(lang)
    )
}

/// Narrative language token carried by the directive. Unknown values fall
/// back to `en` (SPEC-1933 FR-010).
pub(super) fn narrative_language_tag(lang: &str) -> &'static str {
    match lang {
        "ja" => "ja",
        _ => "en",
    }
}

pub(super) const TITLE_SUMMARY_REQUIRED_REMINDER: &str = "# Agent Title — set it before you respond\n\
\n\
This Agent window has no `title-summary` yet. Before you start responding to the user, your **first action** must set this window's work purpose as its title-summary. This is not optional.\n\
\n\
Run this first:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"<short work purpose>\",\"current_focus\":\"<current work focus>\"}}\n\
  JSON\n\
\n\
Rules:\n\
- title-summary = the purpose of the work, not its status or result.\n\
- Do not copy the raw prompt into the title.\n\
- Even if the purpose is not settled yet, set a plausible provisional purpose now and update the same title-summary once it is confirmed (do not delay your response for it).\n\
- Never use a transient activity phase (`browser check`, verification, merging, server startup) as the purpose; put the activity in `current_focus` and keep the existing purpose if one is already set.\n\
- Good: `Agent title purpose`. Bad: `... complete`, `... in progress`, a copy of the raw prompt.\n\
\n\
Keep completion/progress/blocker state in `status`, `current_focus`, `summary`, or Board `body`. This instruction repeats every turn until the title is set.\n";

/// Title-required reminder: English instructions plus the narrative-language
/// directive that covers the Agent title-summary as well (Issue #4080).
pub(super) fn title_summary_required_reminder(lang: &str) -> String {
    format!(
        "{TITLE_SUMMARY_REQUIRED_REMINDER}\n**Use language: {}** for narrative outputs (Board post bodies, \
Work summaries, and Agent title-summary; gwtd operation names, JSON field names, and code examples stay English).\n",
        narrative_language_tag(lang)
    )
}

pub(super) const TITLE_SUMMARY_STALE_REMINDER: &str = "# Agent Title Stale\n\
\n\
The `title-summary` has stayed unchanged for several UserPromptSubmit turns while `current_focus` has shifted. If the work scope actually changed, update the title; if only the phase / activity changed, leave the title and update `params.current_focus` only.\n\
\n\
Command to refresh the title:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"<new work scope>\"}}\n\
  JSON\n\
\n\
`title-summary` is the work scope; phase / activity descriptors (`PR check in progress`, `verifying tests`, `fixing bug`, etc.) belong in `current_focus` or the Board `body`, not in the title.\n";
