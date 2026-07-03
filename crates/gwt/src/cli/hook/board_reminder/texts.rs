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

pub(super) const USER_PROMPT_REMINDER_JA: &str = "# Board Post Reminder\n\
\n\
推論の節目または協調境界を越えたら、共有 Board に投稿してください。\
他の Agent と user が衝突せずに協調できます。\n\
\n\
投稿前に audience を選びます。特定の返答が不要な場合だけ `params.broadcast:true` にします。\
human user には `params.mentions` の `user:<id>`、agent type には `agent:<id>`、\
実行中 session には `session:<id>`、workspace には `branch:<name>` を使います。\
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
- 作業 phase の遷移（例: implementation -> build check -> PR handoff）は `params.kind:\"status\"`。\n\
- 代替案の選択と理由（例: A vs B で B を選んだ）は `params.kind:\"decision\"` または `params.kind:\"status\"`。\n\
- 検証中の懸念や仮説（例: failure は Y 起因と見て検証中）は `params.kind:\"status\"`。\n\
\n\
**協調軸**:\n\
- claim — 担当範囲を宣言して衝突を避ける。`params.kind:\"claim\"`。\n\
- next — recipient を固定せず次の作業を共有する。`params.kind:\"next\"`。\n\
- blocked — unblock が必要な blocker を表に出す。`params.kind:\"blocked\"`。\n\
- handoff — 具体的な引き継ぎを渡す。`params.kind:\"handoff\"`。\n\
- decision — 確定した判断を共有する。`params.kind:\"decision\"`。\n\
\n\
特定 Agent 向けの投稿には `params.targets` を指定できます。\
targeted post は受信側 reminder injection の entry 行先頭に structured marker（現在は `>>` token）が付きます。\
新しい投稿では typed `params.mentions` を優先し、`params.targets` は older agent 互換に使います。\
broadcast の場合はどちらも省略します。\n\
\n\
**Work / Git environment guidance**:\n\
- AGENTS.md は project-local です。対象 repository に AGENTS.md がある場合はそれを優先し、\
gwt の AGENTS.md を他 project に適用しないでください。\n\
- branch / worktree を手動で作成、切替、削除しないでください（`git checkout -b`、\
`git switch -c`、`git branch -D`、`git worktree add/remove`）。Git 環境作成は gwt Start Work / \
Launch materialization が担当します。\n\
- Board は coordination/history log、Work は current state です。現在の task、summary、\
next action、focus が変わったら `workspace.update` JSON envelope で Work を更新します。\n\
- Agent/window title bar では短い purpose label と長い summary を分けます。\
`workspace.update` の `params.purpose` を設定します。Board 投稿で purpose は更新しません。\n\
\n\
tool 単位の報告（例: \"running gcc\"、\"opening file X\"、\"ran test Y\"）は投稿しません。\
diff や log で既に分かる内容も Board entry にする必要はありません。\n\
\n\
Examples:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"status\",\"body\":\"現在の状態: focused tests が RED です。\\n\\n理由: CLI と hook output が multiline Board body をまだ collapse しています。\\n\\n次: block rendering を実装します。\"}}\n\
  JSON\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"question\",\"mentions\":[\"user:akiojin\"],\"body\":\"現在の状態: UX option が 2 つ残っています。\\n\\n質問: reply 通知は mention された user だけにしますか、全 viewer にしますか。\"}}\n\
  JSON\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"claim\",\"mentions\":[\"branch:feature/foo\"],\"body\":\"現在の状態: migration slice を担当します。\\n\\nBoundary: 他 Agent は crates/gwt-core/src/migration.rs を避けてください。\"}}\n\
  JSON\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"handoff\",\"mentions\":[\"agent:codex\"],\"body\":\"現在の状態: phase 1 は local merge 済みです。\\n\\n次: Windows-focused verification を実行して failure を報告してください。\"}}\n\
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

