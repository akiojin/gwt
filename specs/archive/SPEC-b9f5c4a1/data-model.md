# データモデル（ログ運用統一）

## LogConfig
- `level`: string — ログレベル。既定は `"info"`（必要に応じて config で上書き）
- `logDir`: string — 出力先ディレクトリ。既定は `~/.gwt/logs/<cwd basename>`
- `filename`: string — ログファイル名。既定は `<YYYY-MM-DD>.jsonl`
- `category`: string — ログカテゴリ（例: `cli`, `server`, `worker`）。必須
- `base`: object — 追加の共通フィールド
- `keepDays`: number — 保持日数。既定 7
- `sync`: boolean — 同期出力フラグ（主にテスト用）

## LogRecord (構造化ログ出力)
- `time`: ISO timestamp
- `level`: number (pino既定)
- `msg`: string
- `category`: string (LogConfig.category)
- `...base`: 任意の追加フィールド

## RotationPolicy
- `keepDays`: number — 7 (既定)
- `cutoff`: Date — `now - keepDays`
- 動作: 起動時に `logDir` 配下のファイルで `mtime < cutoff` を削除
