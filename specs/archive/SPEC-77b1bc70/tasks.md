---
description: "リリースフロー要件の明文化とリリース開始時 main→develop 同期のタスク"
---

# タスク: リリースフロー要件の明文化とリリース開始時 main→develop 同期

**入力**: `/specs/SPEC-77b1bc70/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

## フェーズ1: ユーザーストーリー1 - リリース開始から公開までを自動化したい (優先度: P1)

**ストーリー**: prepare-release → release の自動化要件を仕様化し、main→develop の同期を追加する

### テスト（TDD）

- [ ] **T101** [P] [US1] `scripts/check-release-flow.sh` を新規作成し、現状で失敗する検証（同期ステップ未設定・sync-develop残存・release guide存在・README参照）を追加する

### 実装

- [ ] **T102** [US1] `.github/workflows/prepare-release.yml` に main→develop 同期ステップ（`git merge --no-ff origin/main` + push）を追加し、`GITHUB_TOKEN` を使用する

## フェーズ2: ユーザーストーリー2 - リリース開始時に main を develop に統合したい (優先度: P1)

**ストーリー**: release.yml の back-merge を廃止する

- [ ] **T201** [US2] `.github/workflows/release.yml` から `sync-develop` ジョブを削除する

## フェーズ3: ユーザーストーリー3 - リリースガイドを廃止し、仕様へ統合したい (優先度: P2)

**ストーリー**: リリースガイド削除と README の参照更新

- [ ] **T301** [US3] `docs/release-guide.md` と `docs/release-guide.ja.md` を削除する
- [ ] **T302** [US3] `README.md` / `README.ja.md` の release guide 参照を仕様ドキュメントへ切り替える

## フェーズ4: 統合と検証

- [ ] **T401** [統合] `scripts/check-release-flow.sh` を実行して要件適合を確認する
- [ ] **T402** [統合] `cargo fmt --all -- --check` を実行し、フォーマット違反が無いことを確認する
- [ ] **T403** [統合] `cargo clippy --all-targets --all-features -- -D warnings` を実行し、警告が無いことを確認する
- [ ] **T404** [統合] `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` を実行し、Markdownlint が通ることを確認する

## タスク凡例

- **[US1]**: ユーザーストーリー1
- **[US2]**: ユーザーストーリー2
- **[US3]**: ユーザーストーリー3
- **[統合]**: 統合タスク
