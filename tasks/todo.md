# TODO: GitHub Copilot CLI 対応

## 背景（Copilot CLI 対応）

gwt に 5 番目の AI コーディングエージェントとして GitHub Copilot CLI（`copilot` コマンド、npm: `@github/copilot`）を追加する。既存の 4 エージェント（Claude Code / Codex / Gemini / OpenCode）と同じパターンに従い、最小限の変更で統合する。

仕様 Issue: #1411

## 実装ステップ（Copilot CLI 対応）

- [x] T000 gwt-spec Issue 作成 (#1411)
- [x] T001 Rust テスト追加（terminal.rs — TDD）
- [x] T002 フロントエンドテスト追加（TDD）
- [x] T003 terminal.rs — 5 つの match 関数に copilot アーム追加
- [x] T004 agents.rs — detect_copilot() 追加 + detect_agents 登録
- [x] T005 agentUtils.ts — AgentId 型 + inferAgentId に copilot 追加
- [x] T006 agentLaunchFormHelpers.ts — supportsModelFor() に copilot 追加
- [x] T007 AgentLaunchForm.svelte — modelOptions に copilot 用モデル一覧追加
- [x] T008 agentLaunchFormHelpers.test.ts — copilot テストアサーション追加
- [x] T009 agentUtils.test.ts — copilot テストアサーション追加
- [x] T010 cargo test 検証
- [x] T011 フロントエンドテスト検証（pnpm test）

## 検証結果（Copilot CLI 対応）

- [x] `cargo test -p gwt-tauri -- copilot` — 6 テスト全パス（548 テスト中）
- [x] `cd gwt-gui && pnpm test` — 65 ファイル / 1394 テスト全パス
- [x] `npx svelte-check` — エラー 0 件

---

## TODO: Issue一覧の無限スクロール "Loading more" フリーズ修正

## 背景（Cleanup）

Issue一覧パネルの無限スクロールで「Loading more」中にUIがフリーズする。
根本原因: 同期 Tauri コマンドが IPC スレッドをブロック + O(n^2) ページネーション + ブランチリンク検索のブロック + IntersectionObserver 再発火不良。

## 実装ステップ（Cleanup）

- [x] T001 GitHub Issue 仕様策定（gwt-spec ラベル）→ #1408
- [x] T002 TDD テスト作成（RED 確認）
  - [x] T002a Rust: Search API エンドポイント生成テスト（4件）
  - [x] T002b Rust: Search API レスポンスパーステスト（7件）
  - [x] T002c Frontend: 無限スクロール継続ロードテスト（2件）
- [x] T003 Fix 1: Issue コマンドの async 化（IPC ブロック解消）
- [x] T004 Fix 2+3: フロントエンド改善（非同期ブランチリンク + IO 修正）
- [x] T005 Fix 4: O(1) ページネーション（REST Search API）
- [x] T006 全テスト GREEN 確認 + lint + 型チェック

## 検証結果（Cleanup）

- [x] `cargo test -p gwt-core --lib` — 1422 tests passed（新規11件含む）
- [x] `cargo test -p gwt-tauri` — 32 tests passed
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
- [x] `cd gwt-gui && pnpm test` — 34 tests passed（新規2件含む）
- [x] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` — 0 errors

---

## fix: Cleanup — 保護ブランチの事前表示とエラーハンドリング (#1404)

## 背景（Cleanup保護ブランチ）

Cleanup処理でリポジトリルールにより保護されたブランチの削除が HTTP 422 でハードエラーになる問題の修正。
事前表示 + エラーハンドリングの2段階で対応。

## 実装ステップ（Cleanup保護ブランチ）

- [x] T001 gwt-spec Issue 作成 (#1404)
- [x] T002 Rust テスト追加 — classify_delete_branch_error / get_branch_deletion_rules
- [x] T003 `classify_delete_branch_error` に protected ケース追加
- [x] T004 `get_branch_deletion_rules()` 追加
- [x] T005 `get_cleanup_branch_protection` Tauri コマンド追加 + 登録
- [x] T006 `cleanup_worktrees` の "Protected:" ハンドリング
- [x] T007 Frontend テスト追加 — branchProtection / badge (4テスト)
- [x] T008 `CleanupModal.svelte` に保護状態の取得・表示
- [x] T009 全テスト通過確認
- [x] T010 clippy 検証

## 検証結果（Cleanup保護ブランチ）

- [x] `cargo test -p gwt-core` — 全テスト通過
- [x] `cargo test -p gwt-tauri` — 540テスト通過
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
- [x] `cd gwt-gui && pnpm test src/lib/components/CleanupModal.test.ts` — 39テスト通過
- [x] `npx markdownlint-cli` — 4つの SKILL.md エラーなし

---

## TODO: Issue #1265 再発（v8.1.1）対策の正規化境界強化

## 背景（v8.1.1 再発）

Issue #1265 にて `v8.1.1` でも Windows Launch Agent の `npx.cmd` 起動失敗が報告されたため、コマンド正規化を launch/probe/path-resolve 境界で再統一し、再発検知テストを拡張する。

## 実装ステップ（v8.1.1 再発）

- [x] T001 `runner.rs` で command path resolve 前にトークン正規化を適用
- [x] T002 `build_fallback_launch` の resolved command を共通正規化
- [x] T003 `terminal.rs` の launch command 正規化を OS 条件分岐から共通化
- [x] T004 `terminal.rs` の command probe (`--version`, `features list`) で同一正規化を適用
- [x] T005 回帰テスト追加（wrapped resolved path / wrapped lookup token / probe normalization）
- [x] T006 対象テストと `cargo fmt --check` で検証

## 検証結果（v8.1.1 再発）

- [x] `cargo test -p gwt-core terminal::runner -- --test-threads=1`
- [x] `cargo test -p gwt-core terminal::pty -- --test-threads=1`
- [x] `cargo test -p gwt-tauri normalize_launch_command_for_platform -- --test-threads=1`
- [x] `cargo test -p gwt-tauri normalized_process_command -- --test-threads=1`
- [x] `cargo fmt --all -- --check`

---

## TODO: Worktree詳細ビューPRタブが常に"No PR"になるバグ修正

## 背景（Worktree PR表示バグ）

コミット `18ee87c4` で `resolvedPrNumber` に `state !== "OPEN"` チェック追加。
副作用として MERGED/CLOSED PR しか持たないブランチで常に "No PR" となるリグレッション発生。
修正方針: PR state ではなくブランチ名で staleness を判定する。

## 実装ステップ（Worktree PR表示バグ）

- [x] T001 テスト修正（TDD RED）: MERGED PR表示のアサーション変更
- [x] T002 RED確認: テスト実行し失敗を確認
- [x] T003 実装: `latestBranchPrBranch` 変数追加・resolvedPrNumber ロジック変更
- [x] T004 GREEN確認: テスト全87件 pass
- [x] T005 Lint/型チェック検証: svelte-check 0 errors

---

## TODO: Issue #1265 追加修正（2026-03-03）

## 背景（Issue #1265 追加修正）

Issue #1265 の再発に対して、単純な正規化強化だけでは不十分だったため、
Launch Agent の責務を「事前可用性判定」から「実行時解決と実行時エラー返却」に寄せる。

仕様同期先: <https://github.com/akiojin/gwt/issues/1304>

## 実装ステップ（Issue #1265 追加修正）

- [x] T101 `terminal.rs` の runner 解決を `bunx` 優先に統一
- [x] T102 `npx` 経路で `--yes` 付与を保証
- [x] T103 `installed` 選択時の事前フォールバック（latest への書換）を廃止
- [x] T104 `AgentLaunchForm.svelte` で `installed` を常時選択肢に表示
- [x] T105 `StatusBar.svelte` の agent 可用性表示を削除
- [x] T106 対応テストを更新（Rust + Frontend test file adjustments）

## TDD / 検証結果

- [x] `cargo test -p gwt-tauri normalize_launch_command_for_platform -- --test-threads=1`
- [x] `cargo test -p gwt-tauri normalized_process_command -- --test-threads=1`
- [x] `cargo test -p gwt-tauri build_runner_launch -- --test-threads=1`
- [x] `cargo test -p gwt-core terminal::pty -- --test-threads=1`
- [x] `cargo fmt --all -- --check`
- [ ] `gwt-gui` の vitest 再実行（pnpm / node_modules 環境不整合の復旧後に実施）

---

## TODO: Windows タブ切り替え時のフリッカー修正

## 背景（Windows タブフリッカー）

Windows 環境でタブ切り替え時に `$derived`（同期）と `$effect`（非同期）のタイミング差で 1 フレーム全ターミナル非表示のギャップが生じ、背景フラッシュが発生する。`isTerminalTabVisible()` が `visibleTerminalTabId`（`$effect` で非同期更新）に依存していることが根本原因。

## 実装ステップ（Windows タブフリッカー）

- [x] T001 gwt-spec Issue 作成 (#1410)
- [x] T002 TDD テスト追加（jsdom では $effect フラッシュ済みのため GREEN だが仕様テストとして有効）
- [x] T003 `isTerminalTabVisible()` 修正（GREEN 化）
- [x] T004 テスト GREEN 確認（33/33 pass）
- [x] T005 型チェック・lint 確認（svelte-check 0 errors）
- [x] T006 コミット＆プッシュ（e637213f）

## 検証結果（Windows タブフリッカー）

- [x] `pnpm test src/lib/components/MainArea.test.ts` — 33 tests passed
- [x] `npx svelte-check --tsconfig ./tsconfig.json` — 0 errors, 1 warning（既存・変更対象外）
- [x] `bunx commitlint --from HEAD~1 --to HEAD` — 通過

---

## fix: 音声入力が動作しない（ランタイムセットアップ・リトライ・Python互換性）

## 背景（音声入力）

音声設定画面で「Voice input runtime unavailable: Voice runtime is unavailable: Missing Python package(s)」と表示され、
音声入力が使えない。根本原因は5つ（設定UIにセットアップ手段なし、リトライ不可、メッセージ冗長、Python 3.13 未対応、API 不統一）。

仕様 Issue: #1429

## 実装ステップ（音声入力）

- [x] T001 gwt-spec Issue 作成 (#1429)
- [x] T002 SettingsPanel.svelte — Setup ボタン追加 + 警告メッセージ改善
- [x] T003 voiceInputController.ts — リトライロジック修正（runtimeBootstrapSucceeded）+ sendToTerminal API 統一
- [x] T004 voiceInputController.test.ts — テスト更新（write_terminal→send_keys_to_pane 15箇所）+ リトライテスト2件追加
- [x] T005 voice.rs — python3.13 追加 + validate_python_version() + メッセージ改善
- [x] T006 全検証

## 検証結果（音声入力）

- [x] `cd gwt-gui && pnpm test src/lib/voice/voiceInputController.test.ts` — 79 tests passed
- [x] `cargo test -p gwt-tauri -- commands::voice::tests` — 6 tests passed
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
- [x] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` — 0 errors, 1 warning（既存・変更対象外）

## TDD 逸脱の記録

本修正ではプランに基づきテストと実装を同時に書いた（RED → GREEN サイクルを経ていない）。
既存テストの更新（write_terminal→send_keys_to_pane）とリトライ動作テスト2件の追加は実施済み。
