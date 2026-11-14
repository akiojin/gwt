# Discussions

This file is the canonical gwt discussion log. Entries are updated in place while active and indexed by the `discussions` semantic scope.

## 2026-06-24 — Improvement Inbox needs review-first information architecture

Status: completed
Topics: gwt-discussion, self-improvement, improvement-inbox, ui-ux, information-architecture, user-verification
Related SPECs: #3164
Related Issues:
Related Works:
Promoted To: SPEC #3164 T-921〜T-926

Summary:
ユーザー視覚確認で、Improvement Inbox の一番上の pending row だけに Approve/Reject があり、Linked/Rejected rows が何なのか分かりにくいことが判明した。調査の結果、現在の UI は pending/promoted/linked/dismissed を同一リスト・同一視覚重みで表示しており、Inbox の主目的である「Issue 化してよい候補をレビューする」作業キューとして情報設計が不足している。初期案は `Needs Review` と `History` のセクション分割だったが、ユーザー視覚確認で `History` は別タブの方がよいと判断された。また Issue Preview は実際に登録される内容として薄すぎるため、GitHub Issue としてそのまま使える本文構造へ拡張する。

Evidence:
- Screenshot `.gwt/drop-files/1782279290161-37-image.png` shows one Pending candidate with Approve/Reject/Details, followed by Linked and Rejected rows without equivalent controls.
- SPEC #3164 FR-010 requires the Inbox to list pending/promoted/dismissed candidates and provide promote/dismiss/open linked Issue actions, but it does not require a single mixed list.
- `crates/gwt/web/improvement-inbox-surface.js` currently renders all candidates in one `.improvement-inbox-list` and only adds approve/reject controls when `state === "pending" || state === "parked"`.
- The existing behavior is semantically valid but visually misleading: processed rows appear as incomplete rows rather than history.
- Existing product UI already uses segmented controls / sections in nearby surfaces, but Improvement Inbox currently lacks a review queue vs history distinction.
- User follow-up: `History` should be a separate tab, not just a lower same-page section.
- Screenshot `.gwt/drop-files/1782281463968-41-image.png` shows the Issue Preview body is too thin: Summary/Candidate/Evidence/Details are not enough for an actionable upstream Issue.

## Discussion TODO - Review-First Inbox IA

### Proposal A - Split Needs Review and History [chosen]
- Summary: Make Improvement Inbox a review-first surface. Pending/Parked candidates appear in a `Needs Review` tab with primary review affordances and Issue preview. Linked/Promoted/Dismissed candidates appear in a separate `History` tab with explicit processed-state explanations, linked Issue actions, dismissed reason, and no Approve/Reject controls. The Issue Preview body must be the exact public Issue body and include Problem, Expected behavior, Observed evidence, Impact, Suggested verification, Source candidate, and Privacy sections.
- Open Questions: None.
- Dependency Checks: SPEC #3164 owns Improvement Inbox. Existing backend states and frontend data are sufficient for grouping without backend schema changes.
- Deferred Decisions: None for this implementation slice. History remains available as a tab so processed rows are discoverable without competing with the active review queue.
- Coverage Checks: Scope boundary, ownership, processed-state explanation, linked issue action, rejected reason, and empty `Needs Review` states are accepted as required coverage for this slice.
- Exit Blockers: None for implementation. PR/build completion remains blocked until fresh browser-check visual verification is confirmed.
- Next Question: None.
- Depth Mode: normal
- Question Ledger: Initial investigation found that the current mixed list causes processed rows to look like missing-action rows. Recommended approach was Review + History. User approved the Review + History IA on 2026-06-24, then refined it to a separate History tab and rejected the thin Issue Preview as insufficient.
- Depth Gate: complete
- Implementation Proof: Inspected `crates/gwt/web/improvement-inbox-surface.js`, `crates/gwt/web/styles/app.css`, `crates/gwt/web/__tests__/improvement-inbox-surface.test.mjs`, and screenshot `.gwt/drop-files/1782279290161-37-image.png`.
- SPEC/Issue Proof: `gwt-search` identified SPEC #3164 as owner; `issue.spec.read number:3164` confirms FR-010 and pending tasks T-917/T-920 are still awaiting visual confirmation.
- Gap Check Proof: Scope is frontend information architecture for Improvement Inbox; backend lifecycle operations remain unchanged. Ownership is SPEC #3164. Edge cases are empty review queue, processed linked rows, rejected rows, and modal action availability.
- Official Docs Proof: not-applicable: this is local gwt UI behavior and repo-local workflow.
- External Research Proof: not-applicable: local evidence and SPEC #3164 fully establish the gap.
- Evidence Gate: complete
- Promotable Changes: Update SPEC #3164 with a UI IA follow-up task and resume gwt-build-spec with frontend RED tests before implementation.

