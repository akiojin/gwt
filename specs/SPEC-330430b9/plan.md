# 実装計画: セッション変換一覧の内容表示とSpaceプレビュー

**仕様ID**: `SPEC-330430b9` | **日付**: 2026-01-29 | **仕様書**: `specs/SPEC-330430b9/spec.md`
**入力**: `/specs/SPEC-330430b9/spec.md` からの機能仕様

## 概要

セッション変換の一覧に「セッション名（Worktree名・ブランチ名相当） + 開始ユーザーメッセージ抜粋 + 更新日時」を表示し、Spaceでプレビュー（先頭10メッセージ）を開閉できるようにする。変換実行中はスピナー「Converting session...」を表示する。UIは英語のみ、ASCIIのみ。既存のExecution Mode: Convertフローは維持し、一覧/プレビューの失敗はプレースホルダ表示で継続する。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui, crossterm, chrono, serde_json
**ストレージ**: ローカルファイル（各エージェントのセッションファイル）
**テスト**: `cargo test`
**ターゲットプラットフォーム**: Linux/macOS/Windows (CLI)
**プロジェクトタイプ**: 単一（Rustワークスペース内のCLI）
**パフォーマンス目標**: 一覧表示の初期描画が体感で遅くならない
**制約**: UI文言は英語、表示はASCIIのみ
**スケール/範囲**: 既存TUIのWizard内のみ

## 原則チェック

- シンプルさの追求: 既存Wizardに最小限の状態追加で実装
- テストファースト: 先にWizardの新規ロジックをテストで固定
- 既存コードの尊重: `wizard.rs` / `app.rs` を中心に改修
- 品質ゲート: `cargo test` を実行可能な範囲で確認

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-330430b9/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
└── tasks.md
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-cli/
│   └── src/tui/
│       ├── app.rs
│       └── screens/wizard.rs
└── gwt-core/
    └── src/ai/session_parser/
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存Wizard/キー処理/セッションパーサーの流れを確認

**出力**: `specs/SPEC-330430b9/research.md`

調査結果は research.md に記載済み。

## フェーズ1: 設計（アーキテクチャと契約）

**出力**:
- `specs/SPEC-330430b9/data-model.md`
- `specs/SPEC-330430b9/quickstart.md`

### 1.1 データモデル設計

- `ConvertSessionEntry` にセッション名（Worktree名）と開始ユーザーメッセージ抜粋を保持
- `WizardState` にプレビュー状態（open/scroll/lines/error）を追加

### 1.2 クイックスタート

- TUI起動→Execution Mode: Convert→Session Select→Spaceでプレビュー

## フェーズ2: 実装計画

### 変更対象

- `crates/gwt-cli/src/tui/screens/wizard.rs`
  - `ConvertSessionEntry` 拡張
  - セッション一覧生成時にセッション名（Worktree名）と開始ユーザーメッセージを抽出
  - プレビュー生成と表示レンダリング
  - Wizard footerのキー表記更新
  - テスト追加・更新
- `crates/gwt-cli/src/tui/app.rs`
  - Wizard可視時のキー処理にSpace/Esc/スクロールを追加
  - 新しいMessage追加とハンドリング

### 実装方針（決定事項）

1. 一覧表示
   - 表示形式: `<session_name> | <start_user_message_snippet> | updated YYYY-MM-DD HH:MM`
   - ID/メッセージ数は表示しない
   - セッション名がない場合は `No name` を表示
   - 開始ユーザーメッセージがない場合は `No user message` を表示
   - 解析失敗は `Unavailable` を表示

2. プレビュー
   - Spaceで開閉、Escで閉じる
   - 先頭10メッセージ（User/Assistant）のみ表示
   - ヘッダーに `Name:` を表示
   - `User:` / `Assistant:` のASCIIラベル
   - Up/Downでスクロール
   - 失敗時は英語のエラーメッセージのみ表示

3. 解析
   - `load_sessions_for_agent` で `SessionParser::parse` を使い開始ユーザーメッセージを抽出
   - セッション名は `worktree_path` の末尾ディレクトリ名（Worktree名・ブランチ名相当）を使用
   - プレビューは選択中セッションを再パースし、先頭10メッセージを生成

4. 変換中表示
   - 変換処理はバックグラウンドで実行し、UIはスピナー表示

## リスクと対策

- **パース負荷**: 一覧表示時にパースコストが増える → 失敗時はプレースホルダで継続、先頭メッセージのみ抽出。
- **キー競合**: SpaceがBranchListの選択に使われる → Wizard可視時に優先的にハンドリング。

## 成果物

- `specs/SPEC-330430b9/research.md`
- `specs/SPEC-330430b9/data-model.md`
- `specs/SPEC-330430b9/quickstart.md`
- `specs/SPEC-330430b9/tasks.md`
