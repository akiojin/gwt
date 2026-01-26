# 調査: npm postinstall ダウンロード安定化

## 既存実装の確認

- `scripts/postinstall.js` は `releases/latest` を使用してアセットURLを取得している。
- GitHub APIからアセットURL取得に失敗した場合も `latest/download` を参照するため、反映遅延時に404が発生する可能性がある。

## 技術的決定

- `package.json` のバージョンから `vX.Y.Z` のタグURLを生成する。
- GitHub API取得に失敗した場合は、`releases/download/vX.Y.Z/<artifact>` にフォールバックする。
- HTTP 404/403/5xx とネットワークエラーに対して指数バックオフで再試行する（最大5回、初回0.5秒、倍率2、上限5秒）。
- 指定バージョンが存在しない場合でも `latest` へはフォールバックしない。
- テストはNode標準の `node:test` を使用する。

## 制約と依存関係

- 追加依存なし（Node標準のみ）
- GitHub Releases APIに依存