## 2026-06-24 — Improvement Inbox needs explicit approval controls

Status: completed
Topics: gwt-discussion, self-improvement, improvement-inbox, approval-ux, user-verification
Related SPECs: #3164
Related Issues:
Related Works:
Promoted To: SPEC #3164 T-914〜T-917

Summary:
ユーザー視覚確認で Improvement Inbox に承認・非承認を選択できる場所がないことが判明した。既存の pending row は `↑` / `×` の icon-only 操作で、public Issue 作成や reject の意味が読み取れない。選択肢として、row-level controls と detail modal の両方を採用し、Approve は即時作成せず in-app confirmation modal を開く。Reject は optional reason modal を開き、空理由でも reject できる。

Evidence:
- Screenshot `.gwt/drop-files/1782276629995-34-image.png` shows Improvement Inbox without visible approval/nonapproval controls.
- Search identified SPEC #3164 as the owner for gwt self-improvement intake and Improvement Inbox behavior.
- `crates/gwt/web/improvement-inbox-surface.js` had pending actions rendered as `↑` and `×`.
- SPEC #3164 tasks に T-914〜T-917 を追加し、explicit Approve/Reject/Details controls, Approve confirmation modal, optional Reject reason modal, and user visual verification を実装対象として固定した。
- Focused RED test confirmed old UI exposed `[ '↑', '×' ]` instead of `[ 'Approve', 'Reject', 'Details' ]`.
- `crates/gwt/web/improvement-inbox-surface.js` now renders pending row `Approve`, `Reject`, and `Details` actions.
- `Approve` now opens an in-app confirmation modal with `Create public Issue` and `Cancel`, and does not send the promote action until confirmed.
- `Reject` now opens an optional reason modal and sends `improvement_dismiss` with `reason` only when provided.
- `Details` now opens a detail modal with candidate target, source, confidence, cause, evidence, and Approve/Reject actions.
- Automated verification passed focused frontend tests, full frontend unit tests, frontend smoke tests, visual tests, Rust tests, clippy, and build.
- User visual verification screenshot `.gwt/drop-files/1782278299705-36-image.png` rejected the metadata-only Details modal: the expected review artifact is the exact public Issue content that will be registered, not just candidate metadata.
- `crates/gwt/src/cli/improvement.rs` now includes `issue_preview.repository/title/body` in candidate public JSON using the same `issue_title` and `render_public_issue_body` path as `improvement.promote_issue`.
- `crates/gwt/web/improvement-inbox-surface.js` now shows `Issue Preview`, repository, title, and the markdown Issue body in both Details and Approve confirmation modals before `Create public Issue`.
- Focused RED/GREEN tests now cover backend preview payload and frontend preview rendering in Details/Approve.

## Discussion TODO - Approval Controls

