# クイックスタートガイド

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-07

## 開発環境セットアップ

### 前提条件

- Rust stable (2021 Edition)
- tmux 3.0+
- 少なくとも1つのコーディングエージェント（Claude Code / Codex / Gemini）
- gh CLI（PR作成機能を使用する場合）

### ビルドと実行

```bash
# ビルド
cargo build --release

# テスト実行
cargo test

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# フォーマット
cargo fmt

# 実行（tmux内で）
tmux
./target/release/gwt
```

### エージェントモードの起動

1. tmuxセッション内で`gwt`を起動
2. `Tab`キーでエージェントモードに切り替え
3. チャット入力欄に機能要求を自然言語で入力
4. マスターエージェントがSpec Kitワークフローを自動実行
5. 計画を確認し、承認（Enter or "y"）

## テスト実行手順

### ユニットテスト

```bash
# 全テスト
cargo test

# agent モジュールのみ
cargo test --lib agent

# 特定テスト
cargo test test_orchestrator_event_handling
```

### テスト対象モジュール

| モジュール | テスト内容 |
|-----------|----------|
| `agent::orchestrator` | イベント駆動ループの状態遷移 |
| `agent::scanner` | リポジトリスキャンのファイル検出 |
| `agent::prompt_builder` | プロンプト生成のテンプレート埋め込み |
| `agent::session_store` | JSON永続化・復元・atomic write |
| `speckit::*` | テンプレート読込・変数置換・成果物生成 |
| `tui::screens::agent_mode` | AgentModeState の状態遷移 |

### 統合テスト（手動）

tmux環境でのE2Eテストは自動化が困難なため、以下の手動テスト手順を使用:

1. tmux内でgwtを起動
2. エージェントモードに切り替え
3. テスト用タスクを入力（例: "Create hello.txt"）
4. 計画が提示されることを確認
5. 承認してサブエージェントが起動することを確認
6. タスク完了後にPRが作成されることを確認
7. クリーンアップ（WT削除・ブランチ削除）を確認

## Spec Kitテンプレートの編集方法

### テンプレートファイルの場所

```text
crates/gwt-core/src/speckit/templates/
├── specify.md    # 仕様策定プロンプト
├── plan.md       # 計画策定プロンプト
├── tasks.md      # タスク生成プロンプト
├── clarify.md    # 曖昧さ解消プロンプト
└── analyze.md    # 整合性分析プロンプト
```

### テンプレート変数

テンプレート内では`{{variable}}`形式の変数プレースホルダーを使用:

| 変数 | 説明 |
|-----|------|
| `{{user_request}}` | ユーザーの機能要求（自然言語） |
| `{{repository_context}}` | リポジトリスキャン結果 |
| `{{claude_md}}` | CLAUDE.md内容 |
| `{{existing_specs}}` | 既存スペック一覧 |
| `{{spec_content}}` | 生成されたspec.md内容 |
| `{{plan_content}}` | 生成されたplan.md内容 |
| `{{directory_tree}}` | ディレクトリ構造 |

### 編集後の反映

テンプレートは`include_str!`でコンパイル時に埋め込まれるため、変更後は再ビルドが必要:

```bash
cargo build --release
```

## ディレクトリ構成

### 新規作成ファイル

```text
crates/gwt-core/src/
├── agent/
│   ├── orchestrator.rs     # OrchestratorLoop（イベント駆動コア）
│   ├── scanner.rs          # RepositoryScanner（ディープスキャン）
│   ├── prompt_builder.rs   # PromptBuilder（アダプティブプロンプト生成）
│   └── session_store.rs    # SessionStore（永続化・復元）
└── speckit/
    ├── mod.rs              # Spec Kit内蔵モジュール
    ├── templates.rs        # LLMプロンプトテンプレート（include_str!）
    ├── specify.rs          # 仕様策定ロジック
    ├── plan.rs             # 計画策定ロジック
    ├── tasks.rs            # タスク生成ロジック
    ├── clarify.rs          # 曖昧さ解消ロジック
    ├── analyze.rs          # 整合性分析ロジック
    └── templates/
        ├── specify.md      # 仕様策定プロンプトテンプレート
        ├── plan.md         # 計画策定プロンプトテンプレート
        ├── tasks.md        # タスク生成プロンプトテンプレート
        ├── clarify.md      # 曖昧さ解消プロンプトテンプレート
        └── analyze.md      # 整合性分析プロンプトテンプレート
```

### 変更対象ファイル

```text
crates/gwt-core/src/
├── agent/
│   ├── mod.rs              # orchestrator等のpub mod追加
│   ├── master.rs           # イベント駆動ループ統合
│   ├── session.rs          # base_branch等のフィールド追加
│   ├── task.rs             # test_status等のフィールド追加
│   └── sub_agent.rs        # auto_mode_flagフィールド追加
├── tmux/
│   ├── launcher.rs         # 全自動モードフラグ対応
│   ├── pane.rs             # send-keys完了確認
│   └── poller.rs           # イベント駆動通知
├── ai/
│   └── client.rs           # コスト追跡、MAX_OUTPUT_TOKENS拡張
└── lib.rs                  # pub mod speckit 追加

crates/gwt-cli/src/tui/
├── screens/
│   ├── agent_mode.rs       # チャットのみUI、ステータスバー
│   └── speckit_wizard.rs   # 【新規】ブランチモード用Spec Kitウィザード
├── screens/mod.rs          # speckit_wizard追加
└── app.rs                  # Esc中断、キュー管理
```

## 関連ドキュメント

- [仕様書](./spec.md)
- [実装計画](./plan.md)
- [技術調査](./research.md)
- [データモデル](./data-model.md)
