# タスク: ブランチ選択時のdivergence/FF失敗ハンドリング（起動継続）

**仕様ID**: `SPEC-40c7b4f1`
**入力**: `/specs/SPEC-40c7b4f1/` の仕様・計画

## Phase 1: TDD - Test First

- [ ] **T001** CLI: divergence時でも起動が継続されることをテストで確認する（`tests/unit/index.protected-workflow.test.ts`）
- [ ] **T002** CLI: divergence検知失敗時でも起動が継続されることをテストで確認する（`tests/unit/index.protected-workflow.test.ts`）
- [ ] **T003** Web UI: divergence警告があっても起動ボタンが無効化されないことをテストで確認する（`tests/unit/web/client/pages/BranchDetailPage.test.tsx`）

## Phase 2: Implementation

- [ ] **T004** CLI: divergence検知時のブロック/Enter待ちを廃止し、警告のみで続行する（`src/index.ts`）
- [ ] **T005** CLI: divergence検知失敗時は警告のみで続行する（`src/index.ts`）
- [ ] **T006** Web UI: divergence警告文を注意喚起に調整し、起動ボタンの無効化条件から除外する（`src/web/client/src/pages/BranchDetailPage.tsx`, `src/web/client/src/components/branch-detail/ToolLauncher.tsx`）

## Phase 3: Verification

- [ ] **T007** `bun run test`（必要に応じて対象テストのみ）を実行し回帰がないことを確認する
- [ ] **T008** `.github/workflows/lint.yml` 相当のチェック（`bun run format:check` / `bun run lint` / `bunx --bun markdownlint-cli ...`）を実行し必要なら修正する

## Phase 4: Documentation

- [ ] **T009** `specs/specs.md` を最新化し、仕様一覧を更新する
