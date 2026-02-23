# 機能仕様: マイグレーション時の `Directory not empty` を非致命化

**仕様ID**: `SPEC-c2d9e741`
**作成日**: 2026-02-23
**ステータス**: 実装完了
**カテゴリ**: Core / Migration
**入力**: ユーザー報告: "マイグレーション時に `IO error ... Failed to remove directory: Directory not empty (os error 66)` が発生する"

## 背景

- 現在のマイグレーション実装は、ルートディレクトリのクリーンアップ中に `remove_dir_all` が `Directory not empty` を返すと即時失敗する
- この失敗はロールバックを誘発し、マイグレーション全体が完了しない
- 実運用では `node_modules` など更新頻度が高いディレクトリでこのエラーが発生しやすい

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - マイグレーションを最後まで完了したい (優先度: P0)

ユーザーとして、`Directory not empty` が発生してもマイグレーションが中断されず完了してほしい。

**独立したテスト**: `Directory not empty` 判定ロジックとクリーンアップ継続ロジックをユニットテストで確認する。

**受け入れシナリオ**:

1. **前提条件** ルートクリーンアップ中のディレクトリ削除で `Directory not empty` が返る、**操作** マイグレーションを実行、**期待結果** マイグレーションはエラー終了せず後続フェーズへ進む
2. **前提条件** ルートクリーンアップ中のディレクトリ削除でそれ以外のIOエラーが返る、**操作** マイグレーションを実行、**期待結果** 従来通りエラーとして扱われる

## エッジケース

- `Directory not empty` が OS 依存コード（macOS `66` / Linux `39`）で返る
- `PermissionDenied` など本当に致命的なエラーは従来通り失敗させる

## 要件 *(必須)*

### 機能要件

- **FR-c2d9e741-01**: システムは、マイグレーションのルートクリーンアップで `Directory not empty` エラーが発生した場合、警告ログを出して処理を継続しなければならない
- **FR-c2d9e741-02**: システムは、`Directory not empty` 以外のIOエラーは従来通り `MigrationError::IoError` として返さなければならない
- **FR-c2d9e741-03**: システムは、`Directory not empty` 判定ロジックをユニットテストでカバーしなければならない

### 非機能要件

- **NFR-c2d9e741-01**: 既存のマイグレーション成功経路と失敗経路（非 `Directory not empty`）を壊さない

## 制約と仮定

- 修正対象は `crates/gwt-core/src/migration/executor.rs` に限定する
- UIやTauriコマンド契約は変更しない

## 成功基準 *(必須)*

- **SC-c2d9e741-01**: ユニットテストで `Directory not empty` 判定が真になるケースが通る
- **SC-c2d9e741-02**: ユニットテストで `PermissionDenied` など非対象エラーは偽になるケースが通る
- **SC-c2d9e741-03**: `cargo test -p gwt-core migration::executor` が成功する

## 範囲外

- 既存マイグレーション全体の設計変更
- クリーンアップ対象ディレクトリの追加/削除ルール変更
