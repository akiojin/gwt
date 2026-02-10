# 実装計画: CI の Node ツールチェーンを pnpm に統一する（commitlint）

**仕様ID**: `SPEC-b28ab8d9` | **日付**: 2026-02-10 | **仕様書**: `specs/SPEC-b28ab8d9/spec.md`

## 概要

GitHub Actions の commitlint ジョブを `pnpm`（Corepack）ベースに移行し、`npm install -g` への依存を廃止する。合わせてリポジトリのロックファイルを `pnpm-lock.yaml` に統一し、pnpm バージョンを `packageManager` で固定する。

## 変更対象

- `.github/workflows/lint.yml`
- `package.json`
- `pnpm-lock.yaml`（新規）
- `.npmrc`（新規）
- `scripts/verify-ci-node-toolchain.sh`（新規）
- `README.md` / `CONTRIBUTING.md`（方針の明記）

## 実装手順

1. `package.json` に `packageManager` を追加し、pnpm バージョンを固定する
2. `pnpm-lock.yaml` を導入し、`package-lock.json` を廃止する
3. `.npmrc` で `package-lock.json` を生成しない方針を追加する
4. `.github/workflows/lint.yml` の commitlint ジョブを以下に置換する
   - `actions/setup-node` のキャッシュを `pnpm` に合わせる
   - `corepack enable` + `corepack prepare pnpm@... --activate`
   - `pnpm dlx @commitlint/cli@... commitlint ...`
5. `scripts/verify-ci-node-toolchain.sh` を追加し、pnpm 移行の不変条件を検証する
6. ドキュメントに「CI の Node ツールは pnpm を使用し、package-lock は使わない」旨を追記する

## テスト戦略

- ローカルで `bash scripts/verify-ci-node-toolchain.sh` を実行して成功すること
- 既存の Rust 側検証（`cargo test` / `cargo clippy` / `cargo fmt --check`）が継続して通ること

## リスクと緩和策

- pnpm のバージョン差分で CI 実行が不安定になる
  - 緩和策: `packageManager` と `corepack prepare` でバージョンを固定する
