# 実装計画: Cleanup Remote Branches

**仕様ID**: `SPEC-ad1ac432` | **日付**: 2026-02-21 | **仕様書**: `specs/SPEC-ad1ac432/spec.md`

## 目的

- SPEC-c4e8f210 で実装済みの Worktree Cleanup 機能にリモートブランチ削除オプションを追加する
- PR 状態（Merged / Open / Closed / None）を取得・表示し、安全性判定に統合する
- 「Cleanup this branch」コンテキストメニューを CleanupModal 経由に統合する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/`, `crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **外部連携**: `gh` CLI（GitHub CLI）— PR 状態取得 + リモートブランチ削除
- **テスト**: `cargo test`（Rust）/ `vitest`（フロントエンド）/ `svelte-check`（型チェック）
- **前提**: gh CLI がインストール・認証済みの環境でのみリモート機能が有効。未対応環境ではフォールバック（UI 非表示）

## 原則チェック（CLAUDE.md 準拠）

- **シンプルさ**: gh CLI の既存コマンドを活用。独自 GitHub API クライアントは実装しない
- **TDD**: 実装前にテストを書く。安全性判定・リモート削除パスの自動テストを先行
- **ユーザビリティ優先**: トグル 1 つの操作。追加確認ステップなし。gh 未対応時は透過的にフォールバック

## 実装方針

### 追補（2026-02-21）: Force cleanup モード

- CleanupModal に `Force cleanup` トグルを追加し、unsafe 削除時の force 実行をユーザーが明示的に認識できる導線を設ける
- force 適用範囲は unsafe（warning/danger）に限定し、safe のみ選択時は `cleanup_worktrees(force=false)` を維持する
- protected/current/agent-running のガードは force 時も解除しない
- 結果ダイアログに force 実行注記を表示し、実行モードを事後確認可能にする

### Phase 1: バックエンド — gh CLI 連携モジュール（gwt-core）

`crates/gwt-core/src/git/gh.rs` を新規追加:

- `GhCli::check_auth() -> bool`
  - `gh auth status` を実行（タイムアウト 5 秒）
  - exit 0 → true、それ以外（未認証 / 未インストール / タイムアウト）→ false
- `GhCli::delete_remote_branch(repo_path, branch) -> Result<()>`
  - `gh api -X DELETE repos/{owner}/{repo}/git/refs/heads/{branch}` を実行
  - タイムアウト 10 秒
  - owner/repo は `gh repo view --json owner,name` またはリモート URL から解決
- `GhCli::get_pr_statuses(repo_path) -> HashMap<String, PrStatus>`
  - `PrStatus` enum: `Merged`, `Open`, `Closed`, `None`, `Unknown`
  - `gh pr list --state all --json headRefName,state,mergedAt --limit 200` で一括取得
  - ブランチ名をキーにマッチング。同一ブランチに複数 PR がある場合は最新を採用
  - gh 失敗時は全ブランチ `Unknown` を返す

`crates/gwt-core/src/git/mod.rs` に `pub mod gh;` を追加。

### Phase 2: バックエンド — Tauri コマンド拡張（gwt-tauri）

#### AppState 拡張

- `AppState` に `gh_available: bool` を追加
- アプリ起動時（`setup` 内）に `GhCli::check_auth()` を呼んでセット

#### 新規コマンド

- `check_gh_available(state) -> bool`: フロントエンドから gh 利用可否を取得
- `get_pr_statuses(project_path) -> HashMap<String, String>`: PR 状態を取得して返す

#### 既存コマンド変更

- `cleanup_worktrees` に `delete_remote: bool` パラメータを追加
  - `delete_remote=true` 時: 各ブランチのローカル削除後にリモート削除を実行
  - gone ブランチはリモート削除をスキップ
  - `CleanupResult` に `remote_success: Option<bool>`, `remote_error: Option<String>` を追加
  - `cleanup-progress` イベントに `remote_status` フィールドを追加
- `cleanup_single_worktree` コマンドは廃止（FR-612: モーダル統合のため不要）

#### 統合安全性判定の拡張

