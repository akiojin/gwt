# タスク一覧: Worktree一覧のエージェント状態アニメーション

**仕様ID**: `SPEC-b80e7996`
**作成日**: 2026-02-16

## Phase 1: データモデル拡張

- [x] T-001: BranchInfo / WorktreeInfo に `agent_status` フィールドを追加（types.ts）
- [x] T-002: Rust バックエンド list_branches レスポンスに `agent_status` を含める
- [x] T-003: Rust バックエンド list_worktrees レスポンスに `agent_status` を含める
- [x] T-004: ブランチ一覧取得時に `check_idle_timeout` を呼び出し Stopped 自動遷移

## Phase 2: fs 監視

- [x] T-005: `notify` crate を Cargo.toml に追加
- [x] T-006: sessions ディレクトリの fs watcher モジュール実装（debounce 500ms）
- [x] T-007: Tauri setup フックで watcher 起動、`agent-status-changed` イベント emit

## Phase 3: フロントエンド状態同期

- [x] T-008: Sidebar で `agent-status-changed` Tauri イベントをリッスンしブランチ再取得
- [x] T-009: 10秒間隔のポーリングフォールバック実装（エージェントタブ存在時のみ）

## Phase 4: インジケーター UI

- [x] T-010: 全ブランチ行に 12px 固定幅の予約スペースを追加（インデント修正）
- [x] T-011: 2層インジケーター実装（静的ドット + Running 時 pulse アニメーション）
- [x] T-012: CSS keyframes `agent-pulse` 定義、prefers-reduced-motion 対応
- [x] T-013: CleanupModal にも同様の 2 層インジケーター適用

## Phase 5: 状態推測（Hook 非対応エージェント）

- [x] T-014: Codex/Gemini/OpenCode のペイン出力解析による状態推測ロジック実装
- [x] T-015: list_branches/list_worktrees で推測ロジックを適用

## テスト

- [x] T-016: agent_status フィールドの Rust ユニットテスト
- [x] T-017: fs watcher の統合テスト
- [x] T-018: フロントエンド Sidebar インジケーターのコンポーネントテスト
