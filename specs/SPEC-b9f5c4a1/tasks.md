# Tasks: ログ運用統一（pino構造化ログ・7日ローテーション）

## Phase 1: Setup
- [ ] T001 Add pino dependency and lockfile update (`package.json`, `bun.lock`)  
- [ ] T002 Define default log path rule (`~/.gwt/logs/<cwd>/<YYYY-MM-DD>.jsonl`) (`src/config/logging.ts` or logger内)

## Phase 2: Foundational (Blocking)
- [ ] T010 Create logger factory scaffolding with category support (`src/logging/logger.ts`)  
- [ ] T011 Implement rotation helper to delete files older than 7 days (`src/logging/rotation.ts`)

## Phase 3: US1 - 統一ロガー（構造化JSON + category）
- [ ] T020 [US1] Write unit tests for JSON structure & category field (`tests/logging/logger.test.ts`)
- [ ] T021 [US1] Implement pino logger factory to satisfy tests (`src/logging/logger.ts`)
- [ ] T022 [US1] Wire CLI entry to emit logs via logger with category `cli` (`src/index.ts`)

## Phase 4: US2 - ローテーション & パス規則
- [ ] T030 [US2] Write unit tests for rotation (7日超削除, 起動時実行) & デフォルトパス規則 (`tests/logging/rotation.test.ts`, `tests/logging/logger.test.ts`)
- [ ] T031 [US2] Integrate rotation into logger init path (`src/logging/rotation.ts`, `src/logging/logger.ts`)

## Phase 5: US3 - サーバー統合（Web UI/REST）
- [ ] T040 [US3] Integrate pino logger into Fastify server with category `server` (`src/web/server/index.ts`)
- [ ] T041 [US3] Ensure server startup/shutdown logs use structured logger, screen messages remain separate (`src/web/server/index.ts`)

## Phase 6: Polish & Docs
- [ ] T050 Update developer quickstart for logging setup & パス規則 (`specs/SPEC-b9f5c4a1/quickstart.md`)
- [ ] T051 Add design notes/data model if needed for logging config (`specs/SPEC-b9f5c4a1/data-model.md`)

## Phase 7: console.log移行（既存コードの構造化ログ対応）

### Phase 7.1: 高優先度ファイル（111件）
- [ ] T060 [Migration] Migrate `src/index.ts` (11件) - printError/printInfo/printWarning を構造化ログと並行出力に
- [ ] T061 [Migration] Migrate `src/github.ts` (8件) - DEBUG_CLEANUP条件をlogger.debug()に統一
- [ ] T062 [Migration] Migrate `src/worktree.ts` (16件) - DEBUG_CLEANUP条件をlogger.debug()に統一
- [ ] T063 [Migration] Migrate `src/claude.ts` (30件) - ユーザー向け表示と構造化ログを分離
- [ ] T064 [Migration] Migrate `src/codex.ts` (20件) - ユーザー向け表示と構造化ログを分離
- [ ] T065 [Migration] Migrate `src/gemini.ts` (20件) - ユーザー向け表示と構造化ログを分離

### Phase 7.2: 中優先度ファイル（6件）
- [ ] T070 [Migration] Migrate `src/utils.ts` (3件) - ユーザー向けメッセージの分離
- [ ] T071 [Migration] Migrate `src/services/dependency-installer.ts` (1件) - 警告メッセージ
- [ ] T072 [Migration] Migrate `src/web/server/services/branches.ts` (2件) - Web UI情報

### Phase 7.3: 低優先度ファイル（28件）
- [ ] T080 [Migration] Migrate `src/config/index.ts` (5件) - DEBUG_* 環境変数をlogger.debug()に
- [ ] T081 [Migration] Migrate `src/claude-history.ts` (15件) - デバッグ出力
- [ ] T082 [Migration] Migrate `src/git.ts` (3件) - DEBUG条件をlogger.debug()に
- [ ] T083 [Migration] Migrate `src/cli/ui/hooks/useGitData.ts` (5件) - DEBUG条件をlogger.debug()に

## Dependencies
- Phase 1 → Phase 2 → US1 → US2 → US3 → Polish → Migration

## Parallel execution opportunities
- [P] T001 and T002 can run in parallel (deps independent)
- [P] T020 and T030 tests can be authored in parallel after foundational scaffolding
- [P] T060-T065 can run in parallel (independent files)
- [P] T070-T072 can run in parallel (independent files)
- [P] T080-T083 can run in parallel (independent files)
