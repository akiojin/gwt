### 背景

gwt は現在 4 つの AI コーディングエージェント（Claude Code, Codex, Gemini, OpenCode）をサポートしている。2026年2月に GA となった GitHub Copilot CLI（スタンドアロン版 `copilot` コマンド、npm: `@github/copilot`）を 5 つ目のエージェントとして追加する。

親仕様: SPEC-3b0ed29b（コーディングエージェント対応 — `specs/archive/SPEC-3b0ed29b/spec.md`）
既存仕様のユーザーストーリー US1〜US17 は Claude Code / Codex / Gemini / OpenCode を対象としており、本 Issue は GitHub Copilot CLI を追加する拡張仕様。

### ユーザーシナリオ

| ID | シナリオ | 優先度 |
|----|---------|--------|
| US1 | Agent Launch Form で GitHub Copilot を選択し、デフォルト設定でエージェントを起動できる | P0 |
| US2 | 「Continue」モードで `--continue` 引数が渡され、最新セッションが再開される | P0 |
| US3 | 権限スキップモードで `--allow-all-tools` フラグが渡される | P1 |
| US5 | モデル選択画面で `claude-sonnet-4-5` を選択し、`--model claude-sonnet-4-5` 引数が渡される | P1 |
| US9 | ステータスバーに「Copilot: v{version}」（緑）または「Copilot: bunx」（黄）が表示される | P1 |
| US10 | Copilot のエージェント名が青色（Blue / #4299e1）で表示される | P2 |

### 機能要件

| ID | 要件 | 関連 US |
|----|------|---------|
| FR-CP01 | `copilot` コマンドの検出（`which copilot` + `copilot --version`） | US1, US9 |
| FR-CP02 | 認証状態の確認（`gh auth status` — GitHub OAuth ベース） | US1 |
| FR-CP03 | エージェント起動（`copilot` / `bunx @github/copilot`） | US1 |
| FR-CP04 | セッションモード対応（Normal / Continue via `--continue`）。Resume も `--continue` にマップ（セッション ID 指定未対応） | US2 |
| FR-CP05 | モデル選択（`--model` フラグ） | US5 |
| FR-CP06 | Skip permissions（`--allow-all-tools` フラグ） | US3 |
| FR-CP07 | フロントエンド UI（AgentId 型拡張、modelOptions、supportsModelFor 更新） | US1, US5 |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | Agent Launch Form から GitHub Copilot を選択して起動できる |
| SC-002 | モデル選択で `--model` 引数が正しく渡される |
| SC-003 | Continue モードで `--continue` 引数が渡される |
| SC-004 | Skip permissions で `--allow-all-tools` 引数が渡される |
| SC-005 | 既存 4 エージェントの動作に影響がない |

### 設計判断

| 項目 | 判断 | 理由 |
|------|------|------|
| 認証チェック | `gh auth status` | Copilot は GitHub OAuth ベース。env var 不使用 |
| Resume | `--continue` にマップ | Copilot CLI はセッション ID 指定の resume 未対応 |
| 色 | `AgentColor::Blue` | 既存エージェントと重複なし。GitHub ブランドカラー |
| Skip permissions | `--allow-all-tools` | Copilot CLI の公式フラグ |
| npm パッケージ | `@github/copilot` | 公式パッケージ名。bunx/npx フォールバック自動対応 |
