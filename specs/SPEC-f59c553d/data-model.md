# データモデル: npm postinstall ダウンロード安定化

## DownloadAttempt

- **attempt**: 試行回数（1..N）
- **status**: HTTPステータス or `network-error`
- **message**: エラーメッセージ（ログ用）
