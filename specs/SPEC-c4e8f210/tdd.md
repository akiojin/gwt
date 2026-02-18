# TDD テスト仕様: Worktree Cleanup（GUI）

## バックエンドテスト（Rust）

実装ファイル: `crates/gwt-tauri/src/commands/cleanup.rs`

### T1: 安全性判定テスト（`compute_safety_level`）

| テスト名 | 状態 | 内容 |
|---|---|---|
| `safe_when_no_changes_and_no_unpushed` | 実装済 | changes=false, unpushed=false → Safe |
| `warning_when_unpushed_only` | 実装済 | changes=false, unpushed=true → Warning |
| `warning_when_changes_only` | 実装済 | changes=true, unpushed=false → Warning |
| `danger_when_both_changes_and_unpushed` | 実装済 | changes=true, unpushed=true → Danger |
| `disabled_when_protected` | 実装済 | is_protected=true → Disabled |
| `disabled_when_current` | 実装済 | is_current=true → Disabled |
| `disabled_when_agent_running` | 実装済 | is_agent_running=true → Disabled |
| `safety_level_sort_order` | 実装済 | Safe < Warning < Danger < Disabled |
| `protected_branch_detected` | 実装済 | main/master/develop/release が protected 判定 |

### T2/T3: クリーンアップガード・実行テスト（`cleanup_single_branch`）

| テスト名 | 状態 | 内容 |
|---|---|---|
| `rejects_protected_branch` | 実装済 | protected branch の削除拒否 |
| `rejects_current_worktree` | 実装済 | current worktree の削除拒否 |
| `rejects_agent_running_branch` | 実装済 | agent 稼働中ブランチの削除拒否 |
| `successful_cleanup_removes_worktree_and_branch` | 実装済 | worktree + ローカルブランチ削除成功 |
| `skips_failure_and_continues_in_batch` | 実装済 | 失敗スキップ + 残り継続 |
| `force_deletes_unsafe_worktree` | 実装済 | force=true で unsafe ブランチ削除成功 |

### イベント関連テスト（統合テスト対象）

以下はTauriの `AppHandle` を必要とするため、ユニットテストではなく手動/E2Eテストで検証する。

| テスト名 | 状態 | 内容 |
|---|---|---|
| `emits_progress_events_for_each_branch` | 統合テスト | 各ブランチに cleanup-progress イベント emit |
| `emits_worktrees_changed_on_completion` | 統合テスト | 完了時に worktrees-changed イベント emit |
| `does_not_delete_remote_branch` | 統合テスト | リモートブランチ非削除（remote setup 必要） |

## フロントエンドテスト（TypeScript / Svelte）

フロントエンドは `vitest` + `@testing-library/svelte` により主要なUIロジックをユニットテストする。
加えて `pnpm check`（svelte-check）で型/静的検証する。

### Sidebar 安全性インジケーター

| テストケース | 検証方法 | 内容 |
|---|---|---|
| 緑ドット表示（safe） | svelte-check + 手動 | changes=false, unpushed=false → 緑 CSS ドット |
| 黄ドット表示（warning） | svelte-check + 手動 | changes XOR unpushed → 黄 CSS ドット |
| 赤ドット表示（danger） | svelte-check + 手動 | changes=true, unpushed=true → 赤 CSS ドット |
| グレードット表示（protected/current） | svelte-check + 手動 | is_protected=true → グレー CSS ドット |
| 削除中スピナー表示 | svelte-check + 手動 | deleting 状態 → スピナー表示、ドット非表示 |
| 削除中クリック無効 | svelte-check + 手動 | deleting 状態 → pointer-events: none |
| agent tab 表示（indicator） | vitest | agent tab が開いているブランチ名の前にグラフィカルなアクティビティインジケーター（アニメーションする3本バー）を表示 |

### Cleanup モーダル

| テストケース | 検証方法 | 内容 |
|---|---|---|
| 安全性順ソート | svelte-check + 手動 | safe → warning → danger → disabled |
| フル情報表示 | svelte-check + 手動 | 各行に全フィールド表示 |
| protected チェック不可 | svelte-check + 手動 | checkbox disabled |
| current チェック不可 | svelte-check + 手動 | checkbox disabled |
| agent-running チェック不可 | svelte-check + 手動 | checkbox disabled |
| Select All Safe | svelte-check + 手動 | safe のみチェック |
| safe のみ選択時は確認なし | svelte-check + 手動 | 確認ダイアログなしで即実行 |
| unsafe 選択時は確認あり | svelte-check + 手動 | 確認ダイアログ表示 |
| モーダル即閉じ | svelte-check + 手動 | Cleanup 実行後モーダル閉じ |
| 失敗時モーダル再表示 | svelte-check + 手動 | cleanup-completed でエラーあり → 再表示 |
| agent tab 表示（indicator） | vitest | agent tab が開いている Worktree のブランチ名の前にグラフィカルなアクティビティインジケーター（アニメーションする3本バー）を表示 |
| agent tab を先頭にソート | vitest | スピナー付き Worktree が先頭に来る（disabled を除く） |
| agent tab 選択時の確認 | vitest | agent tab が選択に含まれる場合に追加確認が表示される |

### コンテキストメニュー

| テストケース | 検証方法 | 内容 |
|---|---|---|
| 2項目表示 | svelte-check + 手動 | "Cleanup this branch" + "Cleanup Worktrees..." |
| 単体削除は常に確認 | svelte-check + 手動 | 確認ダイアログ必ず表示 |
| プリセレクト付きモーダル | svelte-check + 手動 | Cleanup Worktrees... で対象ブランチ選択済み |
| protected は単体削除不可 | svelte-check + 手動 | "Cleanup this branch" disabled |

### キーボードショートカット

| テストケース | 検証方法 | 内容 |
|---|---|---|
| Cmd+Shift+K でモーダル起動 | 手動 | Cleanup モーダルが開く |

## テスト実行コマンド

```bash
# バックエンドテスト
cargo test

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# フロントエンドテスト
cd gwt-gui && pnpm test

# フロントエンドチェック（型/静的）
cd gwt-gui && pnpm check
```

## テストカバレッジサマリー

| カテゴリ | 自動テスト | 型チェック | 手動検証 |
|---|---|---|---|
| 安全性判定 | 9/9 | - | - |
| ガード/削除 | 6/6 | - | - |
| イベント | - | - | 3/3 |
| Sidebar UI | - | 6/6 | 6/6 |
| モーダル UI | - | 10/10 | 10/10 |
| コンテキストメニュー | - | 4/4 | 4/4 |
| ショートカット | - | - | 1/1 |
