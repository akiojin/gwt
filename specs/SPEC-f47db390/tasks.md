---
description: "Continue/Resumeで正しいセッションを再開するためのタスクリスト"
---

# タスク: セッションID永続化とContinue/Resume強化

**入力**: `/specs/SPEC-f47db390/` の spec.md, plan.md  
**前提条件**: plan.md完了、仕様合意済み  
**テスト**: `bun run format:check` / `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` / `bun run lint` / `bun run test` / `bun run build`

## フェーズ1: 共通下準備 (P1)
- [x] **T0001** `[P][共通]` `SessionData`/`ToolSessionEntry`に`sessionId`フィールドを追加し、後方互換で保存・読み出しできるようにする（`src/config/index.ts`）。
- [x] **T0002** `[P][共通]` セッション保存/読み出しのユニット・統合テストを更新（`tests/unit/config/session.test.ts`, `tests/integration/session-continue.test.ts`）。

## フェーズ2: US1 Continue/Resumeが正しいセッションに接続 (P1)
- [x] **T0101** `[US1]` CodexセッションID探索ヘルパーを実装（`~/.codex/sessions/*.json`から最新IDを取得）する（`src/utils/session.ts`）。
- [x] **T0102** `[US1]` Claude CodeセッションID探索ヘルパーを実装（`~/.claude/projects/<encoded>/sessions/*.jsonl`から最新IDを抽出）する（`src/utils/session.ts`）。
- [x] **T0103** `[US1]` `launchCodexCLI`で終了後にIDを取得し`saveSession`へ`lastSessionId`/`history.sessionId`を保存、Continue/Resume時は`resume <id>`を優先する（`src/codex.ts`）。
- [x] **T0104** `[US1]` `launchClaudeCode`で終了後にIDを取得し保存、Continue/Resume時に`--resume <id>`を優先する（`src/claude.ts`）。
- [x] **T0105** `[P][US1]` workflowでmode=continue/resume選択時に保存済みIDを読み出すロジックを追加し、非対応ツールでは従来フローを維持する（`src/index.ts` と関連ハンドラ）。
- [x] **T0106** `[US1]` Continue/Resume引数組み立てをexecaモックで検証するユニットテストを追加（新規 `tests/unit/cli-resume.test.ts` など）。
- [x] **T0107** `[US1]` セッションID取得ヘルパーの異常系（ファイルなし/JSON破損）テストを追加（`tests/unit/utils/session-id.test.ts`）。

## フェーズ3: US2 終了時の再開コマンド提示 (P1)
- [x] **T0201** `[US2]` Codex/Claude終了後にSession IDと再開コマンド例を出力する共通プリンタを実装（`src/codex.ts`, `src/claude.ts`）。
- [x] **T0202** `[US2]` stdoutをスパイしてメッセージが含まれることを確認するテストを追加（`tests/unit/cli-resume.test.ts` 等）。

## フェーズ4: US3 Resume用セッション一覧 (P2)
- [x] **T0301** `[US3]` SessionSelectorScreenへ保存履歴を供給する配線を実装（`src/cli/ui/components/App.tsx`）。
- [x] **T0302** `[US3]` SessionSelectorScreenでツール/ブランチ/時刻/IDを表示し選択値を返す実装を追加（`src/cli/ui/components/screens/SessionSelectorScreen.tsx`）。
- [x] **T0303** `[US3]` UIテストでリスト表示・空表示・選択イベントを検証（`src/cli/ui/__tests__/components/screens/SessionSelectorScreen.test.tsx`）。
- [x] **T0304** `[US3]` Resume選択時に選択IDでCLIを起動する統合テストを追加（`tests/integration/session-resume.test.ts` など）。

## フェーズ5: US4 Gemini/Qwen対応 (P2)
- [x] **T0401** `[US4]` GeminiセッションID抽出ヘルパーを実装（`~/.gemini/tmp/**/chats/*.json` 最新ID）（`src/utils/session.ts`）。
- [x] **T0402** `[US4]` Gemini Continue/Resumeで`--resume <id>`を優先、ID不明時は`--resume`にフォールバック（`src/gemini.ts`）。
- [x] **T0403** `[US4]` Qwenセッションタグ抽出ヘルパーを実装（`~/.qwen/tmp/**` 保存/チェックポイントから取得）（`src/utils/session.ts`）。
- [x] **T0404** `[US4]` Qwen Continue/Resume時に保存タグを表示し、起動後ログで`/chat resume <tag>`案内を出す（`src/qwen.ts` など）。
- [x] **T0405** `[US4]` Gemini/Qwenのセッション保存・再開テストを追加（ユーティリティユニットテスト + CLI引数組み立てテスト）。

## フェーズ6: ポリッシュと検証
- [x] **T9001** `[共通]` フォーマット/リンター/markdownlintをローカル実行し、エラーを解消する。
- [x] **T9002** `[共通]` `bun run test` と `bun run build` を完走させ、失敗がないことを確認する。

## フェーズ7: ブランチクイックスタート (P1)
- [ ] **T0501** `[US5]` ブランチ選択後に前回ツール/モデル/セッションIDを提示するQuick Start画面を追加（Inkコンポーネント新設）。
- [ ] **T0502** `[US5]` 履歴が存在する場合はQuick Startへ遷移し、選択結果に応じてツール/モデル/セッションIDを事前セットするロジックを実装（履歴なしは従来フロー）。
- [ ] **T0503** `[US5]` Quick StartのUIテストと統合テスト（前回設定で続きから/新規、履歴なしフォールバック）を追加。
