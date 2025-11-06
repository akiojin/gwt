# クイックスタート: main/develop保護強化

**仕様ID**: `SPEC-4e948b89` | **日付**: 2025-11-06

## 1. テスト駆動の進め方

```bash
bun install
bun test tests/unit/worktree.test.ts
```

1. `tests/unit/worktree.test.ts` に保護ブランチ拒否の失敗テストを追加
2. `src/worktree.ts` を更新してテストを緑化
3. UI関連テスト (`src/ui/screens/__tests__/BranchActionSelectorScreen.test.tsx` / `tests/ui/__tests__/integration/navigation.test.tsx`) の失敗を確認
4. `src/ui/components/App.tsx` 等を更新してUIテストを通過させる

## 2. 手動確認

```bash
bun run build
bunx .
```

1. ブランチ一覧から `main` を選択 → AIツール選択画面へ直接遷移し、「新規ブランチ作成」が表示されない
2. 再度一覧に戻り `feature/...` を選択 → 従来どおりアクション選択画面が表示される
3. CLI上に「ルートブランチはWorktree化できません」等の案内が出ることを確認

## 3. 運用メモ

- `git worktree list` で `develop` Worktree が存在しない状態を維持する
- ルートWorktreeのチェックアウトは `git checkout develop` で行い、保護ブランチを別ディレクトリに追加しない
