## Findings

- #1579 now owns the embedded skill workflow, storage/API, and completion gate as a single canonical spec.
- `SPEC-1776` owns local SPEC viewing in the `SPECs` tab. `#1354` only covers GitHub Issue detail / legacy issue-body compatibility.
- #1643 owns search/discovery only.
- Migration is currently implemented only for local specs and needs to be treated as a broader redesign concern.
- Current `issue_spec.rs` still documents the Issue body as canonical.
- Artifact comment parsing already exists but only models contract/checklist kinds.
- Viewer code should not absorb storage complexity; storage/API should normalize data before it reaches the UI.
- Migration must handle both repo-local legacy specs and existing GitHub issue-body bundles.
- #1654 demonstrates that the current workflow can falsely converge on `tasks.md complete` while the implementation still diverges from `doc:spec.md`.
- Workflow-owned checklist quality must be part of the completion gate fix.

## References

- [The Complete Guide to Building Skills for Claude](https://resources.anthropic.com/hubfs/The-Complete-Guide-to-Building-Skill-for-Claude.pdf) — Anthropic 公式スキル設計ガイド。description フィールドの `[What] + [When] + [Capabilities]` 構造、トリガーフレーズの記載要件はこのガイドに準拠する。
