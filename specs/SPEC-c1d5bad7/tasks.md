# Tasks: ログ一覧・詳細表示・クリップボードコピー機能

**仕様ID**: `SPEC-c1d5bad7`
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める。

## 作業ToDo (2025-12-25)
- [x] T001〜T041 を順に実施（TDD 優先）

## Phase 1: 基盤準備
- [x] T001 ログファイル読み込みユーティリティの作成 (`src/logging/reader.ts`)
- [x] T002 ログエントリのパース・整形ユーティリティの作成 (`src/logging/formatter.ts`)

## Phase 2: ログ一覧画面
- [x] T010 [Test] ログ一覧コンポーネントのテスト作成 (`src/cli/ui/__tests__/components/screens/LogListScreen.test.tsx`)
- [x] T011 ログ一覧コンポーネントの作成 (`src/cli/ui/components/screens/LogListScreen.tsx`)
- [x] T012 ブランチ選択画面に `l` キーバインドを追加 (`src/cli/ui/components/screens/BranchListScreen.tsx`)

## Phase 3: ログ詳細表示
- [x] T020 [Test] ログ詳細コンポーネントのテスト作成 (`src/cli/ui/__tests__/components/screens/LogDetailScreen.test.tsx`)
- [x] T021 ログ詳細コンポーネントの作成 (`src/cli/ui/components/screens/LogDetailScreen.tsx`)
- [x] T022 ログ一覧画面からの遷移実装 (`src/cli/ui/components/App.tsx`)

## Phase 4: クリップボードコピー
- [x] T030 [Test] クリップボードコピー機能のテスト作成 (`src/cli/ui/__tests__/utils/clipboard.test.ts`)
- [x] T031 クリップボードコピーユーティリティの作成 (`src/cli/ui/utils/clipboard.ts`)
- [x] T032 ログ一覧・詳細画面への `c` キーバインド追加 (`src/cli/ui/components/screens/LogListScreen.tsx`, `src/cli/ui/components/screens/LogDetailScreen.tsx`)

## Phase 5: 日付選択（オプション）
- [x] T040 日付選択コンポーネントの作成 (`src/cli/ui/components/screens/LogDatePickerScreen.tsx`)
- [x] T041 ログ一覧画面への `d` キーバインド追加 (`src/cli/ui/components/screens/LogListScreen.tsx`)

## Dependencies
- Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5
- SPEC-b9f5c4a1 のロガー実装に依存

## Parallel execution opportunities
- [P] T001 and T002 can run in parallel
- [P] T010 and T020 tests can be authored in parallel
- [P] T030 can start after T011 is complete
