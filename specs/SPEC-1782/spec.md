# Quick Start — ブランチ単位のワンクリックエージェント起動

Parent: SPEC-1776 (FR-022, US3, US5)

## Background

gwt はブランチごとにエージェントを起動し、ワークツリーで隔離された作業環境を提供する。ユーザーは同じブランチで繰り返しエージェントを起動するケースが多い。毎回ウィザードの全ステップを通るのは非効率であり、前回の設定を記憶してワンクリックで起動できる Quick Start 機能が必要。

gwt-cli では Quick Start が実装されていたが、gwt-tui への移行で統合レイヤーが未接続のまま残っている。本 SPEC では Quick Start を再設計し、gwt-tui で完全に動作させる。

### 現状の問題

1. `open_for_branch()` に常に空の履歴（`vec![]`）が渡されており Quick Start が表示されない
2. `WizardExecutionMode` が `AgentLaunchBuilder` に伝達されておらず、Resume/Continue が機能しない
3. `detect_session_id_for_tool()` が gwt-core に未移植で session_id が常に None
4. `save_session_entry()` で session_id, mode, tool_label が不正確

## User Stories

### US-1: ワンクリックでセッション再開 (P0)

開発者として、ブランチに戻ったとき、前回のエージェントセッションをワンクリックで再開したい。設定の再選択なしで、前回中断した場所から作業を続けたい。

**受け入れシナリオ:**

- AS-1-1: Branches タブで Enter → session_id がある場合 Quick Start 表示
- AS-1-2: "Resume session (abc123...)" を選択 → `--resume <id>` 付きで即座に起動
- AS-1-3: agent, model, version, skip_permissions, reasoning_level, fast_mode, collaboration_modes が全て復元される

### US-2: ワンクリックで新規セッション開始 (P0)

開発者として、前回と同じエージェント設定で新しいセッションをワンクリックで開始したい。新しいタスクに取り組むとき、設定の再選択を省きたい。

**受け入れシナリオ:**

- AS-2-1: Quick Start 画面で "Start new session" を選択 → Normal モードで即座に起動
- AS-2-2: session_id は渡されない（新規セッション）
- AS-2-3: 前回の全設定が復元される

### US-3: 設定を変更して起動 (P1)

開発者として、Quick Start 表示時に別のエージェントや設定に変更して起動する選択肢も持ちたい。

**受け入れシナリオ:**

- AS-3-1: Quick Start 画面で "Choose different settings" を選択 → フルウィザード（BranchAction から）に遷移
- AS-3-2: フルウィザードは常に Normal モードで新規セッションを起動する

### US-4: session_id の自動検出 (P0)

開発者として、session_id を手動で管理することなく、gwt が自動的にエージェントのセッションファイルから検出・保存してほしい。

**受け入れシナリオ:**

- AS-4-1: エージェント起動後、gwt が各エージェントのセッションファイルをスキャンして session_id を取得
- AS-4-2: 取得した session_id が `~/.gwt/sessions/` の TOML に保存される
- AS-4-3: 次回 Quick Start 時に session_id が表示され Resume が使用可能

## Functional Requirements

### Quick Start 表示

- FR-001: Quick Start はウィザードの最初のステップとして、session_id を持つ履歴が存在する場合のみ表示する
- FR-002: 表示するツールは 1 つのみ。ブランチの履歴を timestamp 降順で探索し、session_id がある最初のツールを採用する
- FR-003: session_id がない場合は Quick Start をスキップし、BranchAction ステップから開始する

### Quick Start UI

- FR-010: フラット 3 項目の UI を表示する:
  - Resume session ({session_id の先頭 8 文字}...)
  - Start new session
  - Choose different settings
- FR-011: ステップタイトルに "Quick Start — {Agent名} ({Model})" を表示する
- FR-012: Resume / Start New はワンクリック起動（追加ステップなし、WizardAction::Complete を返す）

### Resume 動作

- FR-020: Resume は常に `--resume <session_id>` を使用する。`--continue` は使用しない
- FR-021: 全設定を復元する: agent, model, version, skip_permissions, reasoning_level, fast_mode, collaboration_modes
- FR-022: 失効した session_id（エージェント側で削除済み）の事前検証は行わない。エージェントのエラーに委ねる

### Start New 動作

- FR-030: Start New は全設定を復元し、Normal モード（新規セッション）で起動する
- FR-031: session_id は渡さない

### Choose Different 動作

- FR-040: Choose Different はフルウィザード（BranchAction ステップ）に遷移する
- FR-041: フルウィザードは常に Normal モードで新規セッションを起動する（ExecutionMode ステップは存在しない）

### session_id 検出

- FR-050: エージェント起動後、各エージェント固有のセッションファイルから session_id を自動検出する
  - Claude Code: `~/.claude/projects/{encoded_path}/` の history.jsonl をパース
  - Codex CLI: `~/.codex/` のセッションファイルをスキャン
  - Gemini CLI: `~/.gemini/` のセッションファイルをスキャン
  - OpenCode: `~/.opencode/` のセッションファイルをスキャン
- FR-051: 検出した session_id を `~/.gwt/sessions/` の TOML ファイルに保存する（`save_session_entry()` で更新）
- FR-052: gwt-cli の `detect_session_id_for_tool()` を gwt-core に移植して使用する

### ウィザードフロー変更

- FR-060: ウィザードに ExecutionMode ステップは存在しない。フルウィザード経由の起動は常に Normal モード
- FR-061: Convert（セッション変換）機能は本 SPEC のスコープ外

### 履歴保存

- FR-070: エージェント起動時に `save_session_entry()` で以下を保存する: branch, tool_id, tool_label, model, version, skip_permissions, reasoning_level, session_id, fast_mode, collaboration_modes
- FR-071: tool_label は `find_agent_def()` の display_name を使用する

## Non-Functional Requirements

- NFR-001: Quick Start 履歴の読み込みは 100ms 以内に完了する
- NFR-002: session_id 検出はエージェント起動後のバックグラウンドで実行し、TUI のレスポンスをブロックしない
- NFR-003: 保存ファイルの破損時は Quick Start をスキップしてフォールバックする

## Edge Cases

- session_id が失効 → エージェントが "session not found" エラーを PTY に表示 → ユーザーが対処
- session_id 検出前にエージェント終了 → session_id = None → 次回 Quick Start 非表示
- 複数エージェント履歴のブランチ → session_id がある最新ツールのみ Quick Start に表示
- Claude(session_id あり, 古い) → Codex(session_id なし, 新しい) → Claude の session_id で Quick Start 表示

## Success Criteria

- SC-001: session_id がある履歴のあるブランチで Enter → Quick Start がフラット 3 項目で表示される
- SC-002: Resume 選択 → `--resume <id>` 付きで即起動、全設定復元
- SC-003: Start New 選択 → 前回設定で Normal モード即起動
- SC-004: Choose Different → フルウィザード（BranchAction から）に遷移
- SC-005: エージェント起動後に session_id が自動検出され、次回 Quick Start で使用可能
- SC-006: session_id がないブランチでは Quick Start スキップ
