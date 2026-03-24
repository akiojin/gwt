## Phase 1: Setup

- [ ] T001 Reconfirm #1327 as the storage/API canonical spec for Issue-first SPEC bundles.
- [ ] T002 Add RED tests for `doc:*` artifact parsing and mixed-mode precedence.

## Phase 2: Foundational

- [ ] T003 Add `Doc` to `SpecIssueArtifactKind`.
- [ ] T004 Reconstruct `SpecIssueDetail.sections` from `doc:*` artifacts first.
- [ ] T005 Preserve body-section fallback for legacy issues.

## Phase 3: Shared CRUD

- [ ] T006 Extend Tauri `issue_spec` command serialization for `doc` artifacts.
- [ ] T007 Verify builtin/MCP/tooling paths can reason about `doc`, `contract`, and `checklist` together.

## Phase 4: Migration

- [ ] T008 Extend migration scope to support body-canonical issue -> artifact-first issue conversion.
- [ ] T009 Keep local `specs/SPEC-*` migration as a supported path.

## Phase 5: Verification

- [ ] T010 Run targeted Rust and Tauri tests for artifact-first and legacy spec bundles.
- [ ] T011 Record results back into #1327.
