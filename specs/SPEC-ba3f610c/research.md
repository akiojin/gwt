# 技術調査: エージェントモード

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-01-22

## 概要

エージェントモード実装に必要な技術スタックの調査結果をまとめる。

## 1. 既存コードベース分析

### 1.1 AIクライアント (`gwt-core/src/ai/client.rs`)

**現状**:

- `AIClient`構造体: OpenAI Responses API互換のHTTPクライアント
- `reqwest::blocking::Client`を使用
- `create_response()`メソッドでLLM呼び出し
- Azure OpenAI / 標準OpenAI両対応
- 設定: `ResolvedAISettings`から読み込み

**再利用可能な部分**:

- `AIClient`をそのまま使用可能
- `ResolvedAISettings`の設定共有も可能

**拡張が必要な部分**:

- 会話履歴を含むマルチターン対話（新規実装）
- ストリーミング応答（任意、P3以降）

### 1.2 tmux制御 (`gwt-core/src/tmux/`)

**現状のモジュール構成**:

```text
tmux/
├── mod.rs          # エクスポート
├── launcher.rs     # エージェント起動
├── pane.rs         # ペイン管理、send_keys
├── detector.rs     # 完了検出
├── poller.rs       # 状態ポーリング
└── session.rs      # セッション管理
```

**主要な関数**:

| 関数 | ファイル | 用途 |
|------|----------|------|
| `launch_agent_in_pane()` | launcher.rs | エージェント起動 |
| `send_keys()` | pane.rs | キー送信 |
| `capture_pane()` | - | 出力取得（未実装） |
| `infer_agent_status()` | pane.rs | 状態推定 |
| `AgentPane` | pane.rs | エージェントペイン構造体 |

**再利用可能な部分**:

- `launch_agent_in_pane()`: worktreeでのエージェント起動
- `send_keys()`: プロンプト送信
- `AgentPane`: サブエージェント状態管理
- `infer_agent_status()`: 完了検出の一部

**拡張が必要な部分**:

- `capture_pane()`: tmux capture-paneラッパー（新規）
- Claude Code Hook連携（detector.rs拡張）

### 1.3 TUIアーキテクチャ (`gwt-cli/src/tui/`)

**現状**:

- Elm Architecture: `Model`/`Msg`/`update`/`view`パターン
- `ratatui 0.29` + `crossterm 0.28`
- 複数画面: branch_list, wizard, settings, profiles, etc.
- キーバインド: `event.rs`で定義

**画面追加パターン**:

1. `screens/agent_mode.rs`を新規作成
2. `screens/mod.rs`でエクスポート追加
3. `app.rs`にモード切り替えロジック追加

**参考になる既存画面**:

- `wizard.rs`: 複数ステップのUI
- `ai_wizard.rs`: AI関連のUI

### 1.4 セッション管理 (`gwt-core/src/config/`)

**現状**:

- `ToolSessionEntry`: セッション情報の構造体
- JSON形式でファイル保存
- `save_session_entry()`: 保存関数

**再利用可能な部分**:

- JSON永続化パターン
- ファイルパス管理

**新規実装が必要な部分**:

- エージェントモード専用のセッション構造体
- `~/.gwt/sessions/`ディレクトリ管理

## 2. 技術的決定

### 2.1 マスターエージェント実装

**決定**: 既存`AIClient`を使用

**理由**:

- 追加の依存関係不要
- API設定の共有が容易
- 既にテスト済み

**実装方針**:

```rust
// gwt-core/src/agent/master.rs
pub struct MasterAgent {
    client: AIClient,
    conversation: Conversation,
    system_prompt: String,
}

impl MasterAgent {
    pub fn chat(&mut self, user_message: &str) -> Result<String, AgentError>;
    pub fn plan_tasks(&mut self, request: &str) -> Result<Vec<Task>, AgentError>;
}
```

### 2.2 会話履歴管理

**決定**: serde_json + ファイル永続化

**理由**:

- 既存パターンとの整合性
- シンプルな実装
- 人間が読める形式

**データ形式**:

```json
{
  "session_id": "uuid",
  "created_at": "ISO8601",
  "messages": [
    {"role": "user", "content": "..."},
    {"role": "assistant", "content": "..."}
  ],
  "tasks": [...]
}
```

### 2.3 タスク分割

**決定**: LLMプロンプトエンジニアリング

**理由**:

- 追加ライブラリ不要
- 柔軟性が高い
- LLMの判断に任せる仕様と合致

**プロンプト設計**:

```text
You are a task planning assistant. Given a user request, break it down into:
1. Independent sub-tasks that can be executed in parallel
2. Dependent sub-tasks that must be executed sequentially

Output format (JSON):
{
  "tasks": [
    {
      "id": "task-1",
      "name": "...",
      "description": "...",
      "dependencies": [],
      "worktree_strategy": "new" | "shared"
    }
  ]
}
```

### 2.4 完了検出

**決定**: Claude Code Hook + tmux複合方式

**Claude Code完了検出**:

- Claude Code Hookの`Stop`イベントを監視
- 既存の`detector.rs`を拡張

**他エージェント完了検出（複合方式）**:

1. **プロセス終了**: `pane_dead`または`is_process_running()`
2. **出力パターン**: `capture-pane`で特定パターン検出
3. **アクティビティ監視**: 60秒間出力なしで停止と判定

**実装**:

```rust
pub enum CompletionSource {
    Hook,           // Claude Code Hook経由
    ProcessExit,    // プロセス終了
    OutputPattern,  // 出力パターン一致
    IdleTimeout,    // アイドルタイムアウト
}

pub fn detect_completion(agent: &SubAgent) -> Option<CompletionSource>;
```

### 2.5 サブエージェント指示

**決定**: `tmux send-keys`でプロンプト送信

**理由**:

- 既存の`send_keys()`関数を再利用
- 任意のエージェントに対応可能

**指示フォーマット**:

```text
{task_description}

When you have completed this task, please type 'q' to exit.
```

## 3. 制約と依存関係

### 3.1 tmux要件

- **最小バージョン**: tmux 3.0+（`-e`オプション使用）
- **検出方法**: `tmux -V`コマンドで確認
- **エラー処理**: tmux未検出時はエージェントモード無効化

### 3.2 Claude Code Hook連携

**Hook設定ファイル**: `~/.claude/settings.json`

```json
{
  "hooks": {
    "Stop": [
      {
        "type": "command",
        "command": "gwt hook-notify stop $SESSION_ID"
      }
    ]
  }
}
```

**gwt側の実装**:

- `gwt hook-notify`サブコマンド追加
- Unix socket / ファイル通知で完了を伝達

### 3.3 既存AI要約機能との統合

**共有する設定**:

- `ai.endpoint`: API エンドポイント
- `ai.model`: モデル名
- `ai.api_key`: APIキー（環境変数経由）

**独立する設定**:

- `agent.system_prompt`: マスターエージェント用システムプロンプト
- `agent.session_dir`: セッション保存ディレクトリ

## 4. 未解決事項

### 4.1 エッジケースの対処

| ケース | 対処方針 |
|--------|----------|
| tmux切断時の復旧 | セッション永続化から復元、実行中タスクは再起動 |
| 同一ファイル同時編集 | コンフリクト検出はマージ時（PR作成時）に実施 |
| LLMタイムアウト | リトライ3回、失敗時はユーザーに通知 |
| 削除されたworktree参照 | セッション復元時にworktree存在確認、なければスキップ |

### 4.2 将来の拡張

- ストリーミング応答（P3以降）
- 並列タスク数の制限設定（必要に応じて）
- サブエージェント種別の自動検出

## 5. 結論

既存のコードベースを最大限活用し、以下の方針で実装を進める:

1. **AIClient再利用**: 新規HTTPクライアント実装は不要
2. **tmux制御拡張**: `capture_pane`追加とHook連携のみ
3. **TUI画面追加**: 既存パターンに従って`agent_mode.rs`を新規作成
4. **セッション永続化**: JSON形式で`~/.gwt/sessions/`に保存

技術的なリスクは限定的であり、既存の実績あるコンポーネントを活用することで、安定した実装が可能。
