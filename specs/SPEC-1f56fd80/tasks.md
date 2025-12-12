# Tasks: Web UI システムトレイ統合とURL表示

**仕様ID**: `SPEC-1f56fd80`

## Phase 1: Setup

- [x] T001 Create SPEC directory structure (`specs/SPEC-1f56fd80/`)
- [x] T002 Create spec.md with full specification
- [x] T003 Create plan.md with implementation plan
- [x] T004 Create tasks.md (this file)

## Phase 2: TDD - Test First

- [ ] T010 Add tests for system tray initialization
- [ ] T011 Add tests for tray double‑click opening URL
- [ ] T012 Add tests for BranchListScreen URL line

## Phase 3: Implementation

- [ ] T020 Add lightweight tray dependency and wrapper module
- [ ] T021 Hook tray startup into Web UI server
- [ ] T022 Show Web UI URL in BranchListScreen

## Phase 4: Verification

- [ ] T030 Run unit tests and fix failures

## Dependencies

```
T001 → T002 → T003 → T004
T004 → T010‑T012
T010‑T012 → T020‑T022
T022 → T030
```

