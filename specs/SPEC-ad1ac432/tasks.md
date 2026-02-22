# タスクリスト: Cleanup Remote Branches

## 依存関係

- US1（リモート同時削除）← US2（PR 可視化・安全性判定）: PR 状態は安全性に影響するが、リモート削除自体は PR 状態がなくても動作可能
- US3（単一削除モーダル統合）← US1: モーダル経由統合は先にリモート削除機能が動作している必要あり
- US4（gone 区別）は US1 と並列可能
- US5（部分失敗）は US1 の一部として実装
- US6（gh 未対応）は US1 の一部として実装
- US7（Force cleanup モード）は US1/US5 完了後に実装（結果表示・ガードテスト追加）

## 既存コードベース情報

- `crates/gwt-core/src/git/gh_cli.rs`: `resolve_gh_path()`, `gh_command()`, `is_gh_available()` が既存。ここに認証チェック・削除機能を追加する
- `crates/gwt-core/src/git/pullrequest.rs`: `PrCache`, `PrStatusCache`, `PrStatusInfo`, `PullRequest` が既存。PR 状態取得のクリーンアップ用簡易版はここか `gh_cli.rs` に追加する
- `crates/gwt-core/src/git.rs`: モジュール登録ファイル（`mod gh_cli;` は登録済みだが非 pub）
- `crates/gwt-core/src/git/issue.rs`: `is_gh_cli_authenticated()`, `is_gh_cli_available()` が既存
- `gwt-gui/src/lib/types.ts`: `WorktreeInfo`, `CleanupResult`, `CleanupProgress`, `GhCliStatus` が既存

## Phase 1: セットアップ

- [x] T001 [P] [US1] `PrStatus` enum を `gh_cli.rs` に定義（Merged / Open / Closed / None / Unknown）し `git.rs` に pub re-export を追加 `crates/gwt-core/src/git/gh_cli.rs`
- [x] T002 [P] [US1] `gh_cli.rs` に `check_auth() -> bool` のスケルトンと `delete_remote_branch()` / `get_pr_statuses()` のシグネチャを追加 `crates/gwt-core/src/git/gh_cli.rs`

## Phase 2: 基盤 — gh CLI 連携（gwt-core）

- [x] T003 [US1,US6] テスト: `check_auth` の認証済み/未認証/未インストール/タイムアウトの 4 パターン `crates/gwt-core/src/git/gh_cli.rs`
- [x] T004 [US1,US6] `check_auth() -> bool` を実装（`gh auth status` 実行、タイムアウト 5 秒） `crates/gwt-core/src/git/gh_cli.rs`
- [x] T005 [US1] テスト: `delete_remote_branch` の成功/ブランチ不在/権限不足/タイムアウト `crates/gwt-core/src/git/gh_cli.rs`
- [x] T006 [US1] `delete_remote_branch(repo_path, branch) -> Result<()>` を実装（`gh api -X DELETE` 使用、タイムアウト 10 秒） `crates/gwt-core/src/git/gh_cli.rs`
- [x] T007 [US2] テスト: `get_pr_statuses` の Merged/Open/Closed/None/複数PR/gh失敗 `crates/gwt-core/src/git/gh_cli.rs`
- [x] T008 [US2] `get_pr_statuses(repo_path) -> HashMap<String, PrStatus>` を実装（`gh pr list --state all --json` 使用、limit 200） `crates/gwt-core/src/git/gh_cli.rs`

## Phase 3: 基盤 — Tauri コマンド拡張（gwt-tauri）

- [x] T009 [US6] `AppState` に `gh_available: AtomicBool` を追加し、起動時に `check_auth()` でセット `crates/gwt-tauri/src/state.rs`
- [x] T010 [P] [US6] テスト + 実装: `check_gh_available` コマンド（AppState の `gh_available` を返す） `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T011 [P] [US2] テスト + 実装: `get_cleanup_pr_statuses` コマンド（`get_pr_statuses` をラップして HashMap<String, String> を返す） `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T012 [US1] `CleanupResult` に `remote_success: Option<bool>` / `remote_error: Option<String>` を追加 + シリアライズテスト `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T013 [US1,US5] テスト: `cleanup_worktrees` の `delete_remote=true/false`、gone スキップ、部分失敗 `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T014 [US1,US5] `cleanup_worktrees` に `delete_remote` パラメータ追加 + リモート削除ロジック実装 `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T015 [US1] `cleanup-progress` イベントに `remote_status` フィールドを追加 `crates/gwt-tauri/src/commands/cleanup.rs`

## Phase 4: 基盤 — 統合安全性判定 + プロジェクト設定

- [x] T016 [US2] テスト: `compute_safety_level` 拡張の全組み合わせ（Safe+Merged/Closed/Open/None, Warning+Merged, Danger+Open, Disabled, toggle OFF） `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T017 [US2] `compute_safety_level` に `delete_remote` / `pr_status` パラメータ追加 + 統合判定ロジック `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T018 [US1] テスト + 実装: プロジェクト設定の永続化（`delete_remote_branches` トグル状態） `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T019 [US1] テスト + 実装: `get_cleanup_settings` / `set_cleanup_settings` コマンド `crates/gwt-tauri/src/commands/cleanup.rs`

## Phase 5: ストーリー 1+6 — トグル UI + gh 未対応

