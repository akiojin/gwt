# エージェント検出・起動・ライフサイクル

> **Canonical Boundary**: 本 SPEC はビルトインエージェントの catalog / detection / version / launch contract を扱う。Assistant 送信制御は `SPEC-1636`、Custom Agent 登録は `SPEC-1779` が担当する。

## Background

- gwt は Claude Code / Codex / Gemini / OpenCode / Copilot を launch target として扱う。
- 既存の SPEC-1646 は Assistant Mode 監視や UI 全体まで含み、`SPEC-1636` と `SPEC-1779` と責務が重なっている。
- 本 SPEC はビルトインエージェントの検出、利用可能バージョン、起動引数契約に範囲を絞る。

## User Stories

### US-1: 利用可能なエージェントを検出する

開発者として、ローカル環境で利用可能なエージェントとバージョンを一覧で把握したい。

### US-2: 選択したエージェントを正しい引数で起動する

開発者として、model / version / permissions / collaboration mode などを正しい CLI 契約で渡したい。

### US-3: 起動失敗を原因つきで扱う

開発者として、エージェントが見つからない、バージョン不正、起動失敗などを明確に知りたい。

## Acceptance Scenarios

1. 起動ウィザードでビルトインエージェント一覧が表示される。
2. 各エージェントの version / model 選択肢が CLI 契約に沿って表示される。
3. 起動時に選択内容が launch builder へ反映される。
4. エージェント未検出や起動失敗時に原因つきエラーが UI とログへ残る。
5. Custom Agent は別カテゴリとして表示されても、本 SPEC の validation / persistence 要件には含めない。

## Edge Cases

- 検出済みバージョンが古く、新しい引数契約に対応していない。
- Default モデルを選んだ場合に余計な `--model` を渡さない。
- Agent ごとに利用可能な reasoning / fast mode が異なる。

## Functional Requirements

- FR-001: ビルトインエージェントの検出とバージョン確認を提供する。
- FR-002: Agent ごとの launch contract（model/version/permissions/session mode）を定義する。
- FR-003: 起動失敗を構造化エラーとして扱い、UI とログへ反映する。
- FR-004: Built-in agent の UI 表示名と内部 ID の対応を維持する。
- FR-005: Custom Agent の登録・永続化は本 SPEC の対象外とする。

## Success Criteria

- 各ビルトイン Agent の検出と起動契約が 1 つの SPEC にまとまる。
- Agent launch builder の責務境界が明確になる。
- 起動 UI と gwt-core の Agent 定義が同期する。
