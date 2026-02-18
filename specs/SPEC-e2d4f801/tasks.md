# タスクリスト: OS環境変数の自動継承

## Phase 1: OS環境変数キャプチャモジュール

- [ ] T1: `gwt-core` に `config/os_env.rs` を新規作成 + `config/mod.rs` にモジュール登録
- [ ] T2: `parse_env_null_separated()` 関数実装 + テスト（TDD）
- [ ] T3: `parse_env_json()` 関数実装（nushell対応）+ テスト（TDD）
- [ ] T4: `ShellType` enum + `detect_shell_type()` + `build_env_capture_command()` 実装 + テスト（TDD）
- [ ] T5: `capture_login_shell_env()` 非同期関数実装 + タイムアウト/フォールバックテスト（TDD）

## Phase 2: アプリ起動時の非同期取得

- [ ] T6: `gwt-tauri` に `OsEnvState` グローバルステート追加
- [ ] T7: Tauriセットアップ時の非同期環境変数取得フック実装
- [ ] T8: エージェント起動時の環境変数取得完了待機ロジック実装

## Phase 3: 三層マージロジック

- [ ] T9: `launch_agent` の環境変数マージを三層構造に変更 + テスト（TDD）
- [ ] T10: `load_profile_env()` を `merge_profile_env()` にリファクタ（disabled_env対応含む）+ テスト（TDD）

## Phase 4: EnvSnapshot廃止・シンプル化

- [ ] T11: `runner.rs` の `EnvSnapshot` を廃止し、OS環境変数ベースの `resolve_command_path` に移行 + テスト更新

## Phase 5: GUI対応

- [ ] T12: エージェント起動時の "Loading environment..." ステータス表示
- [ ] T13: フォールバック時のGUIトースト通知実装
- [ ] T14: メニューに "Debug: Show Captured Environment" 追加 + Tauri command実装
- [ ] T15: 環境変数一覧表示UI実装（ソート済み一覧 + ソース情報）

## 最終確認

- [ ] T16: `cargo test` で全テストパス
- [ ] T17: `cargo clippy` でwarning/errorなし
- [ ] T18: `cd gwt-gui && npx svelte-check` でエラーなし
- [ ] T19: 手動確認（macOS zsh/bash、PATH反映、APIキー反映、Debug メニュー）