### Proposal A - Add explicit Approve Reject Details controls [chosen]
- Summary: Pending Improvement Inbox rows expose text controls for `Approve`, `Reject`, and `Details`. Approve and Reject open in-app modals so users can review the action before it mutates candidate state or creates a public Issue.
- Open Questions: None.
- Dependency Checks: SPEC #3164 owns the Improvement Inbox self-improvement surface. The existing backend operations can be reused: `improvement_promote_issue` for approval and `improvement_dismiss` for rejection.
- Deferred Decisions: None.
- Coverage Checks: Row affordance, approval confirmation, optional reject reason, details modal, and linked issue action paths are covered by focused frontend tests.
- Exit Blockers: None for implementation. PR handoff remains blocked until user visual verification confirms the fresh browser-check surface.
- Next Question: None.
- Depth Mode: normal
- Question Ledger: User selected both row-level controls and detail modal, optional reject reason, visible terms `Approve`/`Reject`, and in-app confirmation for row approval.
- Depth Gate: complete
- Implementation Proof: Implemented in `crates/gwt/web/improvement-inbox-surface.js` and `crates/gwt/web/styles/app.css` with tests in `crates/gwt/web/__tests__/improvement-inbox-surface.test.mjs`.
- SPEC/Issue Proof: SPEC #3164 tasks now include T-914〜T-917 for the approval UX follow-up.
- Gap Check Proof: The problem is a visual affordance gap, not a backend operation gap; existing promote and dismiss operations remain the mutation boundary.
- Official Docs Proof: not-applicable: behavior is local gwt UI and local gwtd operations.
- External Research Proof: not-applicable: repo-local evidence fully explains the failure.
- Evidence Gate: complete
- Promotable Changes: Applied to SPEC #3164 T-914〜T-916 and T-918〜T-919 locally. T-917/T-920 user visual verification remains pending.

## 2026-06-24 — Improvement Inbox Promote requires GitHub auth during visual check

Status: completed
Topics: gwt-discussion, self-improvement, improvement-inbox, browser-check, github-auth
Related SPECs: #3164
Related Issues:
Related Works:
Promoted To: SPEC #3164 T-911〜T-913

Summary:
browser-check の fresh gwt で Improvement Inbox の `↑` ボタンを押すと `Improvement action error: network error: authentication required` が出る。調査の結果、`↑` は非破壊の視覚確認操作ではなく `improvement.promote_issue` を実行し、upstream `akiojin/gwt` に実 Issue を作成する経路へ進む。fresh browser-check の隔離 HOME では `gh auth token` が解決できず、backend error がそのまま browser alert として表示されていたため、browser-check の起動手順で GitHub token を安全に bridge し、`↑` には public Issue 作成前の確認を追加した。

Evidence:
- Screenshot `.gwt/drop-files/1782232877921-32-image.png` shows the native alert after clicking `↑`.
- Fresh gwt URL `http://127.0.0.1:58420/` returns HTTP 200, but isolated HOME `gh auth token` exits 1 with no OAuth token.
- `HOME=<fresh-home> target/debug/gwtd improvement.promote_issue` reproduces `network error: authentication required`.
- `crates/gwt/web/improvement-inbox-surface.js` sends `improvement_promote_issue` on `↑`.
- `crates/gwt/src/app_runtime/mod.rs` routes the action to `ImprovementCommand::PromoteIssue`.
- `crates/gwt/src/cli/improvement.rs` creates an upstream `akiojin/gwt` issue through `create_issue_in_repo`.
- `crates/gwt/web/app.js` displays `ImprovementActionError` through `window.alert`.
- SPEC #3164 tasks に T-911〜T-913 を追加し、browser-check auth bridge、Promote confirmation、auth error UX を実装対象として固定した。
- `crates/gwt/web/improvement-inbox-surface.js` now confirms before sending `improvement_promote_issue`.
- `crates/gwt/src/app_runtime/mod.rs` now maps missing GitHub auth into a user-readable remediation message.
- `.codex/skills/browser-check/SKILL.md` and `.claude/skills/browser-check/SKILL.md` now pass `GH_TOKEN` / `GITHUB_TOKEN` into isolated browser-check launches without logging the token.

## Discussion TODO

