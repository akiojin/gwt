# データモデル: エージェント状態の可視化（Hook再登録の自動化）

**仕様ID**: `SPEC-861d8cdf`
**日付**: 2026-01-21

## 変更点

- 新規エンティティや永続データ構造の追加はない
- 既存の設定ファイル（~/.claude/settings.json）とセッションファイル（.gwt-session.toml）を再利用する

## 影響範囲

- settings.json 内の gwt hook 設定が、起動時に再登録される
- 既存の非gwt hookは保持される
