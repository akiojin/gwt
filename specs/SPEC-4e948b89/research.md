# 調査ノート: main/develop保護強化

## 現行実装の確認

- `src/worktree.ts` に `PROTECTED_BRANCHES = ["main", "master", "develop"]` が定義され、クリーンアップ機能では既に利用されている。
- `createWorktree` は現在ブランチ名のチェックを行わず、呼び出し元がそのままGitコマンドを実行してしまう。
- Ink UI (`src/ui/components/App.tsx`) の `SelectedBranchState` にはブランチ分類が保持されておらず、保護判定を行う手段が存在しない。
- `BranchActionSelectorScreen` は常に「Use existing」「Create new branch」の2択を表示するため、保護ブランチでも誤操作が可能。

## テストの状況

- `tests/unit/worktree.test.ts` に`createWorktree`の正常系/例外系テストが揃っているが、保護用テストは無い。
- `src/ui/screens/__tests__/BranchActionSelectorScreen.test.tsx` で表示オプションのスナップショットを確認している。
- `tests/ui/__tests__/integration/navigation.test.tsx` が分岐遷移の回帰テストを担っている。

## 想定される影響範囲

- `SelectedBranchState`を利用する箇所（`App.shortcuts`テスト、`ExecutionModeSelector`など）で型更新が必要。
- UIのフッターメッセージ機構（`cleanupFooterMessage`）を流用する場合は状態リセットに注意。

## 追加メモ

- ルートリポジトリ側では既に`develop`がWorktreeとして存在していたため、運用上も保護が求められている。
- 既存のCLIヘルプなどに特別な説明は不要（メッセージ追加のみ）。