pub(super) const USER_PROMPT_REMINDER_SHORT_JA: &str = "# Board Post Reminder\n\
\n\
最近 Board に投稿済みです。新しい推論の節目（phase change、alternative chosen、concern raised）\
または協調境界（claim、next、handoff、blocked、decision）が発生した場合だけ、追加で投稿してください。\n\
\n\
返答が必要な場合は `params.mentions` の `user:<id>`、`agent:<id>`、\
`session:<id>`、`branch:<name>` で宛先を指定します。\
broadcast は `params.broadcast:true` で明示します。\n\
\n\
Board 本文が canonical message です。短い段落または bullet で読みやすく書き、\
AI coordination details は必要な場合に本文へ入れてください。\n\
\n\
AGENTS.md は project-local です。branch / worktree を手動で作成、切替、削除しないでください。\
Git 環境作成は gwt Start Work / Launch materialization が担当します。\n\
\n\
Board は history、Work は current state です。latest status、cumulative progress summary、next action、focus が変わったら \
`workspace.update` JSON envelope で更新します。これまで何をしたかは `params.progress_summary` に書き、Agent/window title bar には `params.purpose` を設定します。\n";

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

pub(super) const STOP_REMINDER_JA: &str = "Board Post Reminder (Stop): agent が停止しようとしています。\
最終 handoff が必要な場合は、停止前に `board.post` JSON envelope で\
完了内容を共有 Board に投稿するよう促してください。Board は history、Work は current state です。\
work summary、next action、focus が変わった場合は `workspace.update` JSON envelope で Work を更新し、\
Agent/window title bar には `params.purpose` を設定します。";

pub(super) const STOP_REMINDER_SHORT_JA: &str = "Board Post Reminder (Stop): agent は最近 Board に投稿済みです。\
停止前に追加の completed-status post は不要です。Work current state が変わった場合は \
`workspace.update` JSON envelope で更新し、Agent/window title bar には `params.purpose` を設定します。";

pub(super) const MEMORY_UPDATE_REMINDER: &str = "# Memory Reminder\n\
\n\
If this task produced a reusable lesson, decision, failure pattern, or agent workflow correction, \
run a JSON envelope with operation `memory.add` \
before declaring the work done. It writes `.gwt/work/memory.md` with `Type`, `Context`, `Learning`, and \
`Future Action` fields. Legacy `tasks/memory.md` / `tasks/lessons.md` are only a compatibility fallback; prefer \
`.gwt/work/memory.md` for new memory.\n";

pub(super) const MEMORY_UPDATE_REMINDER_JA: &str = "# Memory Reminder\n\
\n\
この作業で再利用できる lesson、decision、failure pattern、agent workflow correction が生まれた場合は、\
完了宣言前に operation `memory.add` の JSON envelope \
を実行してください。この command は `Type`、`Context`、`Learning`、`Future Action` 付きで \
`.gwt/work/memory.md` に記録します。legacy `tasks/memory.md` / `tasks/lessons.md` は互換 fallback のみです。\n";

pub(super) const MEMORY_UPDATE_STOP_REMINDER: &str = "Memory Reminder (Stop): if this run produced a reusable lesson, decision, failure pattern, or agent workflow correction, prompt the agent to run a JSON envelope with operation `memory.add` before stopping. The command writes `.gwt/work/memory.md` with `Type`, `Context`, `Learning`, and `Future Action` fields.";

pub(super) const MEMORY_UPDATE_STOP_REMINDER_JA: &str = "Memory Reminder (Stop): この実行で再利用できる lesson、decision、failure pattern、agent workflow correction が生まれた場合は、停止前に operation `memory.add` の JSON envelope を実行するよう agent に促してください。この command は `Type`、`Context`、`Learning`、`Future Action` 付きで `.gwt/work/memory.md` に記録します。";

