# 実装計画: エージェントモード

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-07 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-ba3f610c/spec.md` からの機能仕様

## 概要

マスターエージェント（gwt内蔵LLM）がユーザーと対話し、Spec Kitワークフロー（仕様策定→計画→タスク生成）を自動実行した上で、複数のサブエージェント（Claude Code等）をtmux制御でオーケストレーションする。Spec Kit機能をLLMプロンプトテンプレートとしてgwtに内蔵し、エージェントモードとブランチモードの両方から利用可能にする。イベント駆動のオーケストレーションループで、タスク分割→WT作成→サブエージェント起動→完了検出→テスト検証→PR作成→クリーンアップの全フローを自律実行する。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono, tracing, tracing-appender, uuid
**ストレージ**: ファイルシステム（`~/.gwt/sessions/` JSON形式、`specs/SPEC-XXXXXXXX/` Spec Kit成果物）
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux / macOS（tmux必須環境）
**プロジェクトタイプ**: Rustワークスペース（gwt-core + gwt-cli）
**パフォーマンス目標**: モード切り替え1秒以内、LLM初回応答5秒以内、完了検出10秒以内
**制約**: tmux必須、OpenAI互換APIのみ、サブエージェント全自動モード起動
**スケール/範囲**: 1セッション最大10タスク並列、キュー方式で複数セッション順次処理

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

| 原則 | 状態 | 備考 |
|------|------|------|
| I. シンプルさの追求 | PASS | イベント駆動ループ + LLMプロンプトテンプレート。ステートマシンは明示的で単純 |
| II. テストファースト | PASS | Spec Kit内蔵によりSDD/TDDフローを自動化。サブエージェントにもTDD指示 |
| III. 既存コードの尊重 | PASS | 既存のagent/、tmux/、ai/モジュールを拡張。新規ファイルは最小限 |
| IV. 品質ゲート | PASS | テスト検証フェーズ（最大3回リトライ）、commitlint準拠PR生成 |
| V. 自動化の徹底 | PASS | Conventional Commits形式PR自動生成、クリーンアップ自動化 |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-ba3f610c/
├── spec.md              # 機能仕様（更新済み）
├── plan.md              # このファイル
├── research.md          # フェーズ0出力
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
└── tasks.md             # フェーズ2出力
```

### ソースコード（リポジトリルート）

```text
crates/gwt-core/src/
├── agent/
│   ├── mod.rs              # AgentManager拡張
│   ├── master.rs           # MasterAgent拡張（イベント駆動ループ）
│   ├── session.rs          # AgentSession拡張（永続化・復元）
│   ├── task.rs             # Task拡張（テスト検証状態）
│   ├── orchestrator.rs     # 【新規】OrchestratorLoop（イベント駆動コア）
│   ├── scanner.rs          # 【新規】RepositoryScanner（ディープスキャン）
│   ├── prompt_builder.rs   # 【新規】PromptBuilder（アダプティブプロンプト生成）
│   └── session_store.rs    # 【新規】SessionStore（永続化・復元）
├── speckit/
│   ├── mod.rs              # 【新規】Spec Kit内蔵モジュール
│   ├── templates.rs        # 【新規】LLMプロンプトテンプレート（include_str!）
│   ├── specify.rs          # 【新規】仕様策定ロジック
│   ├── plan.rs             # 【新規】計画策定ロジック
│   ├── tasks.rs            # 【新規】タスク生成ロジック
│   ├── clarify.rs          # 【新規】曖昧さ解消ロジック
│   └── analyze.rs          # 【新規】整合性分析ロジック
├── tmux/
│   ├── launcher.rs         # 拡張（全自動モードフラグ対応）
│   ├── pane.rs             # 拡張（send-keys完了確認）
│   └── poller.rs           # 拡張（イベント駆動通知）
└── ai/
    └── client.rs           # 拡張（コスト追跡、トークン数推定）

crates/gwt-cli/src/tui/
├── screens/
│   ├── agent_mode.rs       # 拡張（チャットのみUI、ステータスバー）
│   └── speckit_wizard.rs   # 【新規】ブランチモード用Spec Kitウィザード
└── app.rs                  # 拡張（Esc中断、キュー管理、Spec Kitショートカット）

crates/gwt-core/src/speckit/templates/
├── specify.md              # 【新規】仕様策定プロンプトテンプレート
├── plan.md                 # 【新規】計画策定プロンプトテンプレート
├── tasks.md                # 【新規】タスク生成プロンプトテンプレート
├── clarify.md              # 【新規】曖昧さ解消プロンプトテンプレート
└── analyze.md              # 【新規】整合性分析プロンプトテンプレート
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存コードベースの統合ポイントと技術的決定を確認する

**出力**: `specs/SPEC-ba3f610c/research.md`

### 調査項目

1. **既存のコードベース分析**
   - gwt-core: agent/, tmux/, ai/ モジュールの現在の実装状態
   - gwt-cli: agent_mode.rs, app.rs のTUI状態管理パターン
   - 既存のSpec Kitスクリプト（.specify/）のプロンプト構造

2. **技術的決定**
   - イベント駆動ループの実装: mpsc::channel（既存パターン）を活用
   - セッション永続化: serde_json + atomic file write（既存のログパターン流用）
   - Spec Kit LLMプロンプト: include_str!マクロでコンパイル時埋め込み
   - ディープスキャン: git ls-tree + ファイル読み取りで実装

3. **制約と依存関係**
   - tmux 3.0+必須（send-keys, capture-pane, list-panes）
   - OpenAI互換API（既存AIClientを共有）
   - gh CLI（PR作成に必要）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-ba3f610c/data-model.md`
