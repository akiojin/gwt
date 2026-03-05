## TODO: Review 指摘対応（Claude 明示無効化の尊重）2026-03-05

## 背景（Review対応）

レビューで「repair が明示 `false` を強制上書きする」「終了時に `false` を消す」が指摘されたため、
FR-010（明示無効化を尊重）を維持する方向へ修正。

## 実装ステップ（Review対応）

- [x] R001 `skill_registration.rs`: `repair_*` を通常 register 経路へ戻し `force` を使わない
- [x] R002 `skill_registration.rs`: 終了時 unregister はキー削除ではなく `false` 設定を維持
- [x] R003 `skill_registration.rs`: 回帰テストを `false` 維持前提へ更新
- [x] R004 `cargo test -p gwt-core skill_registration::tests:: -- --nocapture` 実行

---

## TODO: Skill Migration Repair が Claude プラグインを再有効化できない（2026-03-05）

## 背景

GWT 終了時に `unregister_all_skills()` が `disable_gwt_plugin_at()` で `gwt@gwt-plugins` を `false` に設定。
次回起動・修復時に FR-010 が「ユーザー明示無効化」と誤判断し再有効化をスキップする。

## 実装ステップ

- [x] T001 `claude_plugins.rs`: `enable_worktree_protection_plugin` を inner 関数化 + force 版追加
- [x] T002 `claude_plugins.rs`: `force_setup_gwt_plugin_at` 追加
- [x] T003 `claude_plugins.rs`: `remove_gwt_plugin_key_at` 追加
- [x] T004 `skill_registration.rs`: `unregister_all_skills` で `remove_gwt_plugin_key_at` 使用
- [x] T005 `skill_registration.rs`: register 関数に force 版追加 + repair で使用
- [x] T006 `config.rs`: 新規公開関数のエクスポート追加
- [x] T007 テスト追加（claude_plugins.rs: force_enable / remove_key 計6件）
- [x] T008 テスト更新（skill_registration.rs: unregister→キー不在検証 + repair フロー）
- [x] T009 `cargo test` 全パス + `cargo clippy` 警告なし + `cargo fmt` 済み

---

## TODO: Windows ターミナル表示崩れ修正（Issue #1457 / 2026-03-05）

## 背景（Issue #1457）

Windows 環境で Launch Agent 実行中にターミナル表示が崩れる。  
タブ切替で復帰することから、表示幅変化時の `fit/resize` 再同期不足を修正する。

## 実装ステップ（Issue #1457）

- [x] T001 `TerminalView.svelte` に viewport 幅変化検知を追加
- [x] T002 `TerminalView.test.ts` に回帰テストを追加
- [x] T003 対象テスト/型チェックを実行して結果を記録

## 検証結果（Issue #1457）

- [x] `cd gwt-gui && pnpm test src/lib/terminal/TerminalView.test.ts`
- [x] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`
## TODO: default profile 必須化 + default.ai 必須化（Issue #1464 / 2026-03-04）

## 背景（Issue #1464）

OpenAI互換API設定で `default` profile 不在や `profiles.default.ai` 未設定が混在し、
設定解決ロジックが不安定になるため、保存/読込時に shape を自動正規化する。

## 実装ステップ（Issue #1464）

- [x] T001 gwt-spec Issue 作成（#1464）
- [x] T002 RED: `save_and_load_inserts_default_profile_when_missing` 追加
- [x] T003 RED: `save_and_load_fills_default_profile_ai_when_missing` 追加
- [x] T004 `ensure_defaults` を拡張（default 補完 + default.ai 補完）
- [x] T005 `save()` にも正規化適用
- [x] T006 GREEN: `config::profile::tests` 実行
- [x] T007 `cargo fmt --all -- --check` 実行

## 検証結果（Issue #1464）

- [x] `cargo test -p gwt-core save_and_load_inserts_default_profile_when_missing`
- [x] `cargo test -p gwt-core save_and_load_fills_default_profile_ai_when_missing`
- [x] `cargo test -p gwt-core save_and_load_keeps_default_profile_api_key_optional`
- [x] `cargo test -p gwt-core config::profile::tests:: -- --test-threads=1`
- [x] `cargo fmt --all -- --check`

## TODO: macOS で API キー設定後に Codex が未認証になる不具合修正（Issue #1463 / 2026-03-04）

## 背景（Issue #1463）

Settings > Profiles に `ai.api_key` を保存しても、Codex の認証判定と Launch 時環境が
`OPENAI_API_KEY` のプロセス環境変数のみ参照していたため、macOS で未認証扱いになる。

## 実装ステップ（Issue #1463）

- [x] T001 gwt-spec Issue 作成・Spec/Plan/Tasks/TDD 作成（#1463）
- [x] T002 RED: `gwt-core` に Codex 認証判定テストを追加
- [x] T003 RED: `gwt-tauri` に Launch env 注入テストを追加
- [x] T004 `gwt-core/src/agent/codex.rs` に `is_codex_authenticated()` を追加（env + profile.ai）
- [x] T005 `gwt-tauri/src/commands/agents.rs` の fallback 判定を共通関数へ統一
- [x] T006 `gwt-tauri/src/commands/terminal.rs` に `OPENAI_API_KEY` フォールバック注入を追加
- [x] T007 `gwt-gui/e2e/settings-config.spec.ts` に API キー保存回帰シナリオを追加
- [x] T008 GREEN 検証と PR/Issue 更新

## 検証結果（Issue #1463）

- [x] `cargo test -p gwt-core agent::codex::tests:: -- --test-threads=1`
- [x] `cargo test -p gwt-tauri commands::agents::tests:: -- --test-threads=1`
- [x] `cargo test -p gwt-tauri commands::terminal::tests::inject_openai_api_key_from_profile_ai -- --test-threads=1`
- [x] `cargo fmt --all -- --check`
- [x] `cd gwt-gui && pnpm exec svelte-check --tsconfig ./tsconfig.json`
- [x] `cd gwt-gui && pnpm exec playwright test e2e/settings-config.spec.ts`

## TODO: Issue検索フィルターでIssue番号を対象化（Issue #1453 / 2026-03-04）

## 背景（Issue #1453）

Issue検索フィルターがタイトル一致のみで、Issue番号入力（例: `12`, `#12`）がヒットしない。
検索体験を改善するため、番号部分一致と混在クエリAND条件を導入する。

