# 実装計画: GitView画面

**仕様ID**: `SPEC-1ea18899` | **日付**: 2026-02-02 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-1ea18899/spec.md` からの機能仕様

## 概要

ブランチ一覧画面で`v`キーを押すと、選択中ブランチのgit状態詳細（Files/Commits/PRリンク等）を表示する専用画面（GitView）を実装する。同時に現在のDetailsパネルを削除し、Branches + Sessionの2ペイン構成に変更する。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, serde, serde_json, chrono
**ストレージ**: メモリキャッシュ（GitViewCache）
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux/macOS/Windows（ターミナル環境）
**プロジェクトタイプ**: 単一プロジェクト（Cargoワークスペース）
**パフォーマンス目標**: キャッシュ済みの場合100ms以内で画面表示
**制約**: 既存のElmアーキテクチャ準拠、ASCIIアイコンのみ
**スケール/範囲**: ブランチ数100程度、ファイル数50程度を想定

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

| 原則 | 状態 | 備考 |
|------|------|------|
| I. シンプルさの追求 | ✅ | 垂直スタック構成、フラットリスト表示でシンプルに |
| II. テストファースト | ✅ | spec.md確定後に実装、TDD遵守 |
| III. 既存コードの尊重 | ✅ | 既存のScreen/Message/バックグラウンドパターンを踏襲 |
| IV. 品質ゲート | ✅ | cargo test/clippy/fmt通過を必須とする |
| V. 自動化の徹底 | ✅ | Conventional Commits遵守 |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-1ea18899/
├── spec.md              # 機能仕様（確定済み）
├── plan.md              # このファイル
├── research.md          # フェーズ0出力
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
├── contracts/           # フェーズ1出力（該当なし）
└── tasks.md             # フェーズ2出力
```

### ソースコード（リポジトリルート）

```text
crates/gwt-cli/src/tui/
├── app.rs               # Screen enum, Message enum, Model に GitView 追加
├── screens/
│   ├── branch_list.rs   # Detailsパネル削除、2ペイン化
│   ├── git_view.rs      # 【新規】GitView画面の実装
│   └── mod.rs           # git_view モジュール追加
└── ...

crates/gwt-core/src/git/
├── mod.rs               # 既存のgit操作
└── ...                  # 必要に応じて拡張
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-1ea18899/research.md`

### 調査項目

1. **既存のコードベース分析** ✅
   - Screen enum: `/app.rs` L526-543 に13画面定義済み
   - Message enum: `/app.rs` L546-598 にNavigateTo等定義済み
   - バックグラウンド処理: prepare → build → apply の3段階パターン
   - マウスイベント: handle_*_mouse() パターン、LinkRegion でURL検出

2. **技術的決定** ✅
   - 画面追加: Screen::GitView を enum に追加
   - 状態管理: GitViewState 構造体を Model に追加
   - キャッシュ: HashMap<String, GitViewData> で全ブランチをキャッシュ

3. **制約と依存関係** ✅
   - Elmアーキテクチャ準拠（update/view分離）
   - 既存の BranchItem から情報取得
   - gwt-core の Repository 経由でgitコマンド実行

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-1ea18899/data-model.md`
- `specs/SPEC-1ea18899/quickstart.md`

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要エンティティ:
- **GitViewState**: 画面状態（対象ブランチ、選択位置、展開状態）
- **GitViewCache**: 全ブランチのキャッシュ（HashMap）
- **GitViewData**: 1ブランチ分のgit情報
- **FileEntry**: ファイル情報（パス、ステータス、diff）
- **CommitEntry**: コミット情報（hash、message、author、files）

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの実装手順:
- Screen/Message enum への追加方法
- GitViewState の初期化
- render関数の実装パターン
- テストの書き方

### 1.3 契約/インターフェース

**ディレクトリ**: `contracts/` - 該当なし（内部モジュールのため）

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-1ea18899/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：

1. **P1: US1 + US4** - GitView画面基本実装 + Detailsパネル削除
2. **P2: US2** - PRリンクのブラウザ起動
3. **P2: US3** - ワークツリーなしブランチ対応

### 独立したデリバリー

| マイルストーン | 内容 | 検証可能な成果 |
|--------------|------|--------------|
| M1 | GitView画面の骨格 + 画面遷移 | `v`キーで空のGitView画面が開く |
| M2 | Detailsパネル削除 + 2ペイン化 | ブランチ一覧が2ペイン表示 |
| M3 | ヘッダー表示 | ブランチ名/PR/ahead-behind表示 |
| M4 | Filesセクション | ファイル一覧 + diff展開 |
| M5 | Commitsセクション | コミット一覧 + 詳細展開 |
| M6 | キャッシュ実装 | バックグラウンド事前取得 |
| M7 | マウス対応 | PRリンククリック |

## テスト戦略

- **ユニットテスト**: GitViewState の状態遷移、FileEntry/CommitEntry のパース
- **統合テスト**: 画面遷移（BranchList ↔ GitView）、キャッシュ更新
- **手動テスト**: キーバインド動作、マウスクリック、大量ファイル時の表示

## リスクと緩和策

### 技術的リスク

1. **大量ファイル時のパフォーマンス**
   - **緩和策**: 20件制限 + Show more、diff 50行制限

2. **バックグラウンドキャッシュのメモリ使用量**
   - **緩和策**: ブランチ数に応じた制限、必要に応じてLRUキャッシュ検討

### 依存関係リスク

1. **既存のDetailsパネル削除による影響**
   - **緩和策**: 段階的に移行、テストで既存機能の継続動作を確認

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
