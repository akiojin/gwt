# 実装計画: エージェントモード

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-01-22 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-ba3f610c/spec.md` からの機能仕様

## 概要

gwtにエージェントモードを追加し、マスターエージェント（LLM）がユーザーと対話しながら、タスクを自律的に分割・計画し、複数のサブエージェント（Claude Code等）をtmux制御でオーケストレーションする機能を実現する。

主要な技術的アプローチ:

- 既存AIクライアント（`gwt-core/src/ai/client.rs`）を再利用してマスターエージェントを実装
- 既存tmux制御（`gwt-core/src/tmux/`）を拡張してサブエージェント管理を実装
- 既存TUIアーキテクチャ（ratatui + Elm Architecture）にエージェントモード画面を追加
- セッション永続化は既存の`ToolSessionEntry`パターンを参考に実装

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono, tracing
**ストレージ**: ファイルシステム (`~/.gwt/sessions/` - JSON形式)
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux/macOS (tmux環境必須)
**プロジェクトタイプ**: 単一 - Rustワークスペース (gwt-cli + gwt-core)
**パフォーマンス目標**: モード切り替え1秒以内、初回応答5秒以内、完了検出10秒以内
**制約**: tmux環境必須、LLM APIコスト無制限（ユーザー責任）
**スケール/範囲**: 同時実行サブエージェント数は無制限（tmux/システムリソース制約のみ）

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

| 原則 | 状態 | 備考 |
|------|------|------|
| I. シンプルさの追求 | ✅ | 既存コンポーネント（AIClient, tmux）を最大限再利用 |
| II. テストファースト | ✅ | spec.md完了後、TDD実施予定 |
| III. 既存コードの尊重 | ✅ | 既存ai/, tmux/モジュールを拡張、新規ファイル最小化 |
| IV. 品質ゲート | ✅ | clippy/rustfmt/cargo test必須 |
| V. 自動化の徹底 | ✅ | Conventional Commits遵守 |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-ba3f610c/
├── plan.md              # このファイル
├── research.md          # フェーズ0出力
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
├── contracts/           # フェーズ1出力（該当なし - 内部API）
├── checklists/
│   └── requirements.md  # 品質チェックリスト
└── tasks.md             # フェーズ2出力（/speckit.tasksで生成）
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-cli/src/
│   └── tui/
│       ├── app.rs                    # 既存: モード切り替えロジック追加
│       └── screens/
│           ├── mod.rs                # 既存: agent_mode追加
│           └── agent_mode.rs         # 新規: エージェントモード画面
└── gwt-core/src/
    ├── agent/                        # 新規: エージェントモジュール
    │   ├── mod.rs                    # モジュールエクスポート
    │   ├── master.rs                 # マスターエージェント実装
    │   ├── task.rs                   # タスク管理
    │   ├── session.rs                # セッション永続化
    │   └── orchestrator.rs           # サブエージェントオーケストレーション
    ├── ai/
    │   └── client.rs                 # 既存: 変更なし（再利用）
    └── tmux/
        ├── mod.rs                    # 既存: 新機能エクスポート追加
        ├── launcher.rs               # 既存: 変更なし（再利用）
        ├── pane.rs                   # 既存: capture_pane追加
        └── detector.rs               # 既存: Hook完了検出拡張
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-ba3f610c/research.md`

### 調査項目

1. **既存のコードベース分析**
   - AIClient: `gwt-core/src/ai/client.rs` - OpenAI Responses API互換、reqwest blocking
   - tmux制御: `gwt-core/src/tmux/` - send_keys, capture_pane, AgentPane, 完了検出
   - TUI: `gwt-cli/src/tui/app.rs` - Elm Architecture, ratatui 0.29
   - セッション管理: `gwt-core/src/config/` - ToolSessionEntry, JSON永続化

2. **技術的決定**
   - マスターエージェント: 既存AIClientを使用（追加の依存関係なし）
   - 会話履歴管理: serde_json + ファイル永続化
   - タスク分割: LLMプロンプトエンジニアリング（追加ライブラリなし）
   - 完了検出: Claude Code Hook + tmux複合方式（既存detector.rs拡張）

3. **制約と依存関係**
   - tmux 3.0+必須（-eオプション使用）
   - 既存AI要約機能（SPEC-4b893dae）のAPI設定を共有
   - Claude Code Hook APIとの連携

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:

- `specs/SPEC-ba3f610c/data-model.md`
- `specs/SPEC-ba3f610c/quickstart.md`
- `specs/SPEC-ba3f610c/contracts/` （内部APIのため省略）

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティとその関係を定義：

- Session: エージェントモードセッション全体
- Task: 分割されたタスク単位
- SubAgent: サブエージェントインスタンス
- Conversation: 会話履歴

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：

- エージェントモジュールのセットアップ
- ローカル開発環境構築
- テスト実行方法

### 1.3 契約/インターフェース

**ディレクトリ**: `contracts/`

内部APIのため外部契約は不要。データモデルでRust構造体を定義。

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-ba3f610c/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：

1. **P1**: US-1〜US-4（モード切り替え、タスク分割、サブエージェント起動、完了検出）
2. **P2**: US-5〜US-7（成果物統合、失敗ハンドリング、セッション永続化）
3. **P3**: US-8（コンテキスト管理）

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能：

- Phase 1完了 → モード切り替えとマスターエージェント対話のMVP
- Phase 2完了 → サブエージェント起動と完了検出
- Phase 3完了 → セッション永続化と成果物統合

## テスト戦略

- **ユニットテスト**: 各モジュール（master.rs, task.rs, session.rs）の個別テスト
- **統合テスト**: TUIとの連携テスト（エージェントモード画面）
- **エンドツーエンドテスト**: tmuxモック環境での全体フロー検証
- **パフォーマンステスト**: モード切り替え時間、応答時間の計測

## リスクと緩和策

### 技術的リスク

1. **LLM応答の不確実性**: タスク分割結果が期待と異なる可能性
   - **緩和策**: プロンプトテンプレートの十分なテスト、フォールバック処理

2. **tmux完了検出の精度**: Claude Code以外のエージェントでは検出が不安定
   - **緩和策**: 複合方式（プロセス監視+出力パターン+アクティビティ）の組み合わせ

3. **コンテキストウィンドウ制限**: 大規模タスクでコンテキスト超過
   - **緩和策**: 要約圧縮機能（P3で実装）

### 依存関係リスク

1. **Claude Code Hook API**: 仕様変更の可能性
   - **緩和策**: 抽象化層を設け、Hook失敗時はtmux複合方式にフォールバック

2. **既存AI要約機能**: API設定共有の互換性
   - **緩和策**: AIClient APIを変更せず、設定読み込みのみ共有

## 次のステップ

1. ✅ フェーズ0開始: 調査と技術スタック確認
2. ✅ フェーズ0完了: research.md作成
3. ✅ フェーズ1: data-model.md / quickstart.md 作成
4. ✅ エージェントコンテキスト更新完了
5. ⏭️ `/speckit.tasks` を実行してタスクを生成
6. ⏭️ `/speckit.implement` で実装を開始