- `specs/SPEC-ba3f610c/quickstart.md`

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要エンティティ:
- Session（セッション全体: 会話+タスク+WT+状態）
- Task（個別タスク: 状態遷移、依存関係、WT割当）
- SubAgent（サブエージェント: tmuxペイン、状態、完了検出方式）
- Conversation（対話履歴: メッセージ配列、要約圧縮）
- SessionQueue（セッションキュー: FIFO順序管理）
- SpecKitArtifact（Spec Kit成果物: spec.md/plan.md/tasks.md参照）
- OrchestratorEvent（イベント: 完了/失敗/入力/タイマー）

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド:
- エージェントモードの開発環境セットアップ
- テスト実行手順
- Spec Kitテンプレートの編集方法

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-ba3f610c/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

MVP（複数タスクE2E）を最優先とし、段階的に機能を追加:

1. **P1 Phase A（基盤）**: Spec Kit内蔵 + チャットUI刷新 + ディープスキャン
2. **P1 Phase B（コア）**: タスク分割 + WT作成 + サブエージェント起動 + 完了検出
3. **P1 Phase C（E2E）**: イベント駆動ループ + 並列実行 + 計画承認フロー
4. **P2（品質）**: テスト検証 + PR作成 + 失敗ハンドリング + セッション永続化 + クリーンアップ
5. **P2（UX）**: ドライラン + ライブ介入 + 定期報告 + コスト可視化 + セッション継続判断
6. **P3**: コンテキスト圧縮

### 独立したデリバリー

- Phase A完了: Spec Kit内蔵 + チャットUI → ブランチモードからSpec Kit利用可能
- Phase B完了: 単一タスクの自律実行 → 最小限のエージェントモード
- Phase C完了: 複数タスクE2E → **MVP達成**
- P2完了: 本番品質のエージェントモード

## テスト戦略

- **ユニットテスト**: 各モジュール（orchestrator, scanner, prompt_builder, session_store, speckit/*）に対してcargo testで実行
- **統合テスト**: MasterAgent + tmux連携のモックテスト（tmuxコマンドのモック）
- **E2Eテスト**: 実tmux環境でのフルフロー検証（CI環境での実行は困難なため手動テスト併用）
- **TUIテスト**: AgentModeState の状態遷移テスト（既存パターンに準拠）

## リスクと緩和策

### 技術的リスク

1. **LLMの構造化出力の不安定性**: OpenAI互換APIからのJSON出力がパース失敗する
   - **緩和策**: 最大2回のリトライ + フォールバック（単一タスク化）。プロンプトに明示的なJSON例を含める

2. **tmux send-keys の信頼性**: 長いプロンプトをsend-keysで送信する際の文字化け・切り詰め
   - **緩和策**: プロンプトをファイルに書き出し、`cat file | tmux load-buffer`方式を検討

3. **イベント駆動ループのデッドロック**: 複数イベントの同時発生による状態不整合
   - **緩和策**: mpsc::channelで順序保証。イベントハンドラは排他的に実行

### 依存関係リスク

1. **Claude Code Hook API**: Hook機能のAPIが変更される可能性
   - **緩和策**: tmux複合方式をフォールバックとして常に保持

2. **gh CLI**: PR作成にgh CLIが必要だが、未インストール環境がある
   - **緩和策**: gh CLI未検出時はPR作成をスキップし、ユーザーに手動PR作成を案内

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義
3. ✅ フェーズ2完了: タスク生成（100タスク、8フェーズ）
4. ⏭️ `/speckit.implement` で実装を開始