### Proposal A - Bridge GitHub auth into browser-check for real Promote verification [chosen]
- Summary: Keep `↑` as a real upstream Issue promotion action. The browser-check launch path passes GitHub authentication into the isolated HOME so visual verification can confirm actual upstream gwt Issue creation instead of failing at `gh auth token`; the UI requires confirmation before creating the public Issue.
- Open Questions: None.
- Dependency Checks: SPEC #3164 owns Improvement Inbox; code path and CLI reproduction verified. Current browser-check symlinks `.config`, but macOS `gh` uses keyring and isolated HOME cannot resolve the token. Real HOME `gh auth token` succeeds, so launch-time token bridge via `GH_TOKEN` / `GITHUB_TOKEN` is the likely implementation path.
- Deferred Decisions: None.
- Coverage Checks: Scope boundary, ownership, auth failure path, confirmation UX, and browser-check token bridge checked. Final user-visible proof remains the fresh browser-check verification flow.
- Exit Blockers: None for implementation. PR handoff remains blocked until user visual verification is confirmed.
- Next Question: None.
- Depth Mode: normal
- Question Ledger: Q1 answered - user chose option 3: pass GitHub auth into browser-check and verify real upstream Issue creation. Q2 answered - user chose option 1: require confirmation before creating the public Issue.
- Depth Gate: complete
- Implementation Proof: Inspected `crates/gwt/web/improvement-inbox-surface.js`, `crates/gwt/src/app_runtime/mod.rs`, `crates/gwt/src/cli/improvement.rs`, `crates/gwt/web/app.js`; reproduced with isolated HOME `gwtd improvement.promote_issue`.
- SPEC/Issue Proof: Search identified SPEC #3164 as owner; tasks section now includes T-911〜T-913 for browser-check auth bridge, Promote confirmation UX, and auth error UX.
- Gap Check Proof: Scope is Improvement Inbox promote UX and browser-check auth bridging; ownership is SPEC #3164; failure path is missing GitHub auth in isolated HOME; migration impact is browser-check and normal authenticated use; verification covered focused frontend/backend tests plus full frontend/visual/Rust checks.
- Official Docs Proof: not-applicable: behavior is local gwt + local `gh` invocation already proven by commands.
- External Research Proof: not-applicable: repo-local evidence fully explains the failure.
- Evidence Gate: complete
- Promotable Changes: Applied to SPEC #3164 T-911〜T-913 and implemented locally. User visual confirmation remains pending before PR update.

## 2026-06-19 — Codex resume panes show Error during session restore

Status: completed
Topics: gwt-discussion, session-resume, codex, pane-status, workspace-restore
Related SPECs: #2359, #2014, #1935
Related Issues: #2546, #2995
Related Works:
Promoted To: SPEC #2359 US-79B / T-651〜T-655

Summary:
セッション復元時に Codex pane が `CODEX Codex Error` や blank terminal になり、復元できないように見える場合がある。添付画像では複数の resumed Codex pane が Error 表示になっている一方、対応する session TOML / runtime sidecar / OS process では `codex resume <agent_session_id>` が生存している。現時点では「resume プロセスが起動できない」よりも、「live agent session があるのに stale な PTY Error state が hook Running state より優先され、GUI 表示だけ Error 固定になる」可能性が高い。

