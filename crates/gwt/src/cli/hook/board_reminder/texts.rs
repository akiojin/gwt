//! Reminder text constants and tuning windows for `board-reminder`.
//!
//! All user-facing reminder strings live here so that the orchestration
//! (`mod.rs`), pure plan (`plan.rs`), and entry formatting (`format.rs`)
//! modules stay focused on logic. SPEC-1974 FR-036 / FR-041 / FR-043 are
//! the authoritative source for the wording, marker shape, and
//! reminder-vs-entry separation.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReminderLanguage {
    En,
    Ja,
}

pub(super) fn reminder_language(lang: &str) -> ReminderLanguage {
    match lang {
        "ja" => ReminderLanguage::Ja,
        _ => ReminderLanguage::En,
    }
}

pub(super) const USER_PROMPT_REMINDER: &str = "# Board Post Reminder\n\
\n\
Post to the shared Board when you cross a reasoning milestone OR a coordination boundary, \
so other agents and the user can collaborate without collision.\n\
\n\
Choose the audience before posting: use broadcast only when no specific response is expected. \
Use `--mention user:<id>` for the human user, `--mention agent:<id>` for an agent type, \
`--mention session:<id>` for one running session, or `--mention branch:<name>` for a workspace. \
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
- Work phase transitions (e.g., implementation -> build check -> PR handoff). Use `--kind status`.\n\
- Choices between alternatives with the reasoning behind them (e.g., \"A vs B, chose B because ...\"). Use `--kind decision` or `--kind status`.\n\
- Concerns or hypotheses you are verifying (e.g., \"Hypothesis: failure stems from Y, verifying ...\"). Use `--kind status`.\n\
\n\
**Coordination axes** (so others know what you own and what is next):\n\
- claim — declare ownership of a scope (e.g., \"I claim feature/X migration; others take other ranges\"). Use `--kind claim`.\n\
- next — coordinate the next step without picking a recipient (e.g., \"phase 1 done, please pick up phase 2\"). Use `--kind next`.\n\
- blocked — surface a blocker that needs unblocking (e.g., \"waiting on Y, requesting unblock\"). Use `--kind blocked`.\n\
- handoff — pass concrete work to another agent or the user (e.g., \"completed Y, handing off the PR\"). Use `--kind handoff`.\n\
- decision — broadcast a confirmed decision (e.g., \"adopting X for the migration\"). Use `--kind decision`.\n\
\n\
Add `--target <session-id|branch|agent-id>` (repeatable) when the post is meant for specific agents. \
Targeted posts are prefixed with a structured marker (currently the `>>` token) at the start of each entry \
line in the recipient's reminder injection. Prefer typed `--mention ...` for new posts; keep `--target` \
for compatibility with older agents. Omit both for broadcast.\n\
\n\
**Workspace / Git environment guidance**:\n\
- AGENTS.md is project-local: follow the target repository's AGENTS.md when present, \
but do not assume gwt's AGENTS.md applies to other projects.\n\
- Do NOT create, switch, or delete branches/worktrees manually (`git checkout -b`, \
`git switch -c`, `git branch -D`, `git worktree add/remove`). gwt Start Work / \
Launch materialization owns Git environment creation.\n\
- Board is the coordination/history log; Workspace is the current state. When your current task, \
summary, next action, or focus changes, update Workspace with `gwtd workspace update`.\n\
- For Agent/window title bars, keep the short label separate from long summaries: use \
`--title-summary '<short title>'` with `gwtd board post` or `gwtd workspace update --agent-session <id>`.\n\
\n\
Do NOT post tool-level reports (e.g., \"running gcc\", \"opening file X\", \"ran test Y\"). \
Anything already visible in the diff or log does not need a Board entry.\n\
\n\
Examples:\n\
  gwtd board post --kind status --body $'Current state: focused tests are RED.\\n\\nReason: CLI and hook output still collapse multiline Board bodies.\\n\\nNext: implement block rendering.'\n\
  gwtd board post --kind question --mention user:akiojin --body $'Current state: two UX options remain.\\n\\nQuestion: should replies notify only the mentioned user or all viewers?'\n\
  gwtd board post --kind claim --mention branch:feature/foo --body $'Current state: I am taking the migration slice.\\n\\nBoundary: other agents should avoid files under crates/gwt-core/src/migration.rs.'\n\
  gwtd board post --kind handoff --mention agent:codex --body $'Current state: phase 1 is merged locally.\\n\\nNext: please run the Windows-focused verification and report failures.'\n";

pub(super) const USER_PROMPT_REMINDER_JA: &str = "# Board Post Reminder\n\
\n\
推論の節目または協調境界を越えたら、共有 Board に投稿してください。\
他の Agent と user が衝突せずに協調できます。\n\
\n\
投稿前に audience を選びます。特定の返答が不要な場合だけ broadcast にします。\
human user には `--mention user:<id>`、agent type には `--mention agent:<id>`、\
実行中 session には `--mention session:<id>`、workspace には `--mention branch:<name>` を使います。\
質問、blocker、handoff、next-step request、reply など返答が必要な投稿は mention 付きにします。\n\
\n\
Board 本文が human と AI agent の canonical message です。短い段落または bullet で読みやすく書き、\
他 Agent が必要とする協調情報は metadata に隠さず本文へ直接含めます。使いやすい本文形は:\n\
\n\
現在の状態: <何が変わったか、何が分かったか>\n\
\n\
理由: <なぜ重要か、なぜその判断にしたか>\n\
\n\
次: <次に何をするか。なければ省略>\n\
\n\
**推論軸**:\n\
- 作業 phase の遷移（例: implementation -> build check -> PR handoff）は `--kind status`。\n\
- 代替案の選択と理由（例: A vs B で B を選んだ）は `--kind decision` または `--kind status`。\n\
- 検証中の懸念や仮説（例: failure は Y 起因と見て検証中）は `--kind status`。\n\
\n\
**協調軸**:\n\
- claim — 担当範囲を宣言して衝突を避ける。`--kind claim`。\n\
- next — recipient を固定せず次の作業を共有する。`--kind next`。\n\
- blocked — unblock が必要な blocker を表に出す。`--kind blocked`。\n\
- handoff — 具体的な引き継ぎを渡す。`--kind handoff`。\n\
- decision — 確定した判断を共有する。`--kind decision`。\n\
\n\
特定 Agent 向けの投稿には `--target <session-id|branch|agent-id>` を繰り返し指定できます。\
targeted post は受信側 reminder injection の entry 行先頭に structured marker（現在は `>>` token）が付きます。\
新しい投稿では typed `--mention ...` を優先し、`--target` は older agent 互換に使います。\
broadcast の場合はどちらも省略します。\n\
\n\
**Workspace / Git environment guidance**:\n\
- AGENTS.md は project-local です。対象 repository に AGENTS.md がある場合はそれを優先し、\
gwt の AGENTS.md を他 project に適用しないでください。\n\
- branch / worktree を手動で作成、切替、削除しないでください（`git checkout -b`、\
`git switch -c`、`git branch -D`、`git worktree add/remove`）。Git 環境作成は gwt Start Work / \
Launch materialization が担当します。\n\
- Board は coordination/history log、Workspace は current state です。現在の task、summary、\
next action、focus が変わったら `gwtd workspace update` で Workspace を更新します。\n\
- Agent/window title bar では短い label と長い summary を分けます。`gwtd board post` または \
`gwtd workspace update --agent-session <id>` で `--title-summary '<short title>'` を使います。\n\
\n\
tool 単位の報告（例: \"running gcc\"、\"opening file X\"、\"ran test Y\"）は投稿しません。\
diff や log で既に分かる内容も Board entry にする必要はありません。\n\
\n\
Examples:\n\
  gwtd board post --kind status --body $'現在の状態: focused tests が RED です。\\n\\n理由: CLI と hook output が multiline Board body をまだ collapse しています。\\n\\n次: block rendering を実装します。'\n\
  gwtd board post --kind question --mention user:akiojin --body $'現在の状態: UX option が 2 つ残っています。\\n\\n質問: reply 通知は mention された user だけにしますか、全 viewer にしますか。'\n\
  gwtd board post --kind claim --mention branch:feature/foo --body $'現在の状態: migration slice を担当します。\\n\\nBoundary: 他 Agent は crates/gwt-core/src/migration.rs を避けてください。'\n\
  gwtd board post --kind handoff --mention agent:codex --body $'現在の状態: phase 1 は local merge 済みです。\\n\\n次: Windows-focused verification を実行して failure を報告してください。'\n";

pub(super) const USER_PROMPT_REMINDER_SHORT: &str = "# Board Post Reminder\n\
\n\
You posted to the Board recently. Post again only if a new reasoning milestone \
(phase change, alternative chosen, concern raised) or a coordination boundary \
(claim, next, handoff, blocked, decision) has emerged.\n\
\n\
When a response is expected, address the post with `--mention user:<id>`, \
`--mention agent:<id>`, `--mention session:<id>`, or `--mention branch:<name>`; \
omit mentions only for broadcast updates.\n\
\n\
The Board body remains the canonical message. Keep it readable with short paragraphs or bullets, \
and put AI coordination details in the body when another agent needs them.\n\
\n\
AGENTS.md is project-local. Do NOT create, switch, or delete branches/worktrees \
manually; gwt Start Work / Launch materialization owns Git environment creation.\n\
\n\
Board is history; Workspace is current state. If the work summary, next action, or focus changed, \
update Workspace with `gwtd workspace update`; use `--title-summary '<short title>'` for Agent/window title bars.\n";

pub(super) const USER_PROMPT_REMINDER_SHORT_JA: &str = "# Board Post Reminder\n\
\n\
最近 Board に投稿済みです。新しい推論の節目（phase change、alternative chosen、concern raised）\
または協調境界（claim、next、handoff、blocked、decision）が発生した場合だけ、追加で投稿してください。\n\
\n\
返答が必要な場合は `--mention user:<id>`、`--mention agent:<id>`、\
`--mention session:<id>`、`--mention branch:<name>` で宛先を指定します。\
broadcast は mention が不要なときだけ使います。\n\
\n\
Board 本文が canonical message です。短い段落または bullet で読みやすく書き、\
AI coordination details は必要な場合に本文へ入れてください。\n\
\n\
AGENTS.md は project-local です。branch / worktree を手動で作成、切替、削除しないでください。\
Git 環境作成は gwt Start Work / Launch materialization が担当します。\n\
\n\
Board は history、Workspace は current state です。work summary、next action、focus が変わったら \
`gwtd workspace update` で更新し、Agent/window title bar には `--title-summary '<short title>'` を使います。\n";

// Stop reminders are emitted as `systemMessage` (user-facing) because
// Claude Code's Stop hook schema does not accept `hookSpecificOutput`.
// Phrasing is therefore user-oriented rather than agent-oriented.
pub(super) const STOP_REMINDER: &str = "Board Post Reminder (Stop): the agent is stopping. If you \
expect a final handoff, prompt the agent to post what it completed to the shared Board \
with `gwtd board post --kind status --title-summary '<short title>'` before handing off. Board is history; Workspace is current \
state. If the work summary, next action, or focus changed, prompt the agent to update Workspace \
with `gwtd workspace update`; use `--title-summary '<short title>'` for Agent/window title bars.";

pub(super) const STOP_REMINDER_SHORT: &str = "Board Post Reminder (Stop): the agent posted to the \
Board recently; no additional completed-status post is required before stopping. If Workspace \
current state changed, update it with `gwtd workspace update`; use `--title-summary '<short title>'` for Agent/window title bars.";

pub(super) const STOP_REMINDER_JA: &str = "Board Post Reminder (Stop): agent が停止しようとしています。\
最終 handoff が必要な場合は、停止前に `gwtd board post --kind status --title-summary '<short title>'` で\
完了内容を共有 Board に投稿するよう促してください。Board は history、Workspace は current state です。\
work summary、next action、focus が変わった場合は `gwtd workspace update` で Workspace を更新し、\
Agent/window title bar には `--title-summary '<short title>'` を使います。";

pub(super) const STOP_REMINDER_SHORT_JA: &str = "Board Post Reminder (Stop): agent は最近 Board に投稿済みです。\
停止前に追加の completed-status post は不要です。Workspace current state が変わった場合は \
`gwtd workspace update` で更新し、Agent/window title bar には `--title-summary '<short title>'` を使います。";

pub(super) const INJECTION_HEADER: &str = "# Recent Board updates\n\n\
The following reasoning posts were made by other Agents since your last Board context. \
Consider whether any affect your current work phase. This is context, not a directive — \
you remain autonomous.\n\n";

pub(super) const INJECTION_HEADER_JA: &str = "# 最近の Board 更新\n\n\
前回の Board context 以降に、他 Agent が次の reasoning posts を投稿しました。\
現在の作業 phase に影響するか確認してください。これは context であり、directive ではありません。\
自律的に判断してください。\n\n";

pub(super) const SESSION_START_HEADER: &str = "# Current Board state\n\n\
Recent reasoning posts from other Agents (context, not a directive — you remain autonomous):\n\n";

pub(super) const SESSION_START_HEADER_JA: &str = "# 現在の Board 状態\n\n\
他 Agent の最近の reasoning posts（context であり directive ではありません。自律的に判断してください）:\n\n";

pub(super) fn user_prompt_reminder(lang: ReminderLanguage, short: bool) -> &'static str {
    match (lang, short) {
        (ReminderLanguage::Ja, true) => USER_PROMPT_REMINDER_SHORT_JA,
        (ReminderLanguage::Ja, false) => USER_PROMPT_REMINDER_JA,
        (ReminderLanguage::En, true) => USER_PROMPT_REMINDER_SHORT,
        (ReminderLanguage::En, false) => USER_PROMPT_REMINDER,
    }
}

