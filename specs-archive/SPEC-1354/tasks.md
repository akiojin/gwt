## Phase 1: Setup

- [ ] T001 Confirm #1354 as canonical spec for Issue detail rendering and #1643 as search-only reference.
- [ ] T002 Add RED tests describing artifact-first and legacy fallback behavior.

## Phase 2: Foundational

- [ ] T003 Extend `SpecIssueArtifactKind` to represent `doc` artifacts.
- [ ] T004 Rework `get_spec_issue_detail()` to reconstruct sections from `doc:*` artifacts first.
- [ ] T005 Preserve legacy body section parsing as fallback when `doc:*` artifacts are absent.

## Phase 3: User Story 1 / 2

- [ ] T006 Update `issue_spec` Tauri command/data mapping so detail retrieval remains stable for the frontend.
- [ ] T007 Add or update `IssueSpecPanel` tests for index-only spec issues backed by artifact comments.
- [ ] T008 Add or update `IssueListPanel` tests for spec detail routing with artifact-backed sections.

## Phase 4: User Story 3 / 4

- [ ] T009 Verify legacy body-canonical `gwt-spec` issues still render through the same UI path.
- [ ] T010 Update #1643 so it references #1354 for detail-view behavior instead of owning it.

## Phase 5: Polish / Verification

- [ ] T011 Run targeted Rust tests for `issue_spec` and Tauri `issue_spec` commands.
- [ ] T012 Run frontend tests for `IssueSpecPanel` and `IssueListPanel`.
- [ ] T013 Record verification results back into #1354.
