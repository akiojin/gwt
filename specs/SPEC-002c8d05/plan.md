## Summary

Move Issue-tab spec detail rendering from body-canonical assumptions to artifact-first reconstruction while preserving legacy fallback. Search ownership stays in `#1643`, and local cache/linkage ownership stays in `#1714`.

## Technical Context

- Core detail loader: `crates/gwt-core/src/git/issue_spec.rs`
- Tauri bridge: `crates/gwt-tauri/src/commands/issue_spec.rs`
- Spec detail UI: `gwt-gui/src/lib/components/IssueSpecPanel.svelte`
- Issue detail entrypoint: `gwt-gui/src/lib/components/IssueListPanel.svelte`

## Constitution Check

- Spec before implementation: yes, detail-view contract is fixed here before code changes.
- Test-first: Rust and frontend regression tests must be added before logic changes.
- No workaround-first: preserve one canonical source preference (`doc:*`) with explicit fallback rather than duplicating parallel render paths.
- Minimal complexity: keep `SpecIssueDetail.sections` stable and absorb the change in backend reconstruction.

## Project Structure

- `gwt-core`: artifact parsing, section reconstruction, fallback rules
- `gwt-tauri`: command serialization for detail retrieval and artifact listing
- `gwt-gui`: Issue detail rendering and spec panel regression coverage

## Complexity Tracking

- Added complexity: `doc:*` artifact support in `SpecIssueArtifactKind` and section reconstruction
- Mitigation: no frontend payload shape change; old body parsing remains as fallback

## Phased Implementation

1. Add RED tests for index-only + `doc:*` detail rendering and legacy fallback.
2. Add `Doc` artifact handling and reconstruct `SpecIssueSections` from artifact comments first.
3. Update Tauri command serialization if needed for `doc` artifact exposure.
4. Verify `IssueSpecPanel` / `IssueListPanel` render the reconstructed sections without contract changes.
5. Keep search ownership in `#1643` and cache/linkage ownership in `#1714`.
