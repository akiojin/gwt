# タスク: CI の Node ツールチェーンを pnpm に統一する（commitlint）

**入力**: `/specs/SPEC-b28ab8d9/`  
**前提条件**: plan.md、spec.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

## フェーズ2: ユーザーストーリー1 - CI の commitlint が pnpm で実行できる (P1)

- [ ] **T101** [P] [US1] `.github/workflows/lint.yml` の commitlint を `pnpm dlx` + Corepack に移行
- [ ] **T102** [P] [US1] `scripts/verify-ci-node-toolchain.sh` を追加して CI 定義の不変条件を検証

## フェーズ2: ユーザーストーリー2 - ロックファイルと pnpm バージョンが固定される (P1)

- [ ] **T201** [P] [US2] `pnpm-lock.yaml` を導入し `package-lock.json` を廃止
- [ ] **T202** [P] [US2] `package.json` に `packageManager` を追加
- [ ] **T203** [US2] `.npmrc` に `package-lock=false` を追加

## フェーズ5: 統合と検証

- [ ] **T401** [統合] `bash scripts/verify-ci-node-toolchain.sh` を実行して成功を確認
- [ ] **T402** [統合] `cargo test` と `cargo clippy --all-targets --all-features -- -D warnings` と `cargo fmt --check` を実行して成功を確認
- [ ] **T403** [統合] ドキュメント（README/CONTRIBUTING）に方針を追記
