# 実装計画: エラーポップアップ・ログ出力システム

**仕様ID**: `SPEC-e66acf66` | **日付**: 2026-01-22 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-e66acf66/spec.md` からの機能仕様

## 概要

エラー発生時にブランチ詳細画面ではなくポップアップダイアログで表示し、全エラーをログファイルに出力するシステム。エラーコード体系（GWT-EXXX形式）、サジェスチョン機能、複数エラーのキュー処理、キーボード/マウス操作に対応する。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, tracing, tracing-appender, serde_json, chrono, cli-clipboard
**ストレージ**: ファイル（gwt.jsonl.YYYY-MM-DD）
**テスト**: cargo test
**ターゲットプラットフォーム**: macOS, Linux, Windows
**プロジェクトタイプ**: 単一（Rustワークスペース）
**パフォーマンス目標**: ポップアップ表示100ms以内
**制約**: ASCII文字のみ（絵文字不可）、既存のErrorState/render_error関数を拡張
**スケール/範囲**: 単一ユーザーCLIアプリ

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

| 原則 | 状態 | 備考 |
|------|------|------|
| I. シンプルさの追求 | ✅ 合格 | 既存のErrorState/render_error/ログシステムを拡張 |
| II. テストファースト | ✅ 合格 | 仕様策定完了、TDD実装予定 |
| III. 既存コードの尊重 | ✅ 合格 | error.rs, logs.rs, logging/を改修 |
| IV. 品質ゲート | ✅ 合格 | cargo clippy, cargo fmt適用 |
| V. 自動化の徹底 | ✅ 合格 | Conventional Commits遵守 |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-e66acf66/
├── spec.md              # 機能仕様
├── plan.md              # このファイル
├── research.md          # フェーズ0出力
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
├── contracts/           # フェーズ1出力
├── checklists/
│   └── requirements.md  # 要件チェックリスト
└── tasks.md             # フェーズ2出力（/speckit.tasksで作成）
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-cli/src/tui/
│   ├── screens/
│   │   ├── error.rs     # エラーポップアップ（拡張対象）
│   │   └── logs.rs      # ログビューアー
│   └── app.rs           # メインアプリ状態
└── gwt-core/src/
    ├── logging/
    │   ├── logger.rs    # ログ初期化（拡張対象）
    │   └── reader.rs    # ログ読み込み
    └── error/           # 新規: エラーコード定義
        ├── mod.rs
        ├── codes.rs     # GWT-EXXX定義
        └── suggestions.rs # サジェスチョン
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-e66acf66/research.md`

### 調査項目

1. **既存のコードベース分析**
   - 現在のエラー表示: ErrorState, render_error関数
   - 既存のログシステム: tracing + JSON Lines + tracing-appender
   - 統合ポイント: app.rsのエラーハンドリング箇所

2. **技術的決定**
   - クリップボード: cli-clipboardクレート使用
   - エラーキュー: VecDeque<ErrorState>で実装
   - マウスイベント: crosstermのMouseEventで処理

3. **制約と依存関係**
   - 既存のrender_error関数を拡張（破壊的変更なし）
   - ログディレクトリ: ~/.gwt/logs/

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-e66acf66/data-model.md`
- `specs/SPEC-e66acf66/quickstart.md`
- `specs/SPEC-e66acf66/contracts/`

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要エンティティ:
- **ErrorCode**: GWT-EXXX形式のエラー識別子、カテゴリ、サジェスチョン
- **ErrorState**: 表示用エラー状態（拡張）
- **ErrorQueue**: 複数エラーのFIFOキュー
- **ErrorSeverity**: Error/Warning/Info

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向け:
- エラー発生のトリガー方法
- ログファイルの確認方法
- 新規エラーコード追加手順

### 1.3 契約/インターフェース

**ディレクトリ**: `contracts/`

- エラーコード一覧（GWT-E001〜）
- ログフォーマット（JSON Lines）
- クリップボードエクスポート形式

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-e66acf66/tasks.md`

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装:

1. **P0**: エラーポップアップ表示、ログ画面遷移、ログ出力
2. **P1**: クリップボードコピー、キュー処理、マウス操作、サジェスチョン

### 独立したデリバリー

1. P0完了 → 基本的なエラーポップアップとログ出力が動作
2. P1追加 → 完全な機能セット

## テスト戦略

- **ユニットテスト**: ErrorCode生成、ErrorQueue操作、サジェスチョン取得
- **統合テスト**: エラー発生→ポップアップ表示→ログ記録の一連フロー
- **手動テスト**: キーボード/マウス操作、画面遷移

## リスクと緩和策

### 技術的リスク

1. **クリップボード非対応環境**:
   - **緩和策**: エラーハンドリングで「Copy failed」表示

2. **ログファイル書き込み権限**:
   - **緩和策**: ディレクトリ自動作成、権限エラー時はstderrにフォールバック

### 依存関係リスク

1. **cli-clipboardクレートの互換性**:
   - **緩和策**: 代替クレート（arboard）を検討

## 次のステップ

1. ✅ フェーズ0完了: research.md作成済み
2. ✅ フェーズ1完了: data-model.md, quickstart.md, contracts/error-codes.md作成済み
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
