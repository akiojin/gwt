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

## 追加作業ToDo (2026-01-08)
- [ ] T100〜T160 を順に実施（TDD 優先）

## Phase 6: ブランチ連動ログ
- [ ] T100 [Test] ログ対象ディレクトリ決定ロジックのテスト追加（worktree優先/現在ブランチfallback/それ以外は空） (`tests/cli/logviewer.test.ts` など既存構成に合わせる)
- [ ] T101 ログ対象ディレクトリ決定ロジックの実装 (`src/cli/ui/App.solid.tsx` / `src/logging/reader.ts` など)
- [ ] T102 ブランチ一覧のカーソルブランチを log viewer に渡す (`src/cli/ui/App.solid.tsx`)
- [ ] T110 [Test] Log Viewer に Branch/Source を表示するテスト追加 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [ ] T111 Log Viewer の Branch/Source 表示実装 (`src/cli/ui/screens/solid/LogScreen.tsx`)

## Phase 7: エージェント stdout/stderr 取り込み
- [ ] T120 [Test] opt-in 無効時は stdout/stderr を取り込まないことを検証 (`tests/logging/agent-output.test.ts` など)
- [ ] T121 [Test] opt-in 有効時に stdout/stderr が JSONL に追記されることを検証 (`tests/logging/agent-output.test.ts` など)
- [ ] T130 opt-in 判定と出力取り込みの実装（`GWT_CAPTURE_AGENT_OUTPUT`） (`src/launcher.ts` / `src/logging/logger.ts`)
- [ ] T131 stdout/stderr ログに `agentId` を付与する (`src/launcher.ts`)
- [ ] T132 Log Viewer で agent stdout/stderr が表示されることを検証 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)

## Phase 8: ログ一覧の表示強化
- [ ] T170 [Test] フィルタ入力で一覧が絞り込まれることを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [ ] T171 [Test] 表示レベル切替が循環することを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [ ] T172 [Test] `r`/`t`/`w` のキーバインドが動作することを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [ ] T180 フィルタ入力モードの実装（一覧上部に入力行を表示） (`src/cli/ui/screens/solid/LogScreen.tsx`)
- [ ] T181 表示レベルの循環としきい値フィルタの実装 (`src/cli/ui/screens/solid/LogScreen.tsx`)
- [ ] T182 `r`/`t` による再読込・tail 切替の実装 (`src/cli/ui/App.solid.tsx` / `src/cli/ui/screens/solid/LogScreen.tsx`)
- [ ] T183 `w` による折り返し切替と一覧列揃えの実装 (`src/cli/ui/screens/solid/LogScreen.tsx`)

## Phase 9: 回帰チェック
- [ ] T160 既存ログ表示の回帰テストを実行（関連テスト + lint/format）
