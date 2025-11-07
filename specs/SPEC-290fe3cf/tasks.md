# タスク: Worktree側での.gitignore更新と重複登録防止

**入力**: `/specs/SPEC-290fe3cf/` からの設計ドキュメント
**前提条件**: plan.md, spec.md, research.md, data-model.md, quickstart.md

## フェーズ1: 共通セットアップ

- [ ] **T001** [P] [共通] `specs/SPEC-290fe3cf/` 配下のドキュメントを最新版へ整備（本タスクで完了扱い）

## ユーザーストーリー1 (P1) - Worktreeルートのみで.gitignore更新

- [ ] **T101** [P] [US1] `src/git.ts` に `getWorktreeRoot(): Promise<string>` を追加し、`git rev-parse --show-toplevel` を返す
- [ ] **T102** [US1] `src/worktree.ts#createWorktree` で `.gitignore` 更新対象を `getWorktreeRoot()` に切り替え、失敗時は `config.repoRoot` へフォールバックする
- [ ] **T103** [US1] `tests/unit/worktree.test.ts` に `getWorktreeRoot` のモックを追加し、`ensureGitignoreEntry` の呼び出しパスを検証

## ユーザーストーリー2 (P1) - 重複登録の排除

- [ ] **T201** [US2] `src/git.ts#ensureGitignoreEntry` を改修し、`split(/\r?\n/)` + `trim()` で既存行を比較、改行スタイルを維持したまま追記する
- [ ] **T202** [US2] `tests/unit/git.test.ts` に CRLF / 末尾改行なし / 既存重複 のケースを追加

## ユーザーストーリー3 (P2) - 失敗時の安全デグレード

- [ ] **T301** [US3] `tests/unit/worktree.test.ts` で `.gitignore` 更新失敗時に worktree 作成が継続することを再確認・補強

## 統合・品質

- [ ] **T401** [統合] `bun test` を実行し全テストが成功することを確認
- [ ] **T402** [統合] `bun run lint` / `bun run format:check` / `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` を完走させる

> すべてのタスクは Conventional Commits に対応する最小コミットへまとめる。
