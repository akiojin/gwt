# 機能仕様: Playwrightベースの実装テスト基盤整備（WebView UI）

**仕様ID**: `SPEC-7d1a4b2e`
**作成日**: 2026-02-13
**ステータス**: ドラフト
**カテゴリ**: GUI
**関連Issue**: [#1001](https://github.com/akiojin/gwt/issues/1001)

## 背景

- 現在の `gwt-gui` は Vitest による単体テストが中心で、WebView UI の実装テスト（E2E相当）を継続運用する基盤が不足している。
- Tauri 依存（`@tauri-apps/api`）を含む画面導線を、Web 層で再現可能なモック戦略とともに CI に組み込みたい。

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - 開発者が Playwright をローカルで再現実行できる（優先度: P0）

開発者として、`pnpm` コマンドで Playwright スモークテストを実行し、GUI の主要導線を検証したい。

**受け入れシナリオ**:

1. **前提条件** 依存インストール済み、**操作** `cd gwt-gui && pnpm run test:e2e` を実行、**期待結果** Chromium で E2E が実行される
2. **前提条件** テスト失敗、**操作** レポート出力を確認、**期待結果** 失敗内容を追跡できる

### ユーザーストーリー 2 - CI で同じ Playwright スモークを実行できる（優先度: P0）

開発者として、PR 時に CI 上でも同じ E2E が実行され、回帰を検知したい。

**受け入れシナリオ**:

1. **前提条件** PR を作成、**操作** GitHub Actions `test.yml` が実行、**期待結果** `e2e` ジョブで Playwright が実行される
2. **前提条件** E2E が失敗、**操作** CI artifact を確認、**期待結果** Playwright レポートを参照できる

### ユーザーストーリー 3 - Tauri 依存を Web E2E で明示的にモックできる（優先度: P0）

開発者として、Tauri ネイティブ依存を切り分けた Web E2E を安定運用したい。

**受け入れシナリオ**:

1. **前提条件** Playwright テスト実行、**操作** `window.__TAURI_INTERNALS__` を初期化して画面を起動、**期待結果** `@tauri-apps/api` 依存でクラッシュしない
2. **前提条件** Open Project 導線を実行、**操作** Recent Project クリック後に Agent Mode で送信操作、**期待結果** 主要 UI フロー（起動・入力・送信）が通る

## 要件

### 機能要件

- **FR-001**: `gwt-gui` に Playwright 実行設定（Chromium）を追加しなければならない
- **FR-002**: `gwt-gui/package.json` に `test:e2e` スクリプトを追加しなければならない
- **FR-003**: Tauri API 依存を E2E 実行時に明示的にモックする仕組みを追加しなければならない
- **FR-004**: 起動→Open Project（Recent Project）→Agent Mode 送信までのスモークシナリオを追加しなければならない
- **FR-005**: CI（`test.yml`）で Playwright ジョブを 1 つ以上追加しなければならない
- **FR-006**: README（英語/日本語）にローカルと CI の実行手順を記載しなければならない

### 非機能要件

- **NFR-001**: 初期導入では Chromium のみを対象とする
- **NFR-002**: E2E は既存 Vitest テストと独立して実行できる
- **NFR-003**: E2E 失敗時に CI で調査用 artifact（Playwright report/test-results）を取得できる

## 制約

- 対象は Web 層 E2E のみ（Tauri ネイティブ統合 E2E は別タスク）
- CI は既存 `test.yml` へ統合する
- パッケージ管理は `pnpm` を利用する

## 成功基準

- **SC-001**: `pnpm run test:e2e` で Playwright テストがローカル再現できる
- **SC-002**: `test.yml` の E2E ジョブで同一テストが実行される
- **SC-003**: スモークシナリオで起動・主要入力・送信が通る
- **SC-004**: Tauri 依存が E2E で明示的にモックされている
