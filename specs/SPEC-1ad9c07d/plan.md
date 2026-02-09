<!-- markdownlint-disable MD013 -->
# 実装計画: エージェント起動ウィザード統合 - AIBranchSuggest

**仕様ID**: `SPEC-1ad9c07d` | **日付**: 2026-02-08 | **仕様書**: [spec.md](spec.md)
**入力**: `/specs/SPEC-1ad9c07d/spec.md` からの機能仕様

## 概要

エージェント起動ウィザードにAIBranchSuggestステップを追加する。IssueSelectとBranchNameInputの間に配置し、AI設定が有効な場合にブランチ目的入力→AI候補生成→候補選択→BranchNameInput事前入力のフローを実装する。AI設定が無効な場合は従来フローを維持する。

既存のAIClient（OpenAI互換API）と非同期パターン（thread::spawn + mpsc::channel）を活用し、wizard.rsのステップ遷移パターンに準拠して実装する。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, reqwest (blocking), serde_json, chrono, tracing
**ストレージ**: N/A（メモリ内状態のみ）
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux, macOS, Windows (CLI)
**プロジェクトタイプ**: 単一（Cargoワークスペース）
**パフォーマンス目標**: AI API呼び出し中もUIが応答すること（非ブロッキング）
**制約**: AI API呼び出しは外部サービスに依存、CLIテキストは英語のみ
**スケール/範囲**: 単一ユーザーCLIツール

## 原則チェック

*ゲート: フェーズ0の調査前に合格。フェーズ1の設計後に再チェック済み。*

| 原則 | 状態 | 説明 |
|------|------|------|
| I. シンプルさの追求 | ✅ 合格 | 既存パターンを踏襲、新規依存なし |
| II. テストファースト | ✅ 計画済み | TDDで実装（spec → test → impl） |
| III. 既存コードの尊重 | ✅ 合格 | wizard.rs/app.rsの既存コード改修が中心、新規ファイル不要 |
| IV. 品質ゲート | ✅ 計画済み | clippy/fmt/testを全パス |
| V. 自動化の徹底 | ✅ N/A | リリースワークフローに影響なし |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-1ad9c07d/
├── plan.md              # このファイル
├── research.md          # フェーズ0出力
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
├── contracts/           # フェーズ1出力
│   └── ai-branch-suggest-api.md
└── tasks.md             # フェーズ2出力（/speckit.tasks で生成）
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-cli/src/tui/
│   ├── screens/wizard.rs   # 主要変更: WizardStep/WizardState/render/handlers
│   └── app.rs              # 変更: 非同期チャネル、Enter/Esc/charハンドラ
└── gwt-core/src/
    ├── ai/client.rs        # 既存活用: AIClient::create_response()
    └── agent/worktree.rs   # 既存活用: sanitize_branch_name()
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存のコードパターンを理解し、技術的決定を下す

**出力**: [research.md](research.md) ✅ 完了

### 調査結果サマリ

1. **ウィザードアーキテクチャ**: WizardStep enum + WizardState struct、next_step()/prev_step()で遷移管理
2. **非同期パターン**: thread::spawn + mpsc::channel + try_recv（ai_wizard.rsで実績あり）
3. **AI Client**: ChatMessage + create_response() → Result\<String, AIError\>
4. **ブランチ名正規化**: sanitize_branch_name() → 小文字化、特殊文字除去、64文字上限
5. **AI設定アクセス**: active_ai_enabled() / active_ai_settings() がapp.rsに存在

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:

- [data-model.md](data-model.md) ✅ 完了
- [quickstart.md](quickstart.md) ✅ 完了
- [contracts/ai-branch-suggest-api.md](contracts/ai-branch-suggest-api.md) ✅ 完了

### 1.1 データモデル設計

- **AIBranchSuggestPhase** enum: Input, Loading, Select, Error の4状態
- **WizardState追加フィールド**: ai_enabled, ai_branch_phase, ai_branch_input, ai_branch_cursor, ai_branch_suggestions, ai_branch_selected, ai_branch_error
- **BranchType::from_prefix()**: プレフィックスからBranchTypeと名前を分離
- **AiBranchSuggestUpdate**: 非同期結果の受け渡し構造体

### 1.2 クイックスタートガイド

開発者向けのセットアップ、ビルド、テスト、デバッグ手順を記載。

### 1.3 契約/インターフェース

- AI APIリクエスト/レスポンスのJSON形式
- 各フェーズのイベントハンドリング仕様
- レンダリング仕様（入力/ローディング/選択/エラー各フェーズ）

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-1ad9c07d/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

1. **P1**: AIBranchSuggestステップのコア実装（US5, US6）
   - WizardStep enumへのバリアント追加
   - WizardStateへのフィールド追加
   - next_step()/prev_step()の更新
   - 入力/選択ハンドラ
   - レンダリング関数
   - 非同期AI呼び出し

2. **P1**: AI無効時のスキップ（US6）
   - ai_enabled フラグの設定
   - 条件分岐によるスキップ

3. **P2**: エラーハンドリングとフォールバック（US7）
   - エラーフェーズの実装
   - ローディング中キャンセル
   - フォールバックフロー

### 独立したデリバリー

- US6完了 → AI無効時の後方互換性が保証（既存テスト通過）
- US5完了 → AIブランチ名提案のコア機能が利用可能
- US7完了 → エラー耐性のある完全な機能

## テスト戦略

- **ユニットテスト**: WizardState のステップ遷移（next_step/prev_step）、BranchType::from_prefix()、入力ハンドラ、選択ハンドラ、AIレスポンスパース
- **統合テスト**: wizard.rs内の既存テストパターンに準拠。AIクライアントはモック不要（ユニットテストでカバー）
- **テストケース例**:
  - AI有効時: IssueSelect → next_step → AIBranchSuggest
  - AI無効時: IssueSelect → next_step → BranchNameInput
  - AIBranchSuggest → prev_step → IssueSelect (gh CLI有効時)
  - AIBranchSuggest → prev_step → BranchTypeSelect (gh CLI無効時)
  - 候補選択後: branch_type と new_branch_name が正しく設定される
  - from_prefix("feature/add-login") → Some((Feature, "add-login"))
  - from_prefix("unknown/name") → None

## リスクと緩和策

### 技術的リスク

1. **AIレスポンスの不確実性**: LLMの出力が期待するJSON形式と異なる場合がある
   - **緩和策**: JSONパースに失敗した場合はErrorフェーズに遷移し、手動入力にフォールバック。レスポンスからJSON部分を抽出するロバストなパースを実装

2. **wizard.rs のexhaustive match**: WizardStep に新バリアントを追加すると、全matchが更新必要
   - **緩和策**: コンパイラが未処理のmatchを検出するため、漏れは発生しない。事前に全match箇所を洗い出し済み

3. **非同期チャネルのライフサイクル**: ウィザードを閉じた後もスレッドが残る可能性
   - **緩和策**: 既存パターン（ai_wizard_rx = None でチャネル破棄）と同様に処理。スレッドはsend失敗で自然終了

### 依存関係リスク

1. **AI API可用性**: 外部APIに依存するため、ネットワーク障害時に機能しない
   - **緩和策**: Escスキップ + エラーフォールバックで手動入力に切り替え可能。AI無効時は完全にバイパス

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