pub(super) fn stop_reminder(lang: ReminderLanguage, short: bool) -> &'static str {
    match (lang, short) {
        (ReminderLanguage::Ja, true) => STOP_REMINDER_SHORT_JA,
        (ReminderLanguage::Ja, false) => STOP_REMINDER_JA,
        (ReminderLanguage::En, true) => STOP_REMINDER_SHORT,
        (ReminderLanguage::En, false) => STOP_REMINDER,
    }
}

pub(super) fn injection_header(lang: ReminderLanguage) -> &'static str {
    match lang {
        ReminderLanguage::Ja => INJECTION_HEADER_JA,
        ReminderLanguage::En => INJECTION_HEADER,
    }
}

pub(super) fn session_start_header(lang: ReminderLanguage) -> &'static str {
    match lang {
        ReminderLanguage::Ja => SESSION_START_HEADER_JA,
        ReminderLanguage::En => SESSION_START_HEADER,
    }
}

pub(super) fn no_recent_posts_line(lang: ReminderLanguage) -> &'static str {
    match lang {
        ReminderLanguage::Ja => "- (他 Agent からの最近の投稿はありません)\n",
        ReminderLanguage::En => "- (no recent posts from other Agents)\n",
    }
}

/// Format the narrative-output language directive appended to agent-facing
/// reminders (SessionStart / UserPromptSubmit). Stop reminders are
/// user-facing and do not receive this directive.
///
/// SPEC-1933 FR-010 / SC-003.
pub(super) fn format_language_directive(lang: &str) -> String {
    match reminder_language(lang) {
        ReminderLanguage::Ja => "\n**Use language: ja** for narrative outputs（Board 投稿本文と Workspace summaries。gwtd subcommands、flags、code examples は English のまま）。\n".to_string(),
        ReminderLanguage::En => "\n**Use language: en** for narrative outputs (Board post bodies and Workspace summaries; gwtd subcommands, flags, and code examples stay English).\n".to_string(),
    }
}