- [x] T020 [US6] テスト: gh 利用可能時にトグル表示 / gh 利用不可時にトグル非表示 `gwt-gui/src/lib/components/CleanupModal.test.ts`
- [x] T021 [US1,US6] CleanupModal に「Also delete remote branches」トグルを追加（gh 不可時は非表示） `gwt-gui/src/lib/components/CleanupModal.svelte`
- [x] T022 [US1] テスト: トグル ON/OFF による安全性ドット色の切り替え `gwt-gui/src/lib/components/CleanupModal.test.ts`
- [x] T023 [US1] トグル状態のプロジェクト設定連携（読み込み/保存） `gwt-gui/src/lib/components/CleanupModal.svelte`
- [x] T024 [US1] トグル ON/OFF による安全性レベルのリアクティブ再計算 `gwt-gui/src/lib/components/CleanupModal.svelte`

## Phase 6: ストーリー 2 — PR バッジ + 安全性統合

- [x] T025 [US2] テスト: PR Merged/Closed/Open/None バッジ表示 + 取得中スピナー + gh 不可時非表示 `gwt-gui/src/lib/components/CleanupModal.test.ts`
- [x] T026 [US2] PR バッジ UI を各ブランチ行に追加（Merged/Closed=緑, Open=オレンジ, None=非表示） `gwt-gui/src/lib/components/CleanupModal.svelte`
- [x] T027 [US2] モーダル onMount で PR 状態を非同期取得 + スピナー表示 + 安全性更新 `gwt-gui/src/lib/components/CleanupModal.svelte`

## Phase 7: ストーリー 3 — 単一ブランチ削除のモーダル統合

- [x] T028 [US3] テスト:「Cleanup this branch」がモーダルを開きプリセレクトされること `gwt-gui/src/lib/components/Sidebar.test.ts`
- [x] T029 [US3] 「Cleanup this branch」の動作を CleanupModal 起動（プリセレクト）に変更 `gwt-gui/src/lib/components/Sidebar.svelte`
- [x] T030 [US3] `cleanup_single_worktree` の invoke 呼び出しをフロントエンドから削除 `gwt-gui/src/lib/components/Sidebar.svelte`

## Phase 8: ストーリー 4 — gone バッジ強調

- [x] T031 [P] [US4] テスト: トグル ON+gone → 強調表示 / トグル OFF+gone → 通常表示 `gwt-gui/src/lib/components/CleanupModal.test.ts`
- [x] T032 [US4] gone バッジの強調表示（トグル ON 時に「Remote already deleted」を明示） `gwt-gui/src/lib/components/CleanupModal.svelte`

## Phase 9: ストーリー 5 — 結果ダイアログ + 確認ダイアログ

- [x] T033 [US5] テスト: ローカル+リモート成功/リモート失敗/トグル OFF 時の結果表示 `gwt-gui/src/lib/components/CleanupModal.test.ts`
- [x] T034 [US5] 結果ダイアログの拡張（Local: ✓/✗ + Remote: ✓/✗ 全件表示） `gwt-gui/src/lib/components/CleanupModal.svelte`
- [x] T035 [US1] unsafe 確認ダイアログにリモート削除警告テキストを追加（トグル ON 時のみ） `gwt-gui/src/lib/components/CleanupModal.svelte`

## Phase 10: SPEC-c4e8f210 更新 + 仕上げ

- [x] T036 [P] SPEC-c4e8f210 の FR-508/FR-512/エッジケース/範囲外に上書き注記を追加 `specs/SPEC-c4e8f210/spec.md`
- [x] T037 `cargo test` で全バックエンドテスト通過 `(ルート)`
- [x] T038 `cargo clippy --all-targets --all-features -- -D warnings` で警告なし `(ルート)`
- [x] T039 `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` でエラーなし `gwt-gui/`
- [x] T040 `cd gwt-gui && pnpm test` でフロントエンドテスト通過（既存の4件の失敗は pre-existing） `gwt-gui/`
- [x] T041 [P] `cleanup_single_worktree` コマンドのバックエンド側を deprecated マーク `crates/gwt-tauri/src/commands/cleanup.rs`

## Phase 11: ストーリー 7 — Force cleanup モード（unsafe限定）

- [x] T042 [US7] テスト: Force toggle 表示（初期 OFF）と safe のみ選択時に `force=false` が渡ること `gwt-gui/src/lib/components/CleanupModal.test.ts`
- [x] T043 [US7] テスト: Force toggle ON でも disabled 行が選択不可のままであること `gwt-gui/src/lib/components/CleanupModal.test.ts`
- [x] T044 [US7] テスト: unsafe cleanup 実行後の結果ダイアログに force 注記が表示されること `gwt-gui/src/lib/components/CleanupModal.test.ts`
- [x] T045 [US7] 実装: CleanupModal に `Force cleanup` トグルと結果注記を追加 `gwt-gui/src/lib/components/CleanupModal.svelte`
- [x] T046 [US7] テスト: `cleanup_single_branch` は force=true でも protected/current/agent-running を拒否すること `crates/gwt-tauri/src/commands/cleanup.rs`
- [x] T047 [US7] 仕様同期: `SPEC-ad1ac432` の spec/plan/tasks/tdd を Force cleanup 要件で更新 `specs/SPEC-ad1ac432/spec.md`