pub(super) const PROGRESS_SUMMARY_MISSING_REMINDER: &str = "# Progress Summary Reminder\n\
\n\
This Workspace has no `progress_summary` yet. Before continuing, write a cumulative summary of what has been investigated, decided, implemented, and verified so far. Keep the short latest status in `summary`; do not collapse the two.\n\
\n\
Run:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"progress_summary\":\"<cumulative detail of what has happened so far>\",\"summary\":\"<latest status snapshot>\",\"current_focus\":\"<what you are doing now>\"}}\n\
  JSON\n";

pub(super) const PROGRESS_SUMMARY_MISSING_REMINDER_JA: &str = "# Progress Summary Reminder\n\
\n\
この Workspace にはまだ `progress_summary` がありません。続行前に、これまで調査・判断・実装・検証した内容を累積要約として書いてください。短い直近状態は `summary` に残し、2 つを混ぜないでください。\n\
\n\
実行:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"progress_summary\":\"<これまで何をしていたかの詳細要約>\",\"summary\":\"<直近の状態>\",\"current_focus\":\"<現在の作業>\"}}\n\
  JSON\n";

pub(super) const PROGRESS_SUMMARY_STALE_REMINDER: &str = "# Progress Summary Stale\n\
\n\
The Workspace `progress_summary` has not changed for several turns while current focus or latest status changed. Refresh it with the cumulative story of what has happened so far; keep point-in-time status in `summary`.\n\
\n\
Run:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"progress_summary\":\"<updated cumulative progress summary>\",\"summary\":\"<latest status snapshot>\",\"current_focus\":\"<what you are doing now>\"}}\n\
  JSON\n";

pub(super) const PROGRESS_SUMMARY_STALE_REMINDER_JA: &str = "# Progress Summary Stale\n\
\n\
current_focus や直近状態が変わっている一方で、Workspace の `progress_summary` が複数ターン更新されていません。これまで何が起きたかの累積ストーリーを更新してください。時点の状態は `summary` に分けます。\n\
\n\
実行:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"progress_summary\":\"<更新した累積の詳細要約>\",\"summary\":\"<直近の状態>\",\"current_focus\":\"<現在の作業>\"}}\n\
  JSON\n";

pub(super) const PROGRESS_SUMMARY_STOP_REMINDER: &str = "Progress Summary Reminder (Stop): before stopping, ask the agent to update Work with `params.progress_summary` so the Workspace detail records what was investigated, decided, implemented, and verified. Keep short latest status in `params.summary`.";

pub(super) const PROGRESS_SUMMARY_STOP_REMINDER_JA: &str = "Progress Summary Reminder (Stop): 停止前に、Workspace detail に調査・判断・実装・検証の累積経緯が残るよう `params.progress_summary` で Work を更新するよう agent に促してください。短い直近状態は `params.summary` に分けます。";

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

pub(super) fn memory_update_reminder(lang: &str, stop: bool) -> &'static str {
    match (reminder_language(lang), stop) {
        (ReminderLanguage::Ja, true) => MEMORY_UPDATE_STOP_REMINDER_JA,
        (ReminderLanguage::Ja, false) => MEMORY_UPDATE_REMINDER_JA,
        (ReminderLanguage::En, true) => MEMORY_UPDATE_STOP_REMINDER,
        (ReminderLanguage::En, false) => MEMORY_UPDATE_REMINDER,
    }
}

pub(super) fn progress_summary_reminder(lang: &str, stale: bool, stop: bool) -> &'static str {
    match (reminder_language(lang), stale, stop) {
        (ReminderLanguage::Ja, _, true) => PROGRESS_SUMMARY_STOP_REMINDER_JA,
        (ReminderLanguage::En, _, true) => PROGRESS_SUMMARY_STOP_REMINDER,
        (ReminderLanguage::Ja, true, false) => PROGRESS_SUMMARY_STALE_REMINDER_JA,
        (ReminderLanguage::Ja, false, false) => PROGRESS_SUMMARY_MISSING_REMINDER_JA,
        (ReminderLanguage::En, true, false) => PROGRESS_SUMMARY_STALE_REMINDER,
        (ReminderLanguage::En, false, false) => PROGRESS_SUMMARY_MISSING_REMINDER,
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
        ReminderLanguage::Ja => "\n**Use language: ja** for narrative outputs（Board 投稿本文と Work summaries。gwtd subcommands、flags、code examples は English のまま）。\n".to_string(),
        ReminderLanguage::En => "\n**Use language: en** for narrative outputs (Board post bodies and Work summaries; gwtd subcommands, flags, and code examples stay English).\n".to_string(),
    }
}

