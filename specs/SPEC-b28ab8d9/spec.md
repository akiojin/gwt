# 機能仕様: CI の Node ツールチェーンを pnpm に統一する（commitlint）

**仕様ID**: `SPEC-b28ab8d9`  
**作成日**: 2026-02-10  
**ステータス**: ドラフト  
**入力**: GitHub Issue #938 "chore(ci): migrate gwt-gui + commitlint from npm to pnpm"

## 背景 / 問題

- 現状の GitHub Actions の commitlint ジョブは `npm install -g ...` に依存している。
- グローバル npm インストールは実行時間が増えやすく、依存解決の再現性も低い。
- リポジトリの Node ツールチェーンを `pnpm`（Corepack）に寄せ、ロックファイルと pnpm バージョンを固定して CI の再現性を高めたい。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - CI の commitlint が pnpm で実行できる (優先度: P1)

開発者が PR を作成したとき、GitHub Actions の commitlint ジョブが `pnpm` ベースで動作し、グローバル npm インストールに依存せずにコミットメッセージ検証ができる。

**独立したテスト**: `scripts/verify-ci-node-toolchain.sh` が `pnpm` 利用と `npm install -g` 不使用を検証できること。

**受け入れシナリオ**:

1. **前提条件** PR が作成される、**操作** Lint ワークフローが実行される、**期待結果** commitlint ジョブが `pnpm dlx` で commitlint を実行する
2. **前提条件** Lint ワークフローが存在する、**操作** ワークフロー定義を検証する、**期待結果** `npm install -g` が含まれない

---

### ユーザーストーリー 2 - ロックファイルと pnpm バージョンが固定される (優先度: P1)

開発者がクリーンなチェックアウトで Node ツールチェーンを扱う際に、`pnpm-lock.yaml` と `packageManager` により依存解決と pnpm バージョンが固定される。

**独立したテスト**: `scripts/verify-ci-node-toolchain.sh` が `pnpm-lock.yaml` の存在、`package-lock.json` の不在、`.npmrc` の設定を検証できること。

**受け入れシナリオ**:

1. **前提条件** リポジトリがクリーンな状態、**操作** `pnpm install --frozen-lockfile` を実行、**期待結果** ロックファイルに基づいて成功する
2. **前提条件** リポジトリルート、**操作** ロックファイルを確認、**期待結果** `pnpm-lock.yaml` が存在し `package-lock.json` が存在しない

---

### エッジケース

- `pnpm` が未インストールでも、Corepack を通じて CI で所定バージョンが利用できる
- commitlint の設定（ルール）は変更されない

## 要件 *(必須)*

### 機能要件

- **FR-001**: CI の commitlint ジョブは `pnpm`（Corepack）で commitlint を実行しなければならない
- **FR-002**: CI の commitlint ジョブは `npm install -g` に依存してはならない
- **FR-003**: リポジトリは `pnpm-lock.yaml` を持ち、`package-lock.json` 運用を廃止しなければならない
- **FR-004**: リポジトリは `packageManager` フィールドで pnpm バージョンを固定しなければならない
- **FR-005**: `.npmrc` により `package-lock.json` が生成されない方針を明文化しなければならない

### 非機能要件

- **NFR-001**: 変更は CI の実行時間を悪化させない（または改善する）こと
- **NFR-002**: 既存の commitlint ルール/判定ロジックを変更しないこと

## 成功基準 *(必須)*

- **SC-001**: `.github/workflows/lint.yml` の commitlint ジョブが `pnpm dlx` で動作する
- **SC-002**: `package-lock.json` がリポジトリから削除され、`pnpm-lock.yaml` が導入される
- **SC-003**: `scripts/verify-ci-node-toolchain.sh` が成功する

## 制約と仮定 *(該当する場合)*

- Node.js 18+ と Corepack が利用可能（GitHub Actions の `actions/setup-node` で提供される）
- npm publish の方式変更は本仕様の範囲外（必要なら別仕様で扱う）

## 範囲外 *(必須)*

- `commitlint.config.cjs` のルール仕様変更
- husky フックの実装変更（bunx の利用継続可）
- Rust 側の依存管理やビルドフロー変更
