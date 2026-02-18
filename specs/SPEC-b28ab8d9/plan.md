# 実装計画: CI の Node ツールチェーンを pnpm に統一する（gwt-gui + commitlint）

**仕様ID**: `SPEC-b28ab8d9` | **日付**: 2026-02-10 | **仕様書**: `specs/SPEC-b28ab8d9/spec.md`

## 概要

GitHub Actions の commitlint ジョブを `pnpm`（Corepack）ベースに移行し、`npm install -g` への依存を廃止する。
合わせて `gwt-gui` の依存インストール/チェック/ビルドを `pnpm` に統一し、pnpm バージョンを `packageManager` で固定する。

## 変更対象

- `.github/workflows/lint.yml`
- `.github/workflows/test.yml`
- `.github/workflows/release.yml`
- `package.json`
- `pnpm-lock.yaml`（新規）
- `gwt-gui/package.json`
- `gwt-gui/pnpm-lock.yaml`（新規）
- `gwt-gui/package-lock.json`（削除）
- `.gitignore`（更新）
- `scripts/verify-ci-node-toolchain.sh`（新規）
- `README.md` / `CONTRIBUTING.md`（方針の明記）

## 実装手順

1. `package.json` と `gwt-gui/package.json` に `packageManager` を追加し、pnpm バージョンを固定する
2. `gwt-gui/pnpm-lock.yaml` を導入し、`gwt-gui/package-lock.json` を廃止する
3. `.gitignore` に `gwt-gui/package-lock.json` を追加し、誤って再導入されないようにする
4. `.github/workflows/lint.yml` の commitlint ジョブを以下に置換する
   - `actions/setup-node` のキャッシュを `pnpm` に合わせる
   - `corepack enable` + `corepack prepare pnpm@... --activate`
   - `pnpm dlx @commitlint/cli@... commitlint ...`
5. `.github/workflows/test.yml` / `release.yml` の `gwt-gui` 検証を `pnpm install --frozen-lockfile` / `pnpm run check` / `pnpm run build` に置換する
6. `scripts/verify-ci-node-toolchain.sh` を追加/更新し、pnpm 移行の不変条件を検証する
7. ドキュメントに「gwt-gui のセットアップ/チェックは pnpm」を追記する

## テスト戦略

- ローカルで `bash scripts/verify-ci-node-toolchain.sh` を実行して成功すること
- 既存の Rust 側検証（`cargo test` / `cargo clippy` / `cargo fmt --check`）が継続して通ること

## リスクと緩和策

- pnpm のバージョン差分で CI 実行が不安定になる
  - 緩和策: `packageManager` と `corepack prepare` でバージョンを固定する
