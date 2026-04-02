# Issue タブ — GitHub Issue 一覧・詳細・フルフロー

## Background

- GitHub Issue list/detail, GFM rendering, and `Work on this` flow were already implemented under #1354.
- The embedded spec workflow is now moving to artifact-first storage where `doc:*` issue comments are canonical and the Issue body is only an index.
- Today, `issue_spec.rs` and `IssueSpecPanel` still assume that the full spec bundle lives in the Issue body, so index-only spec issues can render as empty or misleading detail views.
- Search canonical ownership remains with `#1643`.
- Issue title lookup, local cache, and GitHub linkage are provided by `#1714`.
- This issue owns the Issue tab list/detail/full-flow contract only.

## User Stories

### User Story 1 - Keep normal Issue detail unchanged (Priority: P0)

As a developer, I want normal GitHub Issues to keep rendering through the existing GFM detail path.

**Acceptance Scenarios**

1. Given a non-spec Issue, when I open its detail view, then title/body/meta information still render as before.
2. Given a non-spec Issue, when I return to list view, then filters and selection flow remain unchanged.

### User Story 2 - View artifact-first spec issues from the Issue tab (Priority: P0)

As a developer, I want `spec`-labeled issues with index-only bodies to render their real content from `doc:*` artifact comments.

**Acceptance Scenarios**

1. Given a `gwt-spec` Issue whose body only contains `Artifact Index`, when I open detail view, then `IssueSpecPanel` shows `spec/plan/tasks/research/data-model/quickstart` reconstructed from `doc:*` comments.
2. Given a `gwt-spec` Issue with `contracts/checklists` comments, when I open detail view, then those sections are still visible to the user.

### User Story 3 - Preserve legacy spec viewing (Priority: P0)

As a developer, I want older body-canonical spec issues to keep rendering correctly during migration.

**Acceptance Scenarios**

1. Given a legacy `gwt-spec` Issue with `## Spec/Plan/Tasks/...` in the body, when I open detail view, then the same sections still render.
2. Given a partially migrated spec issue, when both body sections and `doc:*` comments exist, then `doc:*` comments win and legacy body acts only as fallback.

### User Story 4 - Keep Issue tab responsibility boundaries clear (Priority: P1)

As a developer, I want the Issue tab to own detail rendering while `#1643` remains search-only.

**Acceptance Scenarios**

1. Given the canonical spec set, when an implementer reads it, then detail rendering requirements are found in `#1354`, not `#1643`.
2. Given the search feature spec, when an implementer reads `#1643`, then it points to `#1354` for detail-view behavior.
3. Given local cache / linkage changes, when an implementer reads Issue title lookup dependencies, then it points to `#1714`.

## Edge Cases

- A `gwt-spec` Issue has an index-only body but is missing one or more `doc:*` artifacts.
- A `gwt-spec` Issue contains both legacy body sections and new `doc:*` artifact comments.
- A spec issue has `contract:*` and `checklist:*` comments but no `doc:*` comments.
- Artifact comments exceed simple `contract/checklist` assumptions and include `doc:data-model.md`, `doc:quickstart.md`, or `checklist:tdd.md`.

## Functional Requirements

- **FR-001**: Normal Issue detail rendering in `IssueListPanel` must remain unchanged.
- **FR-002**: `gwt-spec` Issues with index-only bodies must be rendered from `doc:*` artifact comments rather than body sections.
- **FR-003**: `get_spec_issue_detail()` must prefer `doc:*` artifacts over legacy body sections when both are present.
- **FR-004**: Legacy body-canonical `gwt-spec` Issues must still render correctly as fallback.
- **FR-005**: `contracts` and `checklists` artifact comments must remain visible through the detail API and UI.
- **FR-006**: The public `SpecIssueDetail.sections` shape consumed by the frontend must remain stable.
- **FR-007**: Search canonical ownership is `#1643`; detail rendering canonical ownership is `#1354`.
- **FR-008**: Issue title lookup and local cache dependencies are defined by `#1714`.

## Non-Functional Requirements

- **NFR-001**: The change must be backward compatible for already closed `gwt-spec` Issues.
- **NFR-002**: The frontend must not need a breaking data contract change to adopt artifact-first specs.
- **NFR-003**: Rust and frontend regression tests must cover both artifact-first and legacy spec issues.

## Success Criteria

- **SC-001**: A `gwt-spec` Issue with index-only body plus `doc:*` comments renders correctly in the Issue tab.
- **SC-002**: A legacy body-canonical `gwt-spec` Issue still renders correctly in the Issue tab.
- **SC-003**: `IssueSpecPanel` and `IssueListPanel` regression tests cover both paths and pass.
- **SC-004**: The canonical ownership split is clear: `#1643` for search, `#1354` for detail rendering, `#1714` for local issue cache/linkage.