- `compute_safety_level` に `delete_remote: bool`, `pr_status: Option<PrStatus>` を追加
  - `delete_remote=true` 時:
    - ローカル Safe + PR Merged/Closed → Safe
    - ローカル Safe + PR Open/None → Warning
    - ローカル Warning 以上 → そのまま維持
  - `delete_remote=false` 時: 従来通り

#### プロジェクト設定

- `gwt-core` または `gwt-tauri` にプロジェクト設定の永続化を追加
  - `delete_remote_branches: bool` をプロジェクト設定ファイルに保存
- `get_cleanup_settings` / `set_cleanup_settings` コマンドを追加

### Phase 3: フロントエンド — CleanupModal 拡張

#### トグル UI

- モーダル上部に「Also delete remote branches」スイッチを追加
- gh 利用不可時は非表示（`check_gh_available` の結果で制御）
- トグル状態はプロジェクト設定から読み込み、変更時に保存

#### PR バッジ

- 各ブランチ行に PR 状態バッジを追加
  - 「PR: merged」（緑）、「PR: closed」（緑）、「PR: open」（オレンジ）
  - PR なしの場合はバッジ非表示
  - PR 取得中はスピナー表示
- モーダル `onMount` で `get_pr_statuses` を非同期呼び出し

#### gone バッジ強調

- トグル ON 時、gone バッジのスタイルを変更
- 「Remote already deleted」であることを明示

#### 安全性ドット

- トグル ON/OFF の切り替えで安全性レベルをリアクティブに再計算
- フロントエンド側で `compute_safety_level` 相当のロジックを持ち、トグル変更時に即座にドット色を更新

#### 結果ダイアログ

- `CleanupResult` の表示を拡張
  - 各ブランチ: `branch-name — Local: ✓ / Remote: ✓`
  - リモート失敗時: `branch-name — Local: ✓ / Remote: ✗ (error)`
  - トグル OFF 時: `branch-name — Local: ✓`

#### 確認ダイアログ

- 既存の unsafe 確認ダイアログにリモート削除の警告テキストを追加
- トグル ON 時のみ「Remote branches will also be deleted」を表示

### Phase 4: コンテキストメニュー統合

- 「Cleanup this branch」の動作を変更:
  - 現在: `cleanup_single_worktree` を直接呼び出し
  - 変更後: `CleanupModal` を開き、該当ブランチにプリセレクト
- `cleanup_single_worktree` の invoke 呼び出しをフロントエンドから削除
- バックエンドの `cleanup_single_worktree` コマンドを deprecated または削除

### Phase 5: SPEC-c4e8f210 更新

- FR-508, FR-512, エッジケース, 範囲外に上書き注記を追加

### Phase 6: Force cleanup 仕様反映（SPEC-ad1ac432 追記）

- `spec.md` に US7 / FR-615〜FR-618 / SC-006〜SC-007 を追記
- `CleanupModal.svelte` に `Force cleanup` トグルと結果注記を追加
- `cleanup_single_branch` のガード維持をテストで明示（force=true でも protected/current/agent-running は拒否）

## テスト

### バックエンド（Rust）

- `GhCli::check_auth`: 認証済み/未認証/未インストール/タイムアウトの4パターン
- `GhCli::delete_remote_branch`: 成功/ブランチ不在/権限不足/タイムアウト
- `GhCli::get_pr_statuses`: Merged/Open/Closed/None/複数PR/gh失敗
- `compute_safety_level` 拡張: 統合安全性の全組み合わせ（8パターン）
- `cleanup_worktrees` リモートパス: delete_remote=true/false、gone スキップ、部分失敗
- `CleanupResult` シリアライズ: 新フィールドの JSON 出力
- `cleanup_single_branch` force ガード: force=true でも protected/current/agent-running は削除拒否

### フロントエンド（TypeScript / Svelte）

- トグル表示/非表示（gh 利用可否連動）
- トグル ON/OFF による安全性ドット色の切り替え
- PR バッジの表示（Merged/Closed/Open/None/取得中）
- gone バッジの強調表示
- 結果ダイアログの全件表示
- Force cleanup トグル表示（初期 OFF）
- safe のみ選択時は Force cleanup ON でも `force=false` が渡される
- force 実行時に結果ダイアログへ注記が表示される
- 「Cleanup this branch」がモーダルを開くことの確認
- unsafe 確認ダイアログのリモート警告テキスト
