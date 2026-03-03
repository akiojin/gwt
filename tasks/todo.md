# fix: Cleanup — 保護ブランチの事前表示とエラーハンドリング (#1404)

## 背景

Cleanup処理でリポジトリルールにより保護されたブランチの削除が HTTP 422 でハードエラーになる問題の修正。
事前表示 + エラーハンドリングの2段階で対応。

## 実装ステップ

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

## 検証結果

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

## 背景

Issue #1265 の再発に対して、単純な正規化強化だけでは不十分だったため、
Launch Agent の責務を「事前可用性判定」から「実行時解決と実行時エラー返却」に寄せる。

仕様同期先: https://github.com/akiojin/gwt/issues/1304

## 実装ステップ

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