Evidence:
- 添付画像では `work/20260617-0255` と `work/20260617-0425` 系の Codex pane が Error/blank 表示。
- `~/.gwt/sessions/*` では、それぞれ `session_mode = "Resume"` かつ `agent_session_id` 付きで Running sidecar が存在。
- `ps` では `/Users/akiojin/.bun/bin/codex --no-alt-screen resume <agent_session_id>` が該当セッションで生存。
- 対象 worktree / branch は存在しており、SPEC #2359 US-79 の「missing worktree/branch」ケースとは現在のスクリーンショット上では一致しない。
- `gwtd pane.list` は pane websocket timeout になり、UI/pane 更新系の詰まりも併発している可能性がある。
- `crates/gwt/src/window_state.rs` の `compose_window_state_with_active_session` は PTY state が `Error` の場合、Agent hook state を見ずに PTY state を返す。
- `crates/gwt/src/app_runtime/runtime_events.rs` は PTY status `Error` / `Stopped` で active agent session tracking と runtime tracking を落とすため、その後の hook event が live session を GUI 上で復旧しにくい。
- PR review で重複 PTY `Error` の境界も確認した。最初の recoverable `Error` で hook state を表示用に消しても、同じ window の duplicate `Error` では active session を保持し続ける必要がある。
- PR review で marker lifetime の境界も確認した。live hook recovery 後も duplicate 用 marker が残ると、後続の本物の `Error` まで recoverable と誤判定し得るため、次の hook state 受信時点で marker を消す必要がある。

Approaches:
- Approach 1: Agent window に active session と matching live runtime hook がある場合、PTY Error より hook state を優先する。実際の process exit と stale UI state の境界を test で固定できるが、auto-close / stopped 表示の既存期待を壊さないよう Stopped は対象外にする必要がある。
- Approach 2: hook Running/Waiting/Idle を受けたタイミングで同じ window の PTY terminal state を clear する。局所的だが、status composition の意図が分散する。
- Approach 3: pane websocket timeout / render blank の retry と status refresh を追加する。表示更新の堅牢性は上がるが、Error 固定の根本原因を直接直さない。

Current Recommendation:
最小の修正対象は Approach 1 を中心に、Agent resume pane の status composition と runtime tracking を SPEC #2359 の resume/restore follow-up として扱うこと。Recoverable `Error` は live hook evidence または同じ window で既に recoverable と判定済みの duplicate `Error` に限定し、duplicate 用 marker は次の hook state 受信で終了させる。Startup restore が missing worktree を skip する `crates/gwt/src/app_runtime/startup.rs` の US-79 ギャップは関連するが、今回の画像とは別症状として scope を分ける。

Open Questions:
- なし。

Next:
SPEC #2359 US-79B / T-651〜T-655 は完了。ユーザー視覚確認は 2026-06-19 に confirmed。PR #3118 / #3119 merge 直前の review follow-up として duplicate PTY `Error` regression と marker-clear regression も追加済み。

## 2026-05-23 — Workspace terminology and durable discussions

Status: active
Topics: workspace, work, discussion, semantic-search
Related SPECs: #2359
Related Works:
Promoted To:

Summary:
Workspace の意味が分かりにくい。Branch は作業空間、SPEC は仕様、Work は永続する作業単位として整理する。議論フェーズは Work ではなく Discussion として扱い、memory と同じくファイルへ保存し、セマンティック検索できるようにする。

Decisions:
- Discussion is not Work.
- Work remains durable until completion and can be persisted with completed status.
- A Work can cover multiple SPECs, and concrete tasks may be undecided at creation time.
- Past Work and Discussion records should be semantically searchable and can surface similar candidates during conversation.

Open Questions:
- Workspace という語を UI 上で Project State / Work / Discussion / Branch とどう分けると直感的か。

Next:
実データとして discussion log を保存し、discussions semantic index で検索できることを確認する。

## 2026-06-17 — Managed Hooks UX 5x follow-up

Status: chosen
Topics: managed-hooks, gwt-discussion, workflow-policy, hook-health
Related SPECs: #1935, #3050, #1942, #2077
Related Works:
Promoted To: #1935 Phase 22

Summary:
User selected all UX axes. SPEC #1935 now owns Phase 22: Managed Hooks Orchestrator UX. Agent/Work hook health strip is the primary user-facing surface; Hook Center/Settings and CLI/Board hook status/doctor are supporting surfaces backed by the same health model. Speed/quietness and safety are acceptance criteria, not separate tracks.

