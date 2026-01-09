# 実装計画: セッションID永続化とContinue/Resume強化

**仕様ID**: `SPEC-f47db390` | **日付**: 2025-12-06 | **仕様書**: [spec.md](./spec.md)  
**入力**: `/specs/SPEC-f47db390/spec.md` からの機能仕様

## 概要

Continue/Resume時に必ず正しいセッションへ戻れるよう、セッションIDを自動取得・保存し、再開時に明示的にCLIへ渡す。Quick Startでは選択ブランチのworktreeに紐づく最新セッションを再走査して提示し、他ブランチのセッション混入を防ぐ。

## 技術コンテキスト

- **言語/バージョン**: TypeScript 5.8 / Bun 1.0 / OpenTUI (Solid)
- **主要依存**: chalk, execa, fs/promises, @opentui/solid, existing session config utilities (`src/config/index.ts`)
- **ストレージ**: ローカルファイル  
  - Codex: `~/.codex/sessions/*.json`  
  - Claude Code: `~/.claude/projects/<path-encoded>/sessions/*.jsonl`
- **テスト**: Vitest + Testing Library (CLIコンポーネント)
- **ターゲット**: Unix系ターミナル/WSL/Windows（CLI）
- **制約**: 既存セッションファイルとの後方互換、24時間以内のセッションのみ有効

## 原則チェック

- シンプルに実装しつつUXを最優先（CLAUDE.md指針）。
- 既存ファイルを優先改修し、新規ファイルの乱立を避ける。
- CLI操作の直感性を保つ（退出時の明示メッセージ、一貫したキー操作）。

## プロジェクト構造（本機能）

```
specs/SPEC-f47db390/
├── spec.md      # 機能仕様
├── plan.md      # 本ファイル
└── tasks.md     # タスク一覧（本計画を反映して更新予定）
```

関連コードパス: `src/config/index.ts`, `src/codex.ts`, `src/claude.ts`, `src/gemini.ts`, `src/utils/session/parsers/`, `src/worktree.ts`, `src/cli/ui/App.solid.tsx`, `src/cli/ui/components/solid/QuickStartStep.tsx`, `src/cli/ui/utils/continueSession.ts`。テストは `tests/unit/utils/`, `src/cli/ui/__tests__/solid/`。

## フェーズ0: 調査

- Codexは`codex resume <session-id>`/`--last`で再開でき、セッションは`~/.codex/sessions`に保存される。
- Claude Codeは`--resume <session-id>`/`--continue`を提供し、プロジェクト別に`~/.claude/projects/<cwd-encoded>/sessions/*.jsonl`へ保存する。
- 現行gwtのセッション保存は`SessionData`（24h有効）で、sessionIdは未保持。SessionSelectorは未実装で空配列。

## フェーズ1: 設計

### データモデル
- `SessionData`: 任意フィールド`lastSessionId?: string`を追加。
- `ToolSessionEntry`: `sessionId?: string`を追加し履歴100件の上限は維持。

### 取得アルゴリズム
- Codex: `~/.codex/sessions/*.json*`の最終更新ファイルを読み、`id`を保存。Quick Start再走査では選択ブランチのworktreeに合致するセッションのみを採用。
- Claude: `~/.claude/projects/<encoded cwd>/sessions/*.jsonl`の最新行から`id`を抽出。ブランチ指定時はworktree一覧から候補パスを生成して限定検索。
- Gemini/OpenCode: セッションファイル内の`cwd`をworktree一覧でブランチ解決し、対象ブランチの最新セッションを採用。
- 失敗時は警告ログのみで処理続行し、Quick Startは履歴にフォールバック。

### 起動フロー
- Continue: 保存済みIDがあればCodexに`resume <id>`, Claudeに`--resume <id>`。なければ従来`--last`/`-c`。
- Resume: SessionSelectorに保存履歴を渡し、選択IDで起動。履歴なしなら警告して通常起動。
- 終了時: IDと再開コマンド例を表示（ツール別文言）。

### UI
- SessionSelectorScreenに実データを配線し、ツール・ブランチ・時刻・IDを表示。空時は警告表示。
- Branch選択直後にQuick Start画面を追加し、前回のツール/モデル/セッションIDを提示。「前回設定で続きから」「前回設定で新規」「設定を選び直す」を選べる。
- Quick Startはツール別行を表示し、Resume行はsessionIdがある場合のみ表示。表示時にセッションファイル再走査で最新IDへ更新する。

## フェーズ2: タスク生成

tasks.mdをP1(P2)順に具体化する（後述のタスク案を反映）。

## 実装戦略

1. **P1/US1**: セッション取得と保存・Continue再開を実装（Codex/Claude/Gemini/OpenCode）。  
2. **P1/US2**: 終了時のID表示とログ出力を追加。  
3. **P2/US3**: SessionSelectorへ履歴データを配線し、Resume起動を実装。  
4. **P1/US5**: Branch選択後に前回設定で素早く再開/新規を選べるQuick Start画面を実装（履歴なしは従来フロー）。  
5. **P1/US6**: Quick Start再走査とブランチ整合セッション解決（Claude含む）を実装。  
6. 回帰テスト（保存失敗時フォールバック、非対応ツールの挙動維持）。

## テスト戦略

- ユニット: セッションID抽出とブランチ解決（Codex/Claude/Gemini/OpenCode）を検証。
- 統合: Continue/ResumeフローでCLI呼び出し引数をモックし、sessionIdが渡されることを確認。
- UI: Quick Startのツール別行、Resume非表示条件、再走査反映を検証（Snapshotなし）。
- 回帰: 非対応ツールで従来起動が壊れないことを確認。

## リスクと緩和策

1. **セッションファイルの場所差異**: Windows/WSLでパスが異なる → path.joinとホームディレクトリ取得の共通ヘルパーで吸収、失敗時フォールバック。
2. **ファイル読み取り失敗**: パーミッション/存在しない → 例外を握りつぶしつつ警告を出し、従来挙動に戻す。
3. **履歴スキーマ変更の互換性**: 任意フィールドとして追加し、パース失敗時は無視する。

## 次のステップ

1. tasks.mdを本計画に沿って更新（P1/P2の具体タスク化）。
2. ブランチ整合セッション解決とQuick Start再走査の実装。
3. テスト追加・実行（lint/format含む）。
