# タスク: 画面表示時刻のシステムロケール変換

**入力**: `specs/SPEC-12af0c9a/` からの設計ドキュメント  
**前提条件**: `specs/SPEC-12af0c9a/spec.md`、`specs/SPEC-12af0c9a/plan.md`、`specs/SPEC-12af0c9a/research.md`、`specs/SPEC-12af0c9a/data-model.md`

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（US1/US2/US3）
- 説明に正確なファイルパスを含める

## フェーズ1: ユーザーストーリー1 - 全画面の時刻をローカルで理解する (優先度: P1)

**ストーリー**: CLIとWeb UIの表示時刻をシステムロケールへ変換する  
**価値**: UTC表示の誤認を解消し、日常操作の判断精度を上げる

- [ ] **T101** [P] [US1] CLIの時刻表示を監査しローカル変換に統一する: `src/cli/ui/utils/branchFormatter.ts`, `src/logging/formatter.ts`, `src/cli/ui/screens/solid/BranchListScreen.tsx`, `src/cli/ui/screens/solid/LogScreen.tsx`
- [ ] **T102** [P] [US1] Web UIの時刻表示を監査しローカル変換に統一する: `src/web/client/src/pages/BranchDetailPage.tsx`, `src/web/client/src/components/branch-detail/BranchInfoCards.tsx`, `src/web/client/src/components/branch-detail/SessionHistoryTable.tsx`, `src/web/client/src/components/EnvEditor.tsx`
- [ ] **T103** [US1] ISO文字列の直表示が残っていないことを確認し、必要なら置換する: `src/web/client/src/pages/ConfigPage.tsx`, `src/web/client/src/components/CustomCodingAgentForm.tsx`, `src/cli/ui/screens/solid/LogDetailScreen.tsx`

## フェーズ2: ユーザーストーリー2 - 表示フォーマットを維持する (優先度: P2)

**ストーリー**: 既存の表示形式を保ったままローカル変換する  
**価値**: レイアウト崩れを防ぎ、視認性を維持する

- [ ] **T201** [US2] CLIの表示フォーマットが維持されるよう調整する: `src/cli/ui/utils/branchFormatter.ts`, `src/logging/formatter.ts`, `src/cli/ui/screens/solid/BranchListScreen.tsx`
- [ ] **T202** [US2] Web UIの表示フォーマットが維持されるよう調整する: `src/web/client/src/pages/BranchDetailPage.tsx`, `src/web/client/src/components/branch-detail/BranchInfoCards.tsx`, `src/web/client/src/components/branch-detail/SessionHistoryTable.tsx`, `src/web/client/src/components/EnvEditor.tsx`

## フェーズ3: ユーザーストーリー3 - 表示ロジックを一貫化する (優先度: P3)

**ストーリー**: 表示ロジックを共通化し、UTC表示の再発を防ぐ  
**価値**: 保守性と再発防止

- [ ] **T301** [P] [US3] 共通フォーマッタを整備し既存呼び出しを統一する: `src/cli/ui/utils/branchFormatter.ts`, `src/logging/formatter.ts`, `src/web/client/src/lib/dateTime.ts`（新規または既存に追加）
- [ ] **T302** [P] [US3] CLIの表示テストを更新する: `src/cli/ui/__tests__/utils/branchFormatter.test.ts`, `src/cli/ui/__tests__/solid/LogScreen.test.tsx`, `src/cli/ui/__tests__/solid/BranchListScreen.test.tsx`
- [ ] **T303** [P] [US3] Web UIの表示テストを更新する: `tests/web/client/components/env-editor.test.tsx`（必要に応じて追加テスト）

## フェーズ4: 統合と品質チェック

- [ ] **T401** [統合] 仕様のテスト要件を満たすテストを実行し失敗があれば修正する: `bun run test`
- [ ] **T402** [統合] リント最小要件を満たす: `bun run format:check`, `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`, `bun run lint`
