# タスク: HuskyでCIと同等のLintを実行

**入力**: `/specs/SPEC-6408df0c/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（ユーザーストーリー用に必須）

**テスト**: 自動テストが必須（FR-007）。フック内容を検証するスクリプトを追加する。

**構成**: タスクはユーザーストーリーごとにグループ化する。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

## フェーズ1: セットアップ（共有）

- [ ] **T001** [P] [共通] `package.json` に `prepare` を追加し `bunx husky install` で自動セットアップ
- [ ] **T002** [P] [共通] `.husky/pre-push` を追加し、CIと同等のLint（clippy/fmt/markdownlint）を実行
- [ ] **T003** [共通] `.husky/pre-commit` からCI同等Lintを除去し、pre-pushに集約

## フェーズ2: ユーザーストーリー1 - pre-pushでCI同等Lintを自動実行 (P1)

- [ ] **T101** [US1] `.husky/pre-push` に英語エラーメッセージを追加し、失敗時にpushを中断

## フェーズ3: ユーザーストーリー2 - Huskyの自動セットアップ (P2)

- [ ] **T201** [US2] `package.json` の既存 `postinstall` と共存できる形でHuskyセットアップを統合

## フェーズ4: ユーザーストーリー3 - 失敗時に明確な英語エラー (P3)

- [ ] **T301** [US3] フック内のメッセージを英語に統一し、失敗理由がわかる文言に調整

## テスト

- [ ] **T901** [TEST] `scripts/verify-husky-hooks.sh` を追加し、フック内容と `prepare` 設定を検証
- [ ] **T902** [TEST] `package.json` に `lint:husky` スクリプトを追加し、T901を実行可能にする

