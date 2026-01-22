# 実装計画: エージェント状態の可視化（Hook再登録の自動化）

**仕様ID**: `SPEC-861d8cdf` | **日付**: 2026-01-21 | **仕様書**: `specs/SPEC-861d8cdf/spec.md`
**入力**: `/specs/SPEC-861d8cdf/spec.md` からの機能仕様

## 概要

Hook設定を一度承認した後、gwt起動時にClaude CodeのHook設定を毎回再登録して、現在のgwt実行パスへ同期する。再登録は既存の非gwt hookを保持し、失敗時も起動を継続しログに記録する。対象はTUI起動フローに統合し、既存のHook未登録検出と併用する。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, serde_json, chrono
**ストレージ**: ファイル（~/.claude/settings.json, .gwt-session.toml）
**テスト**: cargo test
**ターゲットプラットフォーム**: CLI (macOS/Linux/Windows)
**プロジェクトタイプ**: 単一（Rustワークスペース）
**パフォーマンス目標**: Hook再登録は起動体験を阻害しない
**制約**: 既存設定の保持、CLI出力は英語のみ
**スケール/範囲**: 1ユーザー/複数worktree

## 原則チェック

- シンプルさ優先: 既存の設定操作関数を再利用し、再登録は「削除→追加」の最小手順で実装する
- TDD必須: 仕様更新 → テスト追加 → 承認 → 実装
- 既存コード尊重: `gwt-core` の `claude_hooks.rs` と `gwt-cli` の起動フローを改修する

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-861d8cdf/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
└── tasks.md
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-core/src/config/claude_hooks.rs
└── gwt-cli/src/tui/app.rs
```

## フェーズ0: 調査（技術スタック選定）

**出力**: `specs/SPEC-861d8cdf/research.md`

- 既存のHook登録/解除ロジックの挙動
- 起動時にHook未登録を検出する現在のフロー
- settings.jsonのフォーマット（新/旧）とgwt hookの判定条件

## フェーズ1: 設計（アーキテクチャと契約）

**出力**:
- `specs/SPEC-861d8cdf/data-model.md`
- `specs/SPEC-861d8cdf/quickstart.md`
- `specs/SPEC-861d8cdf/contracts/`（該当なし）

### 1.1 データモデル設計

既存のsettings.jsonとSession構造を利用し、新規エンティティは追加しない。

### 1.2 クイックスタートガイド

再登録の挙動確認と手動リセット手順を記載する。

### 1.3 契約/インターフェース

該当なし。

## フェーズ2: タスク生成

**次のステップ**: `specs/SPEC-861d8cdf/tasks.md` を更新する。

## 実装戦略

- 起動時に「Hook登録済み」の場合は `unregister_gwt_hooks` → `register_gwt_hooks` を実行
- 失敗時はログ記録のみで起動を継続
- Hook未登録の場合は既存の提案ダイアログにフォールバック
- テストでsettings.jsonの書き換えと非gwt hook保持を検証

## テスト戦略

- **ユニットテスト**: gwt hook再登録が旧パスを更新し、非gwt hookを保持すること
- **統合テスト**: TUI起動フローで再登録が呼ばれること（最小の起動フロー単体テスト）

## リスクと緩和策

1. **既存hookの破損**
   - **緩和策**: gwt hookのみ除去し、非gwt hookは保持するテストを追加

2. **設定ファイル書き込み失敗**
   - **緩和策**: 起動継続 + ログ通知、手動手順をquickstartに記載

## 次のステップ

1. ✅ 仕様更新（spec.md）
2. ✅ plan.md/research.md/data-model.md/quickstart.mdの更新
3. ⏭️ tasks.md 更新
4. ⏭️ テスト設計 → 実装
