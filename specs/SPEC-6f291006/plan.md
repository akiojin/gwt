# 実装計画: Migration backup copy の Windows 互換修正

**仕様ID**: `SPEC-6f291006` | **日付**: 2026-02-13 | **仕様書**: `specs/SPEC-6f291006/spec.md`

## 目的

- Windowsで `create_backup()` が `cp` 未検出により失敗する問題を解消する
- 既存マイグレーションのバックアップ/復元契約を維持する

## 技術コンテキスト

- バックエンド: Rust (`crates/gwt-core/src/migration/backup.rs`)
- 既存挙動: `copy_dir_recursive()` が `cp -a` を直接実行
- 問題: Windowsでは `cp` が存在せず `program not found` となる

## 実装方針

### Phase 1: テスト追加（RED）

- `cp` が `PATH` に無い状態でも `create_backup()` が成功するテストを追加する
- テストは環境変数変更を確実に復元する

### Phase 2: コピー実装修正（GREEN）

- `copy_dir_recursive()` を次の方針に変更する
- Windows: 標準APIで再帰コピー
- 非Windows: `cp -a` を試行し、失敗時は標準API再帰コピーへフォールバック
- フォールバック時はログ警告を出力する

### Phase 3: 検証

- 対象テストを実行して回帰を確認
