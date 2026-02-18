# タスク: CI の Node ツールチェーンを pnpm に統一する（gwt-gui + commitlint）

**入力**: `/specs/SPEC-b28ab8d9/`  
**前提条件**: plan.md、spec.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

## フェーズ2: ユーザーストーリー1 - CI の commitlint が pnpm で実行できる (P1)

- [ ] **T101** [P] [US1] `.github/workflows/lint.yml` の commitlint を `pnpm dlx` + Corepack に移行
- [ ] **T102** [P] [US1] `scripts/verify-ci-node-toolchain.sh` を追加して CI 定義の不変条件を検証

## フェーズ2: ユーザーストーリー2 - gwt-gui の依存インストール/チェック/ビルドが pnpm で実行できる (P1)

- [ ] **T201** [P] [US2] `gwt-gui/pnpm-lock.yaml` を導入し `gwt-gui/package-lock.json` を廃止
- [ ] **T202** [P] [US2] `gwt-gui/package.json` に `packageManager` を追加
- [ ] **T203** [P] [US2] `.github/workflows/test.yml` の `gwt-gui` 検証を `pnpm` に移行
- [ ] **T204** [P] [US2] `.github/workflows/release.yml` の `gwt-gui` 依存インストールを `pnpm` に移行

## フェーズ2: ユーザーストーリー3 - ロックファイルと pnpm バージョンが固定される (P1)

- [ ] **T301** [P] [US3] `package.json` に `packageManager` を追加
- [ ] **T302** [US3] `.gitignore` に `gwt-gui/package-lock.json` を追加
- [ ] **T303** [US3] `scripts/verify-ci-node-toolchain.sh` を更新して pnpm 移行の不変条件を検証

## フェーズ5: 統合と検証

- [ ] **T401** [統合] `bash scripts/verify-ci-node-toolchain.sh` を実行して成功を確認
- [ ] **T402** [統合] `cd gwt-gui && pnpm install --frozen-lockfile && pnpm run check && pnpm run build` を実行して成功を確認
- [ ] **T403** [統合] `cargo test` と `cargo clippy --all-targets --all-features -- -D warnings` と `cargo fmt --check` を実行して成功を確認
- [ ] **T404** [統合] ドキュメント（README/CONTRIBUTING）に方針を追記
