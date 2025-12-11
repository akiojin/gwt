# Tasks: ログ運用統一（pino構造化ログ・7日ローテーション）

## Phase 1: Setup
- [ ] T001 Add pino dependency and lockfile update (`package.json`, `bun.lock`)  
- [ ] T002 Define default log path and env key names (`src/config/logging.ts`)

## Phase 2: Foundational (Blocking)
- [ ] T010 Create logger factory scaffolding with category support (`src/logging/logger.ts`)  
- [ ] T011 Implement rotation helper to delete files older than 7 days (`src/logging/rotation.ts`)

## Phase 3: US1 - 統一ロガー（構造化JSON + category）
- [ ] T020 [US1] Write unit tests for JSON structure & category field (`tests/logging/logger.test.ts`)
- [ ] T021 [US1] Implement pino logger factory to satisfy tests (`src/logging/logger.ts`)
- [ ] T022 [US1] Wire CLI entry to emit logs via logger with category `cli` (`src/index.ts`)

## Phase 4: US2 - ローテーション & 環境変数上書き
- [ ] T030 [US2] Write unit tests for rotation (7日超削除, 起動時実行) & env overrides (`tests/logging/rotation.test.ts`)
- [ ] T031 [US2] Integrate rotation into logger init path (`src/logging/rotation.ts`, `src/logging/logger.ts`)
- [ ] T032 [US2] Apply env-configurable level/path handling (`src/logging/logger.ts`)

## Phase 5: US3 - サーバー統合（Web UI/REST）
- [ ] T040 [US3] Integrate pino logger into Fastify server with category `server` (`src/web/server/index.ts`)
- [ ] T041 [US3] Ensure server startup/shutdown logs use structured logger, screen messages remain separate (`src/web/server/index.ts`)

## Phase 6: Polish & Docs
- [ ] T050 Update developer quickstart for logging setup & env vars (`specs/SPEC-b9f5c4a1/quickstart.md`)
- [ ] T051 Add design notes/data model if needed for logging config (`specs/SPEC-b9f5c4a1/data-model.md`)

## Dependencies
- Phase 1 → Phase 2 → US1 → US2 → US3 → Polish

## Parallel execution opportunities
- [P] T001 and T002 can run in parallel (deps independent)
- [P] T020 and T030 tests can be authored in parallel after foundational scaffolding
