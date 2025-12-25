# Tasks: ログ一覧・詳細表示・クリップボードコピー機能

## Phase 1: 基盤準備
- [ ] T001 ログファイル読み込みユーティリティの作成 (`src/logging/reader.ts`)
- [ ] T002 ログエントリのパース・整形ユーティリティの作成 (`src/logging/formatter.ts`)

## Phase 2: ログ一覧画面
- [ ] T010 [Test] ログ一覧コンポーネントのテスト作成 (`tests/cli/ui/LogList.test.tsx`)
- [ ] T011 ログ一覧コンポーネントの作成 (`src/cli/ui/components/LogList.tsx`)
- [ ] T012 ブランチ選択画面に `l` キーバインドを追加 (`src/cli/ui/screens/BranchSelection.tsx`)

## Phase 3: ログ詳細表示
- [ ] T020 [Test] ログ詳細コンポーネントのテスト作成 (`tests/cli/ui/LogDetail.test.tsx`)
- [ ] T021 ログ詳細コンポーネントの作成 (`src/cli/ui/components/LogDetail.tsx`)
- [ ] T022 ログ一覧画面からの遷移実装

## Phase 4: クリップボードコピー
- [ ] T030 [Test] クリップボードコピー機能のテスト作成 (`tests/cli/ui/clipboard.test.ts`)
- [ ] T031 クリップボードコピーユーティリティの作成 (`src/cli/ui/utils/clipboard.ts`)
- [ ] T032 ログ一覧・詳細画面への `c` キーバインド追加

## Phase 5: 日付選択（オプション）
- [ ] T040 日付選択コンポーネントの作成 (`src/cli/ui/components/LogDatePicker.tsx`)
- [ ] T041 ログ一覧画面への `d` キーバインド追加

## Dependencies
- Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5
- SPEC-b9f5c4a1 のロガー実装に依存

## Parallel execution opportunities
- [P] T001 and T002 can run in parallel
- [P] T010 and T020 tests can be authored in parallel
- [P] T030 can start after T011 is complete
