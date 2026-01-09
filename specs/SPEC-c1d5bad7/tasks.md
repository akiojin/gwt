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
- [x] T120 [Test] デフォルト有効で stdout/stderr を取り込むことを検証 (`tests/logging/agent-output.test.ts` など)
- [x] T121 [Test] `GWT_CAPTURE_AGENT_OUTPUT=false|0` で無効化できることを検証 (`tests/logging/agent-output.test.ts` など)
- [x] T130 デフォルト有効/opt-out 判定と出力取り込みの実装（`GWT_CAPTURE_AGENT_OUTPUT`） (`src/launcher.ts` / `src/logging/agentOutput.ts`)
- [ ] T131 stdout/stderr ログに `agentId` を付与する (`src/launcher.ts`)
- [ ] T132 Log Viewer で agent stdout/stderr が表示されることを検証 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)

## Phase 8: ログ一覧の表示強化
- [ ] T170 [Test] フィルタ入力で一覧が絞り込まれることを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [ ] T171 [Test] 表示レベル切替が循環することを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [ ] T172 [Test] `r`/`t` のキーバインドが動作することを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [ ] T173 [Test] 表示幅超過時に `...` で省略され、折り返しにならないことを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [ ] T180 フィルタ入力モードの実装（一覧上部に入力行を表示） (`src/cli/ui/screens/solid/LogScreen.tsx`)
- [ ] T181 表示レベルの循環としきい値フィルタの実装 (`src/cli/ui/screens/solid/LogScreen.tsx`)
- [ ] T182 `r`/`t` による再読込・tail 切替の実装 (`src/cli/ui/App.solid.tsx` / `src/cli/ui/screens/solid/LogScreen.tsx`)
- [ ] T183 一覧は折り返さず右端を `...` で省略し、Wrap トグル表示/キーを削除 (`src/cli/ui/screens/solid/LogScreen.tsx`)

## Phase 9: 回帰チェック
- [ ] T160 既存ログ表示の回帰テストを実行（関連テスト + lint/format）

## Phase 10: CLIカラーリング統一
- [x] T190 [Test] ログ一覧の選択行がシアン背景で全幅表示されることを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [x] T191 [Test] ログ一覧の `LEVEL` 列がレベル配色で表示されることを確認 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [x] T192 共有カラー定義（選択行/ログレベル）を追加 (`src/cli/ui/core/theme.ts`)
- [x] T193 ログ一覧の選択行/レベル配色を反映 (`src/cli/ui/screens/solid/LogScreen.tsx`)
- [x] T194 CLI全体の選択行スタイルを統一（Selector/Env/Confirm/SelectInput など） (`src/cli/ui/screens/solid/*.tsx`, `src/cli/ui/components/solid/SelectInput.tsx`)

## 追加作業ToDo (2026-01-09)
- [x] T200 [Test] ログ一覧の時刻表示がシステムロケールのローカル時刻になることを検証 (`src/logging/formatter.ts` 周辺)
- [x] T201 ブランチ一覧とログ一覧の時刻表示をローカル時刻へ切り替え (`src/cli/ui/utils/branchFormatter.ts`, `src/cli/ui/screens/solid/BranchListScreen.tsx`, `src/logging/formatter.ts`)
- [x] T202 既存テストの期待値をローカル時刻に合わせて更新 (`src/cli/ui/utils/__tests__/branchFormatter.test.ts`, `src/cli/ui/__tests__/utils/branchFormatter.test.ts`)
- [x] T210 [Test] `x` でログリセットが発火し、フッターに Reset が表示されることを検証 (`src/cli/ui/__tests__/solid/LogScreen.test.tsx`)
- [x] T211 [Test] ログリセットで対象ログファイルが空になることを検証 (`tests/logging/reader.test.ts`)
- [x] T212 ログ対象ディレクトリ内のログファイルを空にするヘルパーを追加 (`src/logging/reader.ts`)
- [x] T213 Log Viewer の `x` キーバインドとリセット通知を実装 (`src/cli/ui/screens/solid/LogScreen.tsx`, `src/cli/ui/App.solid.tsx`)
