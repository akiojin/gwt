# 実装計画: カスタムコーディングエージェント登録機能

**仕様ID**: `SPEC-71f2742d` | **日付**: 2026-01-26 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-71f2742d/spec.md` からの機能仕様

## 概要

ユーザーがビルトインエージェント（Claude Code, Codex CLI, Gemini CLI, OpenCode）に加えて、独自のコーディングエージェントを `~/.gwt/tools.json` に定義して利用できる機能。登録されたカスタムエージェントは Wizard のエージェント選択画面に表示され、ビルトインと同様に起動できる。

**技術アプローチ**:

- 新規 `tools.rs` モジュールで JSON 設定管理
- 既存 `CodingAgent` enum を拡張せず、統一エージェントリストで対応
- タブ切り替えで設定画面（カスタムエージェント管理 + Profile）を追加

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, serde, serde_json, chrono, directories
**ストレージ**: ファイル（~/.gwt/tools.json, .gwt/tools.json）
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux, macOS, Windows (cross-platform CLI)
**プロジェクトタイプ**: 単一 Rust ワークスペース
**パフォーマンス目標**: Wizard 開始時の読み込み < 100ms
**制約**: 追加の外部依存なし
**スケール/範囲**: カスタムエージェント数十個程度

## 原則チェック

*ゲート: constitution.md に基づく*

| 原則 | 状態 | 備考 |
|------|------|------|
| シンプルさの追求 | ✅ | 最小限の新規ファイル、既存パターン活用 |
| テストファースト | ✅ | TDD必須、tasks.md でテスト先行 |
| 既存コードの尊重 | ✅ | wizard.rs, settings.rs を拡張 |
| 品質ゲート | ✅ | clippy, fmt, commitlint 遵守 |
| 自動化の徹底 | ✅ | Conventional Commits 対応 |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-71f2742d/
├── plan.md              # このファイル
├── research.md          # フェーズ0出力（完了）
├── data-model.md        # フェーズ1出力（完了）
├── quickstart.md        # フェーズ1出力（完了）
├── checklists/
│   └── requirements.md  # 要件チェックリスト（完了）
└── tasks.md             # フェーズ2出力（/speckit.tasks で生成）
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-core/src/
│   └── config/
│       ├── mod.rs           # tools モジュール公開追加
│       └── tools.rs         # 新規: ToolsConfig, CustomCodingAgent
└── gwt-cli/src/
    ├── main.rs              # 修正: カスタムエージェント起動対応
    └── tui/
        ├── app.rs           # 修正: Settings スクリーン追加
        └── screens/
            ├── wizard.rs    # 修正: カスタムエージェント表示
            └── settings.rs  # 新規: 設定画面
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-71f2742d/research.md` ✅ 完了

### 調査結果サマリ

1. **既存のコードベース分析**
   - CodingAgent enum は固定4種、拡張困難
   - 履歴保存は String ID で対応済み
   - 設定読み込みは figment ベース

2. **技術的決定**
   - tools.json 専用モジュール新規作成
   - 統一エージェントリストで表示
   - タブ切り替えで設定画面追加

3. **制約と依存関係**
   - 追加の外部依存なし
   - 既存 CodingAgent との後方互換性維持

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:

- `specs/SPEC-71f2742d/data-model.md` ✅ 完了
- `specs/SPEC-71f2742d/quickstart.md` ✅ 完了

### 1.1 データモデル設計

**ファイル**: `data-model.md` ✅

主要なエンティティ:

- **ToolsConfig**: tools.json 全体構造（version, customCodingAgents）
- **CustomCodingAgent**: 個別エージェント定義
- **AgentType**: 実行タイプ（Command/Path/Bunx）
- **ModeArgs**: モード別引数
- **ModelDef**: モデル定義
- **AgentEntry**: ビルトイン/カスタム統一表現

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md` ✅

開発者向けガイド:

- セットアップ手順
- TDD サイクル
- よくある操作
- トラブルシューティング

### 1.3 契約/インターフェース

**該当なし**: 外部 API なし、内部モジュール間のみ

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-71f2742d/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：

1. **P1**: tools.json 読み込み、エージェント起動、TUI 登録
2. **P2**: タブ統合、モデル/バージョン、履歴統合

### 独立したデリバリー

- ストーリー1+2を完了 → 基本的なカスタムエージェント利用可能
- ストーリー3を追加 → TUI から設定可能
- ストーリー4+5+6を追加 → フル機能

### 実装フェーズ

1. **データ層** (P1-Story1)
   - `gwt-core/src/config/tools.rs` 新規作成
   - ToolsConfig, CustomCodingAgent 構造体
   - 読み込み・マージ・バリデーション

2. **表示層** (P1-Story1)
   - `wizard.rs` でカスタムエージェント表示
   - ビルトイン後にセパレータ＋カスタム

3. **起動層** (P1-Story2)
   - `main.rs` でカスタムエージェント起動
   - type 別分岐（command/path/bunx）

4. **設定UI** (P1-Story3)
   - `settings.rs` 新規作成
   - カスタムエージェント CRUD

5. **タブ統合** (P2-Story4)
   - `app.rs` でタブ切り替え拡張

6. **詳細機能** (P2-Story5,6)
   - モデル選択、バージョン取得
   - 履歴統合（既存構造活用）

## テスト戦略

- **ユニットテスト**: ToolsConfig パース、バリデーション、マージロジック
- **統合テスト**: Wizard でのカスタムエージェント表示、起動
- **エッジケーステスト**: JSON パースエラー、未インストールコマンド、ID 重複

## リスクと緩和策

### 技術的リスク

1. **CodingAgent enum との互換性破壊**
   - **緩和策**: enum は変更せず、統一リストで対応

2. **Wizard 状態管理の複雑化**
   - **緩和策**: AgentEntry 抽象で単純化

### 依存関係リスク

1. **bunx 未インストール時の type: "bunx"**
   - **緩和策**: グレーアウト表示で対応

2. **カスタムコマンド未インストール**
   - **緩和策**: "Not installed" 表示で対応

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
