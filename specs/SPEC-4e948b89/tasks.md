# タスク: main/develop保護強化

**入力**: `/specs/SPEC-4e948b89/` の文書
**前提条件**: spec.md（✅）、plan.md（✅）、research.md（✅）、data-model.md（✅）、quickstart.md（✅）

## 実装タスク

- [x] T101 [P] [US2] `tests/unit/worktree.test.ts` に保護ブランチ拒否の失敗テストを追加（main/develop/master）
- [x] T102 [P] [US1] `src/ui/screens/__tests__/BranchActionSelectorScreen.test.tsx` に保護ブランチで「Create new branch」が表示されないテストを追加
- [x] T103 [ ] [US1] `tests/ui/__tests__/integration/navigation.test.tsx` に保護ブランチ選択時の遷移テストを追加（Codex 2025-11-07 完了）
- [x] T104 [ ] [US2] `src/worktree.ts` の `createWorktree` に保護ブランチチェックを実装
- [x] T105 [ ] [US1] `src/ui/components/App.tsx` と `BranchActionSelectorScreen.tsx` を更新し、遷移/表示ロジックを修正（警告メッセージ含む）※Codex 2025-11-06 完了
- [x] T106 [ ] [US1] 追加したUIメッセージのスナップショット/表示検証を調整し、関連テストを更新 ※Codex 2025-11-06 完了
- [x] T107 [ ] [US1/US2] `bun test` 全体を実行して回帰確認（Codex 2025-11-06/11-07 実施）

> 並列属性 `[P]` はテスト追加タスク間で並行可能であることを示す。
