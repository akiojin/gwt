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

## フェーズ5: US4 Gemini対応 (P2)
- [x] **T0401** `[US4]` GeminiセッションID抽出ヘルパーを実装（`~/.gemini/tmp/**/chats/*.json` 最新ID）（`src/utils/session.ts`）。
- [x] **T0402** `[US4]` Gemini Continue/Resumeで`--resume <id>`を優先、ID不明時は`--resume`にフォールバック（`src/gemini.ts`）。
- [x] **T0403** `[US4]` Geminiのセッション保存・再開テストを追加（ユーティリティユニットテスト + CLI引数組み立てテスト）。

## フェーズ6: ポリッシュと検証
- [x] **T9001** `[共通]` フォーマット/リンター/markdownlintをローカル実行し、エラーを解消する。
- [x] **T9002** `[共通]` `bun run test` と `bun run build` を完走させ、失敗がないことを確認する。

## 追加作業: Rust版 Quick Startの即時起動 (2026-01-14)

- [x] **T0607** `[P][共通]` `specs/SPEC-f47db390/spec.md` にQuick Start即時起動要件を追記
- [x] **T0608** `[P][共通]` `specs/SPEC-f47db390/plan.md` にQuick Start即時起動方針を追記
- [x] **T0609** `[Test]` `crates/gwt-cli/src/tui/screens/wizard.rs` にQuick Start確定時の即時完了テストを追加
- [x] **T0610** `[実装]` `crates/gwt-cli/src/tui/screens/wizard.rs` と `crates/gwt-cli/src/tui/app.rs` でQuick Start確定時にExecution Mode/Skip Permissionsをスキップして即時起動する

## 追加作業: Quick StartのセッションID引数反映 (2026-01-14)

- [x] **T0611** `[Test]` `crates/gwt-cli/src/main.rs` にsession_idが起動引数へ反映されるテストを追加
- [x] **T0612** `[実装]` `crates/gwt-cli/src/tui/screens/wizard.rs` と `crates/gwt-cli/src/main.rs` でQuick Startのsession_idを起動引数に反映

## 追加作業: Rust版セッションID取得と保存 (2026-01-14)

- [x] **T0613** `[Test]` `crates/gwt-cli/src/main.rs` にCodex/ClaudeのセッションID検出テストを追加
- [x] **T0614** `[実装]` `crates/gwt-cli/src/main.rs` でCodex/Claude/Gemini/OpenCodeのセッションIDを検出し、終了後に履歴へ保存

## 追加作業: Quick Start履歴フォールバック (2026-01-14)

- [x] **T0615** `[Test]` `crates/gwt-core/src/config/ts_session.rs` に`lastBranch`フォールバックでQuick Start履歴が取得できるテストを追加
- [x] **T0616** `[実装]` `crates/gwt-core/src/config/ts_session.rs` で`history`が空の場合に`lastBranch/lastUsedTool`を Quick Start履歴へ反映する

## 追加作業: Quick Start toolId正規化 (2026-01-19)

- [x] **T0617** `[Test]` `crates/gwt-core/src/config/ts_session.rs` にtoolId正規化で履歴が重複しないことを検証するテストを追加
- [x] **T0618** `[実装]` `crates/gwt-core/src/config/ts_session.rs` でtoolIdを正規化して保存・読込し、Quick Startの集約キーに反映する

## 追加作業: TSセッション履歴の正規化マイグレーション (2026-01-19)

- [ ] **T0619** `[Test]` `crates/gwt-core/src/config/ts_session.rs` で`load_ts_session`が表記ゆれを正規化し、セッションファイルへ書き戻すことを検証するテストを追加
- [x] **T0619** `[Test]` `crates/gwt-core/src/config/ts_session.rs` で`load_ts_session`が表記ゆれを正規化し、セッションファイルへ書き戻すことを検証するテストを追加
- [x] **T0620** `[実装]` `crates/gwt-core/src/config/ts_session.rs` で読み込み時に`toolId`/`lastUsedTool`を正規化し、差分があればセッションファイルを自動更新する
## フェーズ7: ブランチクイックスタート (P1)
- [x] **T0501** `[US5]` ブランチ選択後に前回ツール/モデル/セッションIDを提示するQuick Start画面を追加（Inkコンポーネント新設）。
- [x] **T0502** `[US5]` 履歴が存在する場合はQuick Startへ遷移し、選択結果に応じてツール/モデル/セッションIDを事前セットするロジックを実装（履歴なしは従来フロー）。
- [x] **T0503** `[US5]` Quick StartのUIテストと統合テスト（前回設定で続きから/新規、履歴なしフォールバック）を追加。
- [x] **T0504** `[US5]` Quick Startの表示ルールをツール別に分岐し、CodexのみReasoningを表示、Start newではセッションIDを非表示にするUIテストを追加（`src/cli/ui/__tests__/components/screens/BranchQuickStartScreen.test.tsx`）。
- [ ] **T0505** `[US5]` 同一ブランチでツールごとに直近設定を保持し、Quick Startでツール別行（Resume/Start new）を生成するロジックとテストを追加する。

## フェーズ8: Web UI セッションID表示/再開 (P1)
- [ ] **T0601** `[US6]` Web API型に`resumeSessionId`/`sessionId`を追加し、互換性を維持する（`src/types/api.ts`）。
- [ ] **T0602** `[US6]` Web UIで最終セッションIDを表示し、Continue/Resume起動時にIDを送信する（`src/web/client/src/pages/BranchDetailPage.tsx`, `src/web/client/src/components/branch-detail/BranchInfoCards.tsx`）。
- [ ] **T0603** `[US6]` Web APIで`resumeSessionId`を受け取り、Codex/Claudeの起動引数に反映する（`src/web/server/routes/sessions.ts`, `src/web/server/pty/manager.ts`, `src/services/codingAgentResolver.ts`）。
- [ ] **T0604** `[US6]` Web UI起動セッションの終了時にID検出と履歴保存を行う（`src/web/server/pty/manager.ts`, `src/utils/session/*`）。
- [ ] **T0605** `[US6]` `getLastToolUsageMap`の後方互換で`lastSessionId`を補完する（`src/config/index.ts`）。
- [ ] **T0606** `[US6]` `buildClaudeArgs`/`buildCodexArgs`のID指定をユニットテストで検証する（`tests/unit/*codingAgentResolver*.test.ts`）。
