# 実装計画: npm postinstall ダウンロード安定化

**仕様ID**: `SPEC-f59c553d` | **日付**: 2026-01-26 | **仕様書**: `specs/SPEC-f59c553d/spec.md`
**入力**: `/specs/SPEC-f59c553d/spec.md` からの機能仕様

## 概要

npm postinstallとbunxオンデマンド取得のGitHub Releasesダウンロードを安定化する。`package.json` バージョンから `vX.Y.Z` のアセットURLを生成し、postinstallはAPI取得に失敗した場合のフォールバックを用意する。HTTP 404/403/5xx等の一時的失敗は指数バックオフで再試行し（最大5回、初回0.5秒、倍率2、上限5秒）、失敗時は英語の復旧ガイダンスを表示する。指定バージョンのアセットが無い場合でも `latest` へはフォールバックしない。bunxオンデマンド取得（`bin/gwt.js`）はタグURLのみを使用し、リトライは行わない。テストはNode標準の `node:test` でURL生成とリトライ判定を検証する。

## 技術コンテキスト

**言語/バージョン**: Node.js 18+（ESM）
**主要な依存関係**: Node標準ライブラリ（https, fs, path, url）
**ストレージ**: N/A
**テスト**: `node:test`（Node標準）
**ターゲットプラットフォーム**: darwin/linux/win32（x64/arm64）
**プロジェクトタイプ**: 単一（scripts/配下のJS）
**パフォーマンス目標**: 初回取得の成功率改善（再試行は数秒以内）
**制約**: 追加依存を入れない、CLI出力は英語のみ
**スケール/範囲**: npm postinstall + bunxオンデマンド取得

## 原則チェック

- シンプルさ優先（既存の `scripts/postinstall.js` を最小修正）
- TDD必須（テスト追加→実装）
- 既存ファイル改修優先（新規ファイルはテストのみ）
- 品質ゲート（lint/markdownlint/commitlint）

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-f59c553d/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
└── tasks.md
```

### ソースコード（リポジトリルート）

```text
scripts/
├── postinstall.js
├── postinstall.test.js
├── release-download.js
bin/
└── gwt.js
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存のpostinstall実装とGitHub Releases取得フローを確認し、変更方針を決定する

**出力**: `specs/SPEC-f59c553d/research.md`

### 調査項目

1. **既存のコードベース分析**
   - `scripts/postinstall.js` のURL生成/ダウンロード/エラー処理
   - `bin/gwt.js` のオンデマンド取得ロジック（現状は `releases/latest` 依存）

2. **技術的決定**
   - `package.json` バージョン利用のURL生成
   - `node:test` によるテスト追加
   - リトライ条件と回数（HTTP 404/403/5xx + network error、最大5回、初回0.5秒、倍率2、上限5秒）

3. **制約と依存関係**
   - 追加依存なし
   - GitHub Releases APIのレスポンスに依存

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に関数分割とテスト対象を明確化する

**出力**:
- `specs/SPEC-f59c553d/data-model.md`
- `specs/SPEC-f59c553d/quickstart.md`
- `specs/SPEC-f59c553d/contracts/` （該当なし）

### 1.1 データモデル設計

**ファイル**: `data-model.md`

- DownloadAttempt（試行回数、ステータス、失敗理由）

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

- `node --test scripts/postinstall.test.js` でテストを実行
- `node scripts/postinstall.js` でローカル検証

### 1.3 契約/インターフェース

該当なし（API追加なし）

## フェーズ2: タスク生成

**次のステップ**: tasks.md を生成

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-f59c553d/tasks.md`

## 実装戦略

### 優先順位付け

1. **P1**: URL生成・リトライロジックの追加 + bunxオンデマンド取得のバージョン固定
2. **P2**: 失敗時メッセージの改善

### 独立したデリバリー

- US1完了で初回取得の安定性を改善
- US2完了でbunxオンデマンド取得のバージョン固定を保証
- US3完了で復旧手順の明確化

## テスト戦略

- **ユニットテスト**: URL生成、リトライ判定、エラーメッセージ生成、bunxオンデマンド取得のタグURL構築
- **統合テスト**: なし（ネットワーク依存のため）

## リスクと緩和策

### 技術的リスク

1. **GitHub Releasesの反映遅延**
   - **緩和策**: 404/403/5xxのリトライ（最大5回、初回0.5秒、倍率2、上限5秒）

2. **package.json のバージョン取得失敗**
   - **緩和策**: バージョン取得に失敗した場合は英語のエラーと手動復旧案内を表示する

3. **Node ESMのテスト実行が失敗**
   - **緩和策**: `node --test` の標準機能のみを使用

### 依存関係リスク

1. **GitHub APIの一時的エラー**
   - **緩和策**: API失敗時はURLパターンにフォールバック

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義
3. ⏭️ tasks.md を作成
4. ⏭️ テスト作成 → 実装
