# Synapse Field PoC

TypeScript + Canvas でシナプス風ネットワークを描画する単体 PoC です。本体アプリとは独立した `./poc/` 配下で検証できます。

## 使い方

```bash
cd poc/synapse-field
bun run build   # dist/ に main.js, index.html, styles.css を生成
bun run serve   # http://localhost:3400 で静的配信
```

`bun run dev` を使うとビルドと簡易サーバー起動をまとめて実行できます。

## 実装メモ

- `main.ts` に粒子ノード／エッジのデータ構造とアニメーションループを実装
- `bun build` を使って TypeScript をブラウザ向けの ES Module にバンドル
- `styles.css` でヒーロー／キャンバス／説明パネルを装飾し、シナプスをイメージした配色を適用
