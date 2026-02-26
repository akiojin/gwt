# 実装計画: Windows版 Launch Agent で `npx.cmd` 起動失敗を防止する

**仕様ID**: `SPEC-1265` | **日付**: 2026-02-26 | **仕様書**: `specs/SPEC-1265/spec.md`

## 目的

- Windows で Launch Agent 実行時に `npx.cmd` の外側クォート混入で起動失敗する問題を解消する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + `portable-pty`（`crates/gwt-core/`）
- **フロントエンド**: 変更なし
- **ストレージ/外部連携**: 変更なし
- **テスト**: `cargo test -p gwt-core terminal::pty`
- **前提**: 問題は Windows のコマンド正規化不足に起因し、`resolve_spawn_command_for_platform_*` が主要修正箇所

## 実装方針

### Phase 1: テスト先行（TDD）

- `crates/gwt-core/src/terminal/pty.rs` に、外側クォート混入 `npx.cmd` が失敗する再現ケースのテストを追加する。
- `.cmd/.bat` 判定が正規化後に成立することを追加テストで固定化する。

### Phase 2: Windows コマンド正規化の実装

- `pty.rs` に外側クォート再帰除去ヘルパーを追加する。
- Windows 解決フローで正規化コマンドを使って batch 判定・コマンド式構築を行う。
- 既存の非バッチ経路（PowerShell wrapper）や UTF-8 強制経路には仕様変更を加えない。

### Phase 3: 検証

- 追加テストと関連既存テストを実行し、回帰がないことを確認する。

## テスト

### バックエンド

- `resolve_spawn_command_for_platform` の外側クォート混入ケース
- `.cmd/.bat` 判定の正規化ケース
- 既存の Windows wrapper 分岐テスト一式

### フロントエンド

- 変更なし
