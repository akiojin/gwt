# Feature Specification: Issue-first SPEC storage and artifact CRUD

## Background

- `issue_spec.rs` and the Tauri `issue_spec` commands are the backend contract behind gwt's SPEC registration, retrieval, and editing behavior.
- The older design treated the GitHub Issue body as the canonical spec bundle with `Spec/Plan/Tasks/TDD/...` sections embedded directly in the body.
- The embedded skill workflow is now being restructured around artifact-first storage where `doc:*`, `contract:*`, and `checklist:*` issue comments hold the real content and the Issue body is only an index.
- This issue is the canonical storage/API spec for that change. Detail rendering behavior is owned by #1354; workflow and registration behavior is owned by #1579.

## User Stories

### User Story 1 - Persist spec artifacts without relying on a monolithic body (Priority: P0)

As a developer, I want the spec system to store `spec.md`, `plan.md`, `tasks.md`, and supporting docs as first-class artifact comments rather than a giant Issue body.

**Acceptance Scenarios**

1. Given a new `gwt-spec` Issue, when it is created or updated, then the Issue body may be index-only and the canonical content lives in `doc:*` comments.
2. Given a `gwt-spec` Issue with `doc:*` comments, when backend detail APIs are called, then they reconstruct `SpecIssueSections` from those artifacts.

### User Story 2 - Keep legacy issues readable during migration (Priority: P0)

As a developer, I want legacy body-canonical spec issues to continue working while the system migrates to artifact-first storage.

**Acceptance Scenarios**

1. Given a legacy `gwt-spec` Issue with body sections only, when detail APIs are called, then the same sections are returned.
2. Given a mixed issue with both body sections and `doc:*` comments, when detail APIs are called, then `doc:*` comments take precedence and body sections act as fallback.

### User Story 3 - Manage document artifacts through the same CRUD layer (Priority: P0)

As a developer or agent, I want `doc:*`, `contract:*`, and `checklist:*` artifacts to use the same list/get/upsert/delete model.

**Acceptance Scenarios**

1. Given an artifact key `doc:plan.md`, when it is upserted, then the system stores it as a comment artifact with stable retrieval metadata.
2. Given an artifact key `contract:openapi.yaml` or `checklist:tdd.md`, when it is listed, then it is returned through the same API family.

### User Story 4 - Support migration tooling for old formats (Priority: P1)

As a maintainer, I want migration tooling to handle both local legacy specs and old Issue-body bundles.

**Acceptance Scenarios**

1. Given local `specs/SPEC-*`, when migration runs, then it can create Issue-first artifacts.
2. Given an existing body-canonical `gwt-spec` Issue, when migration or repair runs, then it can split the body into `doc:*` and `checklist:*` artifact comments.

## Edge Cases

- A spec issue has no `doc:*` comments and only body sections.
- A spec issue has partial `doc:*` coverage (for example `doc:spec.md` exists but `doc:plan.md` does not).
- Artifact comments use marker format or legacy prefix format.
- Consumers request only contract/checklist artifacts while `doc:*` artifacts also exist.

## Functional Requirements

- **FR-001**: `SpecIssueArtifactKind` must support `doc`, `contract`, and `checklist` artifact families.
- **FR-002**: `get_spec_issue_detail()` must reconstruct `SpecIssueSections` from `doc:*` artifacts first and body sections second.
- **FR-003**: `SpecIssueDetail.sections` must remain the stable frontend-facing aggregate shape.
- **FR-004**: `list_spec_issue_artifact_comments` and related CRUD APIs must support `doc:*` artifacts alongside existing contract/checklist artifacts.
- **FR-005**: Legacy body-canonical `gwt-spec` Issues must remain readable until migration is complete.
- **FR-006**: The canonical body format for new `gwt-spec` Issues must be an artifact index plus status/links, not a full bundle dump.
- **FR-007**: Migration tooling must cover both `specs/SPEC-* -> Issue-first` and `body-canonical issue -> artifact-first issue` flows.

## Non-Functional Requirements

- **NFR-001**: Artifact-first migration must not require a breaking frontend payload change.
- **NFR-002**: Existing closed SPEC issues remain readable without manual repair.
- **NFR-003**: Rust/Tauri regression tests must cover artifact-first, legacy, and mixed-mode issues.

## Success Criteria

- **SC-001**: The backend can read index-only spec issues backed by `doc:*` comments.
- **SC-002**: The backend can still read legacy body-canonical spec issues.
- **SC-003**: Artifact CRUD supports `doc`, `contract`, and `checklist` consistently.
- **SC-004**: Migration scope explicitly covers old local specs and old body-canonical issues.
