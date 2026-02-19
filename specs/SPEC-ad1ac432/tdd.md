# TDD テスト仕様: Cleanup Remote Branches

## バックエンドテスト（Rust）

### gh CLI 認証チェック（`check_auth`）

実装ファイル: `crates/gwt-core/src/git/gh_cli.rs`

| テスト名 | 状態 | 内容 |
|---|---|---|
| `check_auth_returns_bool` | 実装済 | `check_auth()` が bool を返すこと（環境依存） |
| `gh_auth_command_structure` | 実装済 | `gh auth status` のコマンド構造が正しいこと |

### リモートブランチ削除（`delete_remote_branch`）

実装ファイル: `crates/gwt-core/src/git/gh_cli.rs`

| テスト名 | 状態 | 内容 |
|---|---|---|
| `delete_remote_branch_returns_result` | 実装済 | 関数シグネチャの型テスト |
| `resolve_owner_repo_function_exists` | 実装済 | owner/repo 解決関数の構造テスト |

### PR 状態取得（`get_pr_statuses`）

実装ファイル: `crates/gwt-core/src/git/gh_cli.rs`

| テスト名 | 状態 | 内容 |
|---|---|---|
| `pr_status_merged` | 実装済 | マージ済み PR → `PrStatus::Merged` |
| `pr_status_open` | 実装済 | Open な PR → `PrStatus::Open` |
| `pr_status_closed` | 実装済 | マージせずクローズ → `PrStatus::Closed` |
| `pr_status_none_for_unknown_branch` | 実装済 | PR なし → map に含まれない |
| `pr_status_multiple_prs_uses_latest` | 実装済 | 同一ブランチに複数 PR → 最新を採用 |
| `pr_status_gh_failure_returns_empty` | 実装済 | 不正 JSON → 空 map |
| `pr_status_empty_array` | 実装済 | 空配列 → 空 map |
| `gh_state_to_pr_status_variants` | 実装済 | 全状態変換テスト |

### PrStatus enum テスト

実装ファイル: `crates/gwt-core/src/git/gh_cli.rs`

| テスト名 | 状態 | 内容 |
|---|---|---|
| `pr_status_serializes_to_lowercase` | 実装済 | 全バリアントの lowercase シリアライズ |
| `pr_status_deserializes_from_lowercase` | 実装済 | lowercase からのデシリアライズ |
| `pr_status_roundtrip` | 実装済 | シリアライズ→デシリアライズの往復 |

### 統合安全性判定（`compute_safety_level` 拡張）

実装ファイル: `crates/gwt-tauri/src/commands/cleanup.rs`

| テスト名 | 状態 | 内容 |
|---|---|---|
| `integrated_safe_with_merged_pr` | 実装済 | delete_remote=true, ローカルSafe, PR Merged → Safe |
| `integrated_safe_with_closed_pr` | 実装済 | delete_remote=true, ローカルSafe, PR Closed → Safe |
| `integrated_safe_with_open_pr_downgrades_to_warning` | 実装済 | delete_remote=true, ローカルSafe, PR Open → Warning |
| `integrated_safe_with_no_pr_downgrades_to_warning` | 実装済 | delete_remote=true, ローカルSafe, PR None → Warning |
| `integrated_warning_stays_warning_regardless_of_pr` | 実装済 | delete_remote=true, ローカルWarning, PR Merged → Warning |
| `integrated_danger_stays_danger_regardless_of_pr` | 実装済 | delete_remote=true, ローカルDanger, PR Open → Danger |
| `toggle_off_ignores_pr_status` | 実装済 | delete_remote=false → 従来のローカルのみ判定 |
| `disabled_stays_disabled_regardless_of_pr` | 実装済 | protected=true, delete_remote=true → Disabled |

### CleanupResult シリアライズ

実装ファイル: `crates/gwt-tauri/src/commands/cleanup.rs`

| テスト名 | 状態 | 内容 |
|---|---|---|
| `cleanup_result_serializes_with_remote_fields` | 実装済 | remote_success / remote_error が JSON に含まれる |
| `cleanup_result_remote_none_when_toggle_off` | 実装済 | delete_remote=false → remote_success=None |
| `cleanup_result_remote_failure` | 実装済 | remote 失敗時の JSON 出力 |

### CleanupProgressPayload

実装ファイル: `crates/gwt-tauri/src/commands/cleanup.rs`

| テスト名 | 状態 | 内容 |
|---|---|---|
| `progress_event_includes_remote_status` | 実装済 | remote_status がセットされている |
| `progress_event_remote_status_none` | 実装済 | remote_status が None の場合 null |