Decisions:
- Adopt all three axes: integrated UX, speed/quietness, and safety.
- Use SPEC #1935 as the owner and append Phase 22 to spec, plan, and tasks instead of creating a new SPEC.
- Primary surface: Agent/Work hook health strip. Supporting surfaces: Hook Center/Settings audit and CLI/Board hook status/doctor.
- Keep diagnostics out of hook stdout; expose health, profile, and recovery through explicit status surfaces.
- Treat stale binary/trust/asset recovery, linked-worktree Codex discovery, and delayed SessionStart as first-class UX states.
- Tighten workflow-policy safety as implementation-mutation owner readiness, while preserving read-only exploration and explicit low-risk exceptions.

Open Questions:
- None for planning. Implementation may split surfaces if TDD shows a narrower vertical slice is safer.

Next:
Action Bundle: run gwt-build-spec for #1935 Phase 22. Start with T-HUX2-001 through T-HUX2-006: RED tests and backend ManagedHookHealth read model before UI work.

## 2026-06-19 — Tray Copy URL menu displays port-bearing URL

Status: completed
Topics: tray-menu, copy-url, server-url
Related SPECs: #2785
Related Works:
Promoted To: SPEC #2785 tray menu Copy URL label amendment

Summary:
ユーザー要望は tray menu の `Copy URL` にポート番号を表示すること。現状は menu label が静的 `Copy URL` だが、clipboard へコピーされる値は `browser_url` の完全 URL で、`browser_url` は menu 作成前に確定している。既存 owner は SPEC #2785 Server URL Surface で、2026-06-09 Amendment が tray menu Copy URL surface を定義済み。

Decisions:
- `Copy URL` label は `Copy URL (http://127.0.0.1:<port>/)` 形式にする。表示値と clipboard へ入る root browser URL を一致させる。
- 新規 SPEC は作らず、SPEC #2785 の tray menu amendment / tasks へ小さな follow-up として統合する。
- 実装は label 生成 helper と focused contract test の追加を優先し、clipboard 処理・browser_url source は変更しない。

Open Questions:

Next:
Action Bundle: Update Spec + Resume Build。実装時は focused RED test で `Copy URL (<browser_url>)` label を固定し、`main.rs` の tray menu item label を browser_url 付きに変更する。検証は focused tray contract test、cargo test -p gwt --test tray_module_present_test、必要に応じて browser-check による tray menu 視覚確認。

## 2026-06-20 — Antigravity CLI visual verification route

Status: active
Topics: SPEC-1921, antigravity-cli, visual-verification
Related SPECs: #1921
Related Works:
Promoted To:

Summary:
ユーザー報告では fresh check で何も変わっていないように見える。調査の結果、スクリーンショットの失敗は Antigravity CLI 起動ではなく Start Work が remote branch を作る段階で GitHub 認証プロンプト不可により失敗していた。open_launch_wizard 経路の live Playwright check では Agent picker に Antigravity CLI と Gemini CLI (legacy) が期待順で表示されることを確認した。

Decisions:
- 実装差分は fresh binary に反映済み。Start Work branch 作成失敗は SPEC #1921 Antigravity descriptor/label slice とは別の確認導線または Start Work 認証 scope として扱う。

Open Questions:
- PR gate の視覚確認として既存 branch Launch Wizard の表示確認で進めるか、Start Work 認証失敗をこの流れで追加 scope として扱うか。

Next:
ユーザーに確認する: A) 既存 branch Launch Wizard の表示確認を採用して PR gate に進む、B) Start Work 認証失敗を別 Issue/SPEC scope として扱う、C) Antigravity 実起動まで current branch で追加確認する。

## 2026-07-04 — Index window UI improvement follow-up

Status: promoted
Topics: gwt-discussion, index-window, ui-ux, project-index, spec-1939
Related SPECs: #1939
Related Works:
Promoted To: SPEC #1939 follow-up amendment / plan/tasks update

