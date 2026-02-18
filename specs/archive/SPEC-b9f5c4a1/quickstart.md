# Quickstart: ログ運用統一（pino構造化ログ）

## 前提
- Bun 1.x
- 依存はリポジトリに追加済み（pino@10）

## セットアップ
```bash
bun install
```

## 主要コマンド
- 単体テスト（ロガー関連）: `bun test tests/logging/logger.test.ts tests/logging/rotation.test.ts`
- 全テスト: `bun test`

## カテゴリ運用
- CLI: `category=cli`
- Webサーバー: `category=server`
- 追加コンポーネントは同一ファイル出力で `category` を付与

## ローテーション
- 起動時に `~/.gwt/logs/<cwd名>/` 配下で7日より古いファイルを削除
- 日次サイズ上限なし

## 出力パス
- 既定: `~/.gwt/logs/<カレントディレクトリ名>/<YYYY-MM-DD>.jsonl`
- サブディレクトリは存在しなければ自動作成

## 画面出力との分離
- ユーザー向けのメッセージは console 出力（「ログ」という語を使わない）
- 永続ログは pino 構造化ログのみ
