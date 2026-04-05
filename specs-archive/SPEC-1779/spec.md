> **Canonical Boundary**: `SPEC-1779` は custom agent 登録 UI と永続化の正本である。built-in agent の検出 / launch contract は `SPEC-1646` が担当する。

# カスタムエージェント登録

## Background

gwt-tui の Settings タブに CustomAgents カテゴリを追加し、ユーザーが独自のエージェント定義を登録できるようにする。カスタムエージェントは Wizard の AgentSelect ステップに表示され、AgentLaunchBuilder に統合される。

## User Stories

### US-1: カスタムエージェントを登録する

開発者として、Settings タブの CustomAgents カテゴリで新しいエージェント定義（ID、表示名、コマンド、引数テンプレート、npm パッケージ名、対応モデル）を登録し、Wizard で選択可能にしたい。

### US-2: カスタムエージェントを Wizard で選択する

開発者として、Wizard の AgentSelect ステップでビルトインエージェントとカスタムエージェントの両方から選択し、同じワークフローでエージェントを起動したい。

### US-3: カスタムエージェントを編集・削除する

開発者として、登録済みのカスタムエージェント定義を編集または削除して、設定を更新したい。

## Acceptance Scenarios

### AS-1: エージェント登録

- Settings タブで CustomAgents カテゴリを選択
- 新規登録フォームで ID、表示名、コマンド、引数テンプレートを入力
- 保存後、カスタムエージェント一覧に表示される

### AS-2: Wizard での選択

- Wizard の AgentSelect ステップを開く
- ビルトインエージェントとカスタムエージェントが一覧に表示される
- カスタムエージェントを選択して起動できる

### AS-3: AgentLaunchBuilder 統合

- カスタムエージェントを選択して Wizard を進める
- AgentLaunchBuilder がカスタムエージェントのコマンドと引数テンプレートを使用してプロセスを起動する

### AS-4: 編集・削除

- カスタムエージェント一覧で既存定義を選択
- 編集して保存、または削除を実行
- 変更が即座に Wizard に反映される

## Functional Requirements

| ID | 要件 |
|----|------|
| FR-001 | カスタムエージェント定義: ID、表示名、コマンド、引数テンプレート、npm パッケージ名（オプション）、対応モデル（オプション）を保持する |
| FR-002 | Settings タブの CustomAgents カテゴリで CRUD 操作を提供する |
| FR-003 | カスタムエージェント定義を `~/.gwt/custom-agents.json` に永続化する |
| FR-004 | Wizard の AgentSelect ステップにカスタムエージェントを表示する |
| FR-005 | AgentLaunchBuilder がカスタムエージェントのコマンドと引数テンプレートでプロセスを起動する |
| FR-006 | 引数テンプレートは変数展開をサポートする（`{worktree_path}`, `{branch_name}` 等） |
| FR-007 | カスタムエージェントのバリデーション（コマンド存在確認等）を提供する |

## Non-Functional Requirements

| ID | 要件 |
|----|------|
| NFR-001 | カスタムエージェント定義の読み込みは起動時に非同期で行い、TUI の初期表示をブロックしない |
| NFR-002 | 定義ファイル（JSON）の破損時はデフォルト値にフォールバックする |

## Success Criteria

| ID | 基準 |
|----|------|
| SC-001 | Settings タブでカスタムエージェントの登録・編集・削除ができる |
| SC-002 | Wizard の AgentSelect にカスタムエージェントが表示される |
| SC-003 | カスタムエージェントのコマンドでプロセスが起動される |
| SC-004 | 引数テンプレートの変数展開が正しく動作する |
| SC-005 | 定義が `~/.gwt/custom-agents.json` に永続化される |