Summary:
添付画像では dedicated Index window の Search/Health surface が機能はあるものの、空状態・scope状態・Health未取得状態の情報設計が薄く、巨大な余白と一文の説明だけで次の操作や現在のindex状態が読み取りにくい。現行実装は SPEC-1939 Phase 15 の `project-index-search-surface.js` / `index-settings-panel.js` / Index CSS に集中しており、backend protocolを変えずにUI密度、状態サマリ、空状態、Health tableの読みやすさを改善できる。方針は Search と Health の両方を整理し、A案 Unified Operator Workbench をベースにする。空状態/未取得状態は簡潔な状態 + 次アクション、SearchタブにはAbnormal-firstのHealth概要のみ常時表示し、Rebuild詳細はHealthタブに残す。No project/status unavailable時はIndex window内にOpen Project導線を追加せず、repair_required/error時もSearchを止めずにInline警告 + Health CTAに留める。Healthタブはsummary cardsをtable上部に追加し、tableは詳細/RebuildのSOTとして維持する。Health unavailableの空状態にはRefresh CTAを置く。検証は自動テスト + fresh browser-checkのユーザー視覚確認を必須にする。

Decisions:
- OwnerはSPEC-1939 Phase 15 follow-upとする。
- 改善主眼はSearchとHealthの両方を整理する。
- A案 Unified Operator Workbench をベースにする。
- 空状態と未取得状態は簡潔な状態 + 次アクションに留め、チュートリアル文や長い使い方案内は避ける。
- SearchタブにもHealth summaryを常時表示するが、Rebuild詳細操作はHealthタブに残す。
- SearchタブのHealth summaryはAbnormal-firstにし、ready scopeを全部並べず、ready countとwarning/errorだけを表示する。
- repair_required/error時もSearchは続行可能にし、Inline警告 + HealthタブCTAを出す。
- Healthタブにはsummary cardsを追加し、既存tableは詳細とRebuild操作として維持する。
- Health unavailable空状態にはRefresh CTAを置く。
- No project/status unavailable時はIndex内にOpen Project導線を追加しない。
- 実装時の検証はfrontend unit/source contract、Playwright index-status、fresh browser-check user visual confirmationを必須にする。

Open Questions:


Next:
Action Bundle: Update Spec + Update Plan + Resume Build。SPEC #1939 のspec/plan/tasksにIndex window UI polish follow-upを追加し、gwt-build-specでTDD実装へ渡す。

## 2026-07-09 — SPEC-2008 Window Size Fit & Equalize 追補

Status: promoted
Topics: window size fit, equalize, smart guides
Related SPECs: #2008
Related Works: 
Promoted To: SPEC #2008 2026-07-09 Amendment（US-28/US-29/FR-108〜FR-116/SC-068〜SC-073）+ plan/tasks Phase 40（T-347〜T-363）

Summary:
PowerPoint 風サイズ調整の要望を調査。端合わせ・リサイズ中サイズ一致吸着は既存 Smart Guides Amendment（US-27/FR-099-107、Phase 39 未実装）がカバー済み。追加で US-28 Equalize（focused 基準一括同サイズ化）と US-29 Fit（terminal cell grid / panel intrinsic、titlebar ボタン・handle double-click・hotkey の 3 経路）を SPEC-2008 に追補することをユーザーと確定。配信順序は Phase 39 先行、Fit/Equalize は Phase 40 として別 PR。

Decisions:
- 既存 SPEC-2008 が owner（新規 SPEC 不要）
- Equalize は Tile/Stack/Align と並ぶ Floating action、focused 基準・位置維持
- Fit は terminal=セルグリッド整数倍丸め、panel=intrinsic size、maximized は no-op
- 発動 3 経路すべて採用（titlebar ボタン / handle double-click / hotkey）
- Phase 39 実装先行、Phase 40 は別 PR

Open Questions:


Next:
Execute レーンが gwt-build-spec で Phase 39（T-328〜T-346 Smart Guides）→ Phase 40（T-347〜T-363 Fit/Equalize）の順に実装する。spec/plan/tasks への amendment 反映は 2026-07-09 に完了・roundtrip 検証済み。

