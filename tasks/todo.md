# TODO: プロジェクト単位の完全分離（PTY・ChromaDB・GitHub Issue）

## 背景

gwt で複数プロジェクト同時利用時に PTY 通信・ChromaDB 検索・GitHub Issue がプロジェクト境界を越える問題を修正する。

## 実装ステップ

- [x] T001 gwt-spec Issue 作成 (#1395)
- [x] T002 `TerminalPane`/`PaneConfig` に `project_root` フィールド追加
- [x] T003 `PaneManager` に `panes_for_project()` メソッド追加
- [x] T004 `list_terminals` にプロジェクトフィルタ適用
- [x] T005 `send_keys_to_pane`/`capture_scrollback_tail` にプロジェクト検証追加
- [x] T006 MCP ハンドラにプロジェクトフィルタ追加
- [x] T007 `gwt-project-index` SKILL.md 更新
- [x] T008 `gwt-issue-spec-ops` SKILL.md 更新
- [x] T009 `gwt-spec-to-issue-migration` SKILL.md 更新
- [x] T010 `gwt-pty-communication` SKILL.md 更新
- [x] T011 `.codex/skills/gwt-spec-to-issue-migration/` 削除
- [x] T012 `cargo test` + `cargo clippy` + markdownlint 検証

## 検証結果

- [x] `cargo test` — 534 tests passed (gwt-tauri) + 4 tests passed (voice_eval)
- [x] `cargo clippy --all-targets --all-features -- -D warnings` — 警告なし
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

## TODO: Windows タブ切り替え時のフリッカー修正

## 背景

Windows 環境でタブ切り替え時に `$derived`（同期）と `$effect`（非同期）のタイミング差で 1 フレーム全ターミナル非表示のギャップが生じ、背景フラッシュが発生する。`isTerminalTabVisible()` が `visibleTerminalTabId`（`$effect` で非同期更新）に依存していることが根本原因。

## 実装ステップ

- [x] T001 gwt-spec Issue 作成 (#1410)
- [x] T002 TDD テスト追加（jsdom では $effect フラッシュ済みのため GREEN だが仕様テストとして有効）
- [x] T003 `isTerminalTabVisible()` 修正（GREEN 化）
- [x] T004 テスト GREEN 確認（33/33 pass）
- [x] T005 型チェック・lint 確認（svelte-check 0 errors）
- [x] T006 コミット＆プッシュ（e637213f）

## 検証結果

- [x] `pnpm test src/lib/components/MainArea.test.ts` — 33 tests passed
- [x] `npx svelte-check --tsconfig ./tsconfig.json` — 0 errors, 1 warning（既存・変更対象外）
- [x] `bunx commitlint --from HEAD~1 --to HEAD` — 通過
