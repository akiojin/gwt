# 実装計画: Docker Compose の Playwright noVNC を arm64 で起動可能にする

**仕様ID**: `[SPEC-925c010b]` | **日付**: 2026-01-17 | **仕様書**: [spec.md](spec.md)
**入力**: `/specs/SPEC-925c010b/spec.md` からの機能仕様

## 概要

playwright-novnc サービスに platform の上書き設定を追加し、arm64 環境でも manifest 不一致のエラーを回避する。既定は linux/amd64 とし、必要に応じて環境変数で上書きできるようにする。さらに gwt サービスから `host.docker.internal` をホストへ解決できるようにする。合わせて arm64 向けの手順と注意点をドキュメントへ追記する。

## 技術コンテキスト

**言語/バージョン**: Docker Compose YAML
**主要な依存関係**: Docker Engine, Docker Compose
**ストレージ**: N/A
**テスト**: 手動検証（docker compose up -d の結果確認）
**ターゲットプラットフォーム**: macOS/Linux (arm64/amd64)
**プロジェクトタイプ**: 単一プロジェクト
**パフォーマンス目標**: N/A
**制約**: 既存の Playwright noVNC イメージは変更しない
**スケール/範囲**: Docker 開発環境の起動のみ

## 原則チェック

- シンプルさ: 既存の compose 定義に最小の設定追加のみ
- テストファースト: 手動検証項目を tasks に明記し、変更前提の確認を先に定義
- 既存コード尊重: 既存 docker-compose.yml と docs の改修に留める
- 品質ゲート: markdownlint/commitlint の実行をタスクに含める

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-925c010b/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
└── tasks.md
```

### ソースコード（リポジトリルート）

```text
Docker Compose 定義: docker-compose.yml
ドキュメント: docs/docker-usage.md
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 現在の Docker Compose 定義と arm64 での挙動を把握する

**出力**: `specs/SPEC-925c010b/research.md`

### 調査項目

1. 既存 compose での playwright-novnc イメージ指定の確認
2. arm64 で no matching manifest となる原因の整理
3. platform 指定による回避方針の妥当性

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 設定追加とドキュメント更新の内容を確定する

**出力**:
- `specs/SPEC-925c010b/data-model.md`
- `specs/SPEC-925c010b/quickstart.md`

### 1.1 データモデル設計

**ファイル**: `data-model.md`

- 対象なし（設定変更のみ）

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

- arm64 向けの platform 上書き方法
- エミュレーション有効化の注意点

## フェーズ2: タスク生成

**次のステップ**: tasks.md を作成し、変更内容を細分化する

**出力**: `specs/SPEC-925c010b/tasks.md`

## 実装戦略

### 優先順位付け

1. **P1**: arm64 での起動回避策（platform 指定）
2. **P2**: Linux での host.docker.internal 解決
3. **P3**: amd64 既存フローの維持
4. **P4**: ドキュメント補足

### 独立したデリバリー

- platform 指定追加のみで arm64 のエラー回避が可能
- host.docker.internal 解決の追加は独立して提供可能
- ドキュメント追記は独立して提供可能

## テスト戦略

- 手動検証: arm64 で docker compose up -d を実行し、manifest エラーが出ないことを確認
- 手動検証: Linux で `host.docker.internal` が名前解決できることを確認
- 回帰確認: amd64 で従来通り起動できることを確認

## リスクと緩和策

### 技術的リスク

1. **arm64 で amd64 実行時のエミュレーション未設定**
   - **緩和策**: ドキュメントで Docker Desktop/エミュレーション要件を明記する

### 依存関係リスク

1. **playwright-novnc イメージの仕様変更**
   - **緩和策**: platform を環境変数で上書き可能にし、調整可能にする

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術方針の整理
2. ✅ フェーズ1完了: 設計ドキュメント作成
3. ✅ タスク生成: tasks.md を作成
4. ⏭️ 実装作業へ進む