## Discussion TODO - Window Size Fit & Equalize

### Proposal A - SPEC-2008 に Fit & Equalize を追補し Phase 39→40 で実装 [chosen]
- Summary: 端合わせ・吸着・リサイズ中サイズ一致は既存 Smart Guides Amendment（US-27 / FR-099〜FR-107、Phase 39 未実装）が正本。追加要望の Equalize（focused 基準一括同サイズ化、Floating action）と Fit（terminal cell grid 丸め / panel intrinsic size、titlebar ボタン・handle double-click・hotkey の 3 経路、maximized は no-op）を 2026-07-09 Amendment（US-28 / US-29 / FR-108〜FR-116 / SC-068〜SC-073）として spec/plan/tasks に追補する。実装は Phase 39 先行、Phase 40 は別 PR。
- Open Questions: None.
- Dependency Checks: SPEC-2008 が canvas window manager の owner。double-click Fit は Phase 39 の directional resize handles（T-336）に依存。Equalize は既存 arrange 経路の mode 追加で backend protocol を増やさない。
- Deferred Decisions: Fit hotkey の具体キーは実装時に既存 hotkey registry と衝突チェックの上で確定（contract test で固定）。最小ウィンドウサイズは SPEC #3263 rescope（2026-07-09 Board decision）で判明した frontend 420×260 と backend window_canvas.rs 360×260 の衝突を Phase 39 実装時に単一値へ確定するため、Phase 40 の FR-108/FR-111/FR-112 に書かれた 420×260 はその確定値に読み替える（#3263 T-009 の単一ソース化と整合させる）。
- Coverage Checks: scope boundary / ownership / failure・edge（min 420×260 clamp、cell metrics 未確定 no-op、maximized no-op、viewport clamp）/ migration（persisted 形式変更なし）/ verification（unit・contract・Playwright・browser-check）をカバー。
- Exit Blockers: none
- Next Question: none
- Depth Mode: normal
- Question Ledger: Q1 Fit の意味 → 3 解釈すべて採用（吸着 + ワンアクション同サイズ + コンテンツ Fit）。Q2 Equalize UX → 一括・focused 基準の Floating action。Q3 Fit 対象 → terminal cell grid と panel intrinsic の両方。Q4 発動経路 → titlebar ボタン / handle double-click / hotkey の 3 経路すべて。Q5 配信順序 → Phase 39 先行、Fit/Equalize は Phase 40 で別 PR。
- Depth Gate: complete
- Implementation Proof: crates/gwt/web/ に window-smart-guides.js 不存在、app.js の resizeState は単一ハンドル、SPEC-2008 tasks Phase 39（T-328〜T-346）全 unchecked を確認。
- SPEC/Issue Proof: issue.spec.section number:2008 で spec（US-27 / FR-099〜FR-107 / SC-064〜SC-067）、plan（Phase 39）、tasks（Phase 39、最終 T-346）を読了。FR-108 / SC-068 / T-347 が未使用であることを grep で確認。
- Gap Check Proof: scope = 2 機能追加（Equalize / Fit）+ 既存 Phase 39、ownership = SPEC-2008 単独、failure/edge = min clamp・metrics 不明 no-op・maximized no-op・viewport clamp、migration = persisted 形式変更なし、verification = SC-068〜SC-073 と browser-check。
- Official Docs Proof: not-applicable: 純ローカルの frontend/backend 挙動（PowerPoint は着想であり外部 API 依存なし）。
- External Research Proof: not-applicable: repo 内証跡と SPEC-2008 で確定できる。
- Evidence Gate: complete
- Promotable Changes: SPEC-2008 spec/plan/tasks へ 2026-07-09 Amendment と Phase 40（T-347〜T-363）を追記（.gwt/tmp-spec2008-amend-*.md 準備済み、適用は .gwt/tmp-spec2008-apply.md）。