### CleanupSettings 永続化

実装ファイル: `crates/gwt-tauri/src/commands/cleanup.rs`

| テスト名 | 状態 | 内容 |
|---|---|---|
| `cleanup_settings_default` | 実装済 | デフォルトは delete_remote_branches=false |
| `cleanup_settings_serialization_roundtrip` | 実装済 | シリアライズ→デシリアライズの往復 |
| `load_cleanup_settings_returns_default_when_missing` | 実装済 | ファイルなし → デフォルト値 |
| `save_and_load_cleanup_settings` | 実装済 | 保存→読み込みの往復 |
| `save_cleanup_settings_creates_gwt_dir` | 実装済 | .gwt ディレクトリを自動作成 |

## フロントエンドテスト（TypeScript / Svelte）

### CleanupModal — トグル

| テストケース | 検証方法 | 内容 |
|---|---|---|
| gh 利用可能時にトグル表示 | vitest | gh_available=true → トグル DOM 存在 |
| gh 利用不可時にトグル非表示 | vitest | gh_available=false → トグル DOM なし |
| トグル ON/OFF で安全性再計算 | vitest | トグル切り替え → 安全性ドット色が変化 |
| トグル状態の設定保存 | vitest | トグル変更 → set_cleanup_settings 呼び出し |

### CleanupModal — PR バッジ

| テストケース | 検証方法 | 内容 |
|---|---|---|
| PR Merged バッジ（緑） | vitest | pr_status=Merged → 緑バッジ表示 |
| PR Closed バッジ（緑） | vitest | pr_status=Closed → 緑バッジ表示 |
| PR Open バッジ（オレンジ） | vitest | pr_status=Open → オレンジバッジ表示 |
| PR None でバッジ非表示 | vitest | pr_status=None → バッジなし |
| PR 取得中のスピナー | vitest | 取得中 → スピナー表示 |
| gh 不可時はバッジ非表示 | vitest | gh_available=false → バッジなし |

### CleanupModal — gone バッジ強調

| テストケース | 検証方法 | 内容 |
|---|---|---|
| トグル ON + gone → 強調表示 | vitest | gone バッジに強調スタイル適用 |
| トグル OFF + gone → 通常表示 | vitest | gone バッジは従来通り |

### CleanupModal — 結果ダイアログ

| テストケース | 検証方法 | 内容 |
|---|---|---|
| ローカル+リモート成功の全件表示 | vitest | 各ブランチに Local: ✓ / Remote: ✓ |
| リモート失敗の表示 | vitest | Remote: ✗ (error) 表示 |
| トグル OFF 時はリモート列なし | vitest | Local: ✓ のみ表示 |

### コンテキストメニュー統合

| テストケース | 検証方法 | 内容 |
|---|---|---|
| 「Cleanup this branch」がモーダルを開く | vitest | CleanupModal が表示される |
| プリセレクト状態 | vitest | 該当ブランチにチェック |
| cleanup_single_worktree の呼び出しなし | vitest | invoke('cleanup_single_worktree') が呼ばれない |

### 確認ダイアログ

| テストケース | 検証方法 | 内容 |
|---|---|---|
| トグル ON 時にリモート警告テキスト表示 | vitest | 「Remote branches will also be deleted」 |
| トグル OFF 時はリモート警告なし | vitest | リモート警告テキストなし |

## テスト実行コマンド

```bash
# バックエンドテスト
cargo test

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# フロントエンドテスト
cd gwt-gui && pnpm test

# フロントエンドチェック（型/静的）
cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json
```

## テストカバレッジサマリー

| カテゴリ | 自動テスト | 型チェック | 手動検証 |
|---|---|---|---|
| gh CLI チェック | 2 | - | - |
| リモート削除 | 2 | - | - |
| PR 状態取得 | 8 | - | - |
| PrStatus enum | 3 | - | - |
| 統合安全性 | 8 | - | - |
| CleanupResult 拡張 | 3 | - | - |
| CleanupProgressPayload | 2 | - | - |
| CleanupSettings 永続化 | 5 | - | - |
| トグル UI | 4 | 4 | 4 |
| PR バッジ UI | 6 | 6 | 6 |
| gone バッジ強調 | 2 | 2 | 2 |
| 結果ダイアログ | 3 | 3 | 3 |
| コンテキストメニュー統合 | 3 | 3 | 3 |
| 確認ダイアログ | 2 | 2 | 2 |
