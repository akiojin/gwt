> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

# 機能仕様: Worktree一覧のエージェント状態アニメーション

**仕様ID**: `SPEC-b80e7996`
**作成日**: 2026-02-16
**ステータス**: ドラフト
**カテゴリ**: GUI / Agent Status

**依存仕様**:

- SPEC-861d8cdf（エージェント状態の可視化 — archive/TUI版）
- SPEC-c4e8f210（Worktree Cleanup GUI — FR-515/FR-516 アクティブエージェントインジケーター）
- SPEC-1b98b6d7（Hook実行でGUI増殖しない）

**入力**: ユーザー説明: "Worktree一覧でタブに存在するアクティブエージェントのWorktreeにインジケーターが表示されているが、Claude Codeの場合はHookでLLMが動作中のみアニメーションしてほしい。Codexの場合はHookがないので常時アニメーション。また、アクティブエージェントの場合に一文字分インデントされてしまっている。"