pub(super) fn title_summary_required_reminder(lang: &str) -> &'static str {
    match reminder_language(lang) {
        ReminderLanguage::Ja => "# Agent Title Required\n\
\n\
この Agent session にはまだ短い `title-summary` が設定されていません。実装や検証に入る前に、現在の作業目的を Workspace に明示してください。\n\
\n\
必須コマンド例:\n\
  gwtd workspace update --agent-session \"$GWT_SESSION_ID\" --current-focus '<現在の作業内容>' --title-summary '<短い作業タイトル>'\n\
\n\
必要に応じて同じ短いタイトルを Board milestone にも付けます:\n\
  gwtd board post --kind status --title-summary '<短い作業タイトル>' --body '<現在の状態 / 理由 / 次>'\n\
\n\
`title-summary` は Agent window tab と Workspace summary 用の短い作業名です。状態や結果ではなく「何の作業か」を書いてください。例: `エージェントタイトル改善`。不可: `エージェントタイトル改善完了`、`エージェントタイトル改善中`。完了/進行中/ブロック中などの状態は `--status`、`--current-focus`、`--summary`、または Board `--body` に分けてください。\n\
\n\
**Use language: ja** for narrative outputs（Board 投稿本文、Workspace summaries、Agent title-summary）。gwtd subcommands、flags、code examples は English のまま。\n",
        ReminderLanguage::En => "# Agent Title Required\n\
\n\
This Agent session does not have a short `title-summary` yet. Before implementation or verification work, explicitly publish the current work purpose to Workspace.\n\
\n\
Required command shape:\n\
  gwtd workspace update --agent-session \"$GWT_SESSION_ID\" --current-focus '<current work focus>' --title-summary '<short work title>'\n\
\n\
When useful, use the same short title on the Board milestone:\n\
  gwtd board post --kind status --title-summary '<short work title>' --body '<current state / reason / next>'\n\
\n\
`title-summary` is the short work name for Agent window tabs and Workspace summaries. Describe what the work is, not its status or result. Good: `Agent title improvement`. Bad: `Agent title improvement complete`, `Agent title improvement in progress`. Keep completion/progress/blocker state in `--status`, `--current-focus`, `--summary`, or Board `--body`.\n\
\n\
**Use language: en** for narrative outputs (Board post bodies, Workspace summaries, and Agent title-summary; gwtd subcommands, flags, and code examples stay English).\n",
    }
}
