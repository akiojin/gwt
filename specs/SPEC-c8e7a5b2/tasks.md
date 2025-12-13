# Tasks: CLI起動時Web UIサーバー自動起動

**仕様ID**: `SPEC-c8e7a5b2`

## Phase 1: Setup

- [x] T001 Create SPEC directory structure (`specs/SPEC-c8e7a5b2/`)
- [x] T002 Create spec.md with full specification
- [x] T003 Create plan.md with implementation plan
- [x] T004 Create tasks.md (this file)

## Phase 2: TDD - Test First

- [x] T010 Create test file scaffolding (`tests/unit/index.webui-startup.test.ts`)
- [x] T011 Write test: startWebServerが呼び出される
- [x] T012 Write test: printInfoでデフォルトポート3000が表示される
- [x] T013 Write test: PORT環境変数がメッセージに反映される
- [x] T014 Write test: エラー時にappLogger.warnが呼び出される
- [x] T015 Write test: エラー時もmain()が正常完了する（CLIが継続）
- [x] T016 Write test: Gitリポジトリ外ではサーバー起動しない

## Phase 3: Verification

- [x] T020 Run tests to verify existing implementation passes (6/6 passed)
- [x] T021 Fix any test failures (Ink/React mocking issue fixed)

## Phase 4: Commit

- [x] T030 Commit SPEC files and test file
- [x] T031 Push to remote

## Dependencies

```
T001 → T002 → T003 → T004 (SPEC files in order)
T004 → T010 (spec before tests)
T010 → T011-T016 (scaffolding before test cases)
T011-T016 → T020 (tests before verification)
T020 → T030 → T031 (verification before commit)
```

## Parallel execution opportunities

- T011-T016 can be written in parallel after T010