pub(super) fn title_summary_required_reminder(lang: &str) -> &'static str {
    match reminder_language(lang) {
        ReminderLanguage::Ja => "# Agent Title — 応答する前に必ず設定\n\
\n\
この Agent window にはまだ `title-summary` が設定されていません。ユーザーへの応答を始める前に、**最初のアクションとして**この window の作業の目的を title-summary に設定してください。これは任意ではありません。\n\
\n\
まず最初に実行:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"<作業の目的（短い作業名）>\",\"current_focus\":\"<現在の作業内容>\"}}\n\
  JSON\n\
\n\
ルール:\n\
- title-summary には「何の作業か（作業の目的）」を書きます。状態や結果ではありません。\n\
- 入力された生プロンプトをそのままコピーしないでください。\n\
- 目的がまだ固まっていない場合でも、それっぽい暫定の目的を今すぐ設定し、目的が定まったら同じ title-summary を更新します（応答を遅らせないでください）。\n\
- `browser check`・検証・マージ・サーバー起動 のような一時的な activity 名は title にしません。activity は `current_focus` に書き、既に purpose がある場合はそれを保持します。\n\
- 例: `エージェントタイトル目的化`。不可: `…完了`、`…中`、生プロンプトのコピー。\n\
\n\
完了/進行中/ブロック中などの状態は `status`、`current_focus`、`summary`、または Board `body` に分けてください。設定が済むまで毎ターンこの指示を再掲します。\n\
\n\
**Use language: ja** for narrative outputs（Board 投稿本文、Work summaries、Agent title-summary）。gwtd subcommands、flags、code examples は English のまま。\n",
        ReminderLanguage::En => "# Agent Title — set it before you respond\n\
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
Keep completion/progress/blocker state in `status`, `current_focus`, `summary`, or Board `body`. This instruction repeats every turn until the title is set.\n\
\n\
**Use language: en** for narrative outputs (Board post bodies, Work summaries, and Agent title-summary; gwtd operation names, JSON field names, and code examples stay English).\n",
    }
}

pub(super) fn title_summary_stale_reminder(lang: &str) -> &'static str {
    match reminder_language(lang) {
        ReminderLanguage::Ja => "# Agent Title Stale\n\
\n\
`title-summary` が複数ターン同じ値のままで、`current_focus` だけが変化しています。実装の scope が本当に変わった場合は title を更新してください。phase / activity の変化だけなら title は変えずに `params.current_focus` だけ更新する運用が正しいです。\n\
\n\
更新する場合のコマンド例:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"<新しい作業 scope>\"}}\n\
  JSON\n\
\n\
`title-summary` は作業の scope です。phase / activity descriptor (`PR チェック中`、`verifying tests`、`fixing bug` 等) は `current_focus` または Board `body` に分けます。\n",
        ReminderLanguage::En => "# Agent Title Stale\n\
\n\
The `title-summary` has stayed unchanged for several UserPromptSubmit turns while `current_focus` has shifted. If the work scope actually changed, update the title; if only the phase / activity changed, leave the title and update `params.current_focus` only.\n\
\n\
Command to refresh the title:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"<new work scope>\"}}\n\
  JSON\n\
\n\
`title-summary` is the work scope; phase / activity descriptors (`PR check in progress`, `verifying tests`, `fixing bug`, etc.) belong in `current_focus` or the Board `body`, not in the title.\n",
    }
}