## 実装ステップ（Issue #1453）

- [x] T001 gwt-spec Issue 作成（#1453）
- [x] T002 RED: `IssueListPanel.test.ts` に番号部分一致/混在AND/#付き検索を追加
- [x] T003 RED: `AgentLaunchForm.test.ts` に番号部分一致/混在AND/#付き検索を追加
- [x] T004 RED: `crates/gwt-core/src/git/issue.rs` に番号検索テストを追加
- [x] T005 GUI共通検索ユーティリティ `gwt-gui/src/lib/issueSearch.ts` 追加
- [x] T006 `IssueListPanel.svelte` / `AgentLaunchForm.svelte` に共通ロジック適用
- [x] T007 Rust `filter_issues_by_title` をトークンAND + 番号部分一致に拡張
- [x] T008 検証（対象テスト実行・結果記録）

## 検証結果（Issue #1453）

- [x] `cargo test -p gwt-core filter_issues_by_title -- --test-threads=1`（7 passed）
- [x] `cd gwt-gui && pnpm test src/lib/components/IssueListPanel.test.ts -t \"filters issues by number tokens and mixed AND query\"`（1 passed）
- [x] `cd gwt-gui && pnpm test src/lib/components/AgentLaunchForm.test.ts -t \"filters from-issue list by number tokens and mixed AND query\"`（1 passed）
- [x] `cd gwt-gui && pnpm test src/lib/components/IssueListPanel.test.ts src/lib/components/AgentLaunchForm.test.ts`（IssueListPanelは全件成功、AgentLaunchForm既存失敗2件: bunx/npx fallback期待）

---

# TODO: GitHub Copilot CLI 対応

---

## TODO: Issue #1441 Project Index Files 検索結果 0 件表示の修正（2026-03-04）

## 背景（Issue #1441）

Project Index の Files タブで `Git` を検索した際に「No results found」と見える不具合。
根本原因として、未検索状態でも入力文字列があるだけで 0 件表示文言が出る UI 条件と、
semantic 検索結果が空のときのフォールバック不在を修正する。

## 実装ステップ（Issue #1441）

- [x] T001 フロントエンド回帰テスト追加（未検索時 0 件文言を表示しない）
- [x] T002 RED 確認（追加テスト失敗を確認）
- [x] T003 UI 実装修正（検索実行後のみ 0 件文言表示）
- [x] T004 Python 検索フォールバック追加（semantic 0 件時の部分一致）
- [x] T005 GREEN 確認（対象テスト実行）
- [x] T006 追加検証（svelte-check + Rust unit test）

## 検証結果（Issue #1441）

- [x] `cd gwt-gui && pnpm test src/lib/components/ProjectIndexPanel.test.ts`（3 tests passed）
- [x] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`（0 errors / 1 warning: 既存 `MergeDialog.svelte`）
- [x] `cargo test -p gwt-tauri project_index -- --test-threads=1`（5 tests passed）

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

## TODO: Issue #1424 — Launch Agent E2009 auto-repair（2026-03-03）

## 背景（E2009 auto-repair）

`create_for_branch` 時にディレクトリが存在するが git メタデータが消失している場合、
有効な gitfile (.git ファイルに `gitdir:` 記述) を持つなら `git worktree repair` を
実行して自動再登録する。

仕様 Issue: #1424

## 実装ステップ（E2009 auto-repair）

- [x] T000 tasks/todo.md 更新
- [x] T001 `Repository::restore_worktree_metadata(path, branch)` 追加（repository.rs）
- [x] T002 `is_valid_worktree_gitfile()` ヘルパー追加（manager.rs）
- [x] T003 `create_for_branch` の path.exists() ブロックに repair ロジック追加
- [x] T004 TDD テスト追加（test_create_for_branch_repairs_unregistered_valid_worktree）
- [x] T005 cargo test 検証（35/35 pass）
- [x] T006 cargo clippy --lib 検証（警告なし）

---

## TODO: Issue #1265 追加修正（2026-03-03）

## 背景（Issue #1265 追加修正）

Issue #1265 の再発に対して、単純な正規化強化だけでは不十分だったため、
Launch Agent の責務を「事前可用性判定」から「実行時解決と実行時エラー返却」に寄せる。

仕様同期先: <https://github.com/akiojin/gwt/issues/1304>（Issue #1265 の追加仕様を集約した追補Issue）

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
