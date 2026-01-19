# 実装計画: ブランチサマリーパネル

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-4b893dae/spec.md` からの機能仕様

## 概要

ブランチ選択画面のフッター領域を拡張し、選択中ブランチの詳細情報（コミットログ、変更統計、メタデータ、AIサマリー）を常時表示する情報パネルを実装する。既存の`Worktree:`表示を置き換え、12行固定のパネルとして表示する。

## 技術コンテキスト

**言語/バージョン**: Rust (Stable)
**主要な依存関係**: Ratatui, reqwest（新規）, serde_yaml, tokio
**ストレージ**: ファイル（YAML/TOML）、メモリキャッシュ
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux, macOS, Windows (CLI)
**プロジェクトタイプ**: 単一（Cargoワークスペース）
**パフォーマンス目標**: パネル更新200ms以内、コミットログ取得500ms以内
**制約**: ASCII文字のみ（絵文字禁止）、オフライン対応（AI機能除く）
**スケール/範囲**: ブランチ数1000件、コミット履歴5件表示

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

| 原則                  | 状態 | 備考                                   |
| --------------------- | ---- | -------------------------------------- |
| I. シンプルさの追求   | OK   | 既存パターン踏襲、最小限の新規実装     |
| II. テストファースト  | OK   | 仕様確定後にテスト作成                 |
| III. 既存コードの尊重 | OK   | branch_list.rs拡張、新規ファイル最小化 |
| IV. 品質ゲート        | OK   | clippy, cargo test通過必須             |
| V. 自動化の徹底       | OK   | Conventional Commits遵守               |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-4b893dae/
├── spec.md              # 機能仕様
├── plan.md              # このファイル
├── research.md          # 調査報告
├── data-model.md        # データモデル設計
├── quickstart.md        # 開発者向けガイド
├── contracts/           # API契約
│   └── openai-api.md    # OpenAI互換API仕様
├── checklists/
│   └── requirements.md  # 要件チェックリスト
└── tasks.md             # 実装タスク
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-core/src/
│   ├── git/
│   │   ├── repository.rs    # [変更] コミットログ・diff統計追加
│   │   └── commit.rs        # [新規] CommitEntry構造体
│   ├── config/
│   │   └── profile.rs       # [変更] AI設定追加
│   └── ai/
│       ├── mod.rs           # [新規] AIモジュール
│       ├── client.rs        # [新規] OpenAI互換APIクライアント
│       └── summary.rs       # [新規] サマリー生成ロジック
│
├── gwt-cli/src/tui/
│   ├── screens/
│   │   └── branch_list.rs   # [変更] パネル表示追加
│   └── components/
│       └── summary_panel.rs # [新規] サマリーパネルコンポーネント
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-4b893dae/research.md` ✅ 完了

### 調査結果サマリー

1. **既存のコードベース分析**
   - TUI: Ratatui、レイアウトはLayout::default()でVertical分割
   - Git操作: std::process::Commandでgitコマンドラップ
   - プロファイル: YAML形式、Profile構造体

2. **技術的決定**
   - コミットログ: `git log --oneline -n 5`
   - 変更統計: `git diff --shortstat` + 既存has_changes/has_unpushed
   - AI API: reqwestでOpenAI互換API呼び出し

3. **制約と依存関係**
   - SPEC-d2f4762aの安全性判定データを再利用
   - ASCII文字のみ（Ratatuiガイドライン）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-4b893dae/data-model.md` ✅ 完了
- `specs/SPEC-4b893dae/quickstart.md` ✅ 完了
- `specs/SPEC-4b893dae/contracts/openai-api.md` ✅ 完了

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要エンティティ:
- **BranchSummary**: パネル全体のデータ（コミットログ、統計、メタ、AIサマリー）
- **CommitEntry**: 個々のコミット情報
- **ChangeStats**: 変更統計
- **AISettings**: プロファイルに追加するAI設定

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド:
- 開発環境セットアップ
- テスト実行方法
- AI機能の設定方法

### 1.3 契約/インターフェース

**ディレクトリ**: `contracts/`

- OpenAI互換API仕様（エンドポイント、リクエスト/レスポンス形式）

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-4b893dae/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装:

| フェーズ | 優先度 | 内容                         |
| -------- | ------ | ---------------------------- |
| Phase 1  | P0     | パネル枠表示 + コミットログ  |
| Phase 2  | P0     | 変更統計（既存データ再利用） |
| Phase 3  | P1     | ブランチメタデータ           |
| Phase 4  | P2     | AI設定 + AIサマリー          |

### 独立したデリバリー

各フェーズは独立してテスト・デプロイ可能:
- Phase 1完了 → コミットログ表示のみのMVP
- Phase 2追加 → 統計情報付きパネル
- Phase 3追加 → メタデータ完備
- Phase 4追加 → AI機能付き完全版

## テスト戦略

- **ユニットテスト**: CommitEntry, ChangeStats, AISettings のパース・シリアライズ
- **統合テスト**: git log/diff コマンドの出力パース
- **TUIテスト**: パネルレンダリングのスナップショットテスト
- **モックテスト**: AI API呼び出しのモック

## リスクと緩和策

### 技術的リスク

1. **AI API依存**
   - **緩和策**: AIセクションを完全にオプショナル化、エラー時は非表示

2. **パフォーマンス低下**
   - **緩和策**: バックグラウンド取得、キャッシュ、遅延ロード

### 依存関係リスク

1. **SPEC-d2f4762aとの統合**
   - **緩和策**: 既存インターフェースを変更せず、データ参照のみ

2. **reqwest追加によるバイナリサイズ増加**
   - **緩和策**: feature flagでAI機能をオプショナル化

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ⏭️ フェーズ1: 設計ドキュメント作成
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
