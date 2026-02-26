# クイックスタート: AI自動ブランチ命名モード

**仕様ID**: `SPEC-9cd50c7c` | **日付**: 2026-02-26

## 変更の概要

Launch Agentフォームのブランチ命名UIを刷新する。

**Before**: Suggestモーダル（3候補選択）→ 手動でブランチ名入力 → Launch
**After**: セグメンテッドボタンで Direct / AI Suggest を切替 → Launch時にAI自動生成

## 主な変更ファイル

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-core/src/ai/branch_suggest.rs` | プロンプト・パーサーを1つ生成に改修 |
| `crates/gwt-tauri/src/commands/branch_suggest.rs` | Tauriコマンドの戻り値変更 |
| `crates/gwt-tauri/src/commands/terminal.rs` | LaunchAgentRequestにAI説明追加、createステップにAI生成統合 |
| `gwt-gui/src/lib/types.ts` | BranchSuggestResult、LaunchAgentRequest型更新 |
| `gwt-gui/src/lib/agentLaunchDefaults.ts` | branchNamingModeの永続化追加 |
| `gwt-gui/src/lib/components/AgentLaunchForm.svelte` | Suggestモーダル削除、セグメンテッドボタン追加、フォールバック実装 |

## ビルド・テスト

```bash
# バックエンド
cargo test
cargo clippy --all-targets --all-features -- -D warnings

# フロントエンド
cd gwt-gui && pnpm install
cd gwt-gui && pnpm test
cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json
```
