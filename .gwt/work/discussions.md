# Discussions

This file is the canonical gwt discussion log. Entries are updated in place while active and indexed by the `discussions` semantic scope.

## 2026-05-23 — Workspace terminology and durable discussions

Status: active
Topics: workspace, work, discussion, semantic-search
Related SPECs: #2359
Related Works:
Promoted To:

Summary:
Workspace の意味が分かりにくい。Branch は作業空間、SPEC は仕様、Work は永続する作業単位として整理する。議論フェーズは Work ではなく Discussion として扱い、memory と同じくファイルへ保存し、セマンティック検索できるようにする。

Decisions:
- Discussion is not Work.
- Work remains durable until completion and can be persisted with completed status.
- A Work can cover multiple SPECs, and concrete tasks may be undecided at creation time.
- Past Work and Discussion records should be semantically searchable and can surface similar candidates during conversation.

Open Questions:
- Workspace という語を UI 上で Project State / Work / Discussion / Branch とどう分けると直感的か。

Next:
実データとして discussion log を保存し、discussions semantic index で検索できることを確認する。

## 2026-06-17 — Managed Hooks UX 5x follow-up

Status: chosen
Topics: managed-hooks, gwt-discussion, workflow-policy, hook-health
Related SPECs: #1935, #3050, #1942, #2077
Related Works:
Promoted To: #1935 Phase 22

Summary:
User selected all UX axes. SPEC #1935 now owns Phase 22: Managed Hooks Orchestrator UX. Agent/Work hook health strip is the primary user-facing surface; Hook Center/Settings and CLI/Board hook status/doctor are supporting surfaces backed by the same health model. Speed/quietness and safety are acceptance criteria, not separate tracks.

Decisions:
- Adopt all three axes: integrated UX, speed/quietness, and safety.
- Use SPEC #1935 as the owner and append Phase 22 to spec, plan, and tasks instead of creating a new SPEC.
- Primary surface: Agent/Work hook health strip. Supporting surfaces: Hook Center/Settings audit and CLI/Board hook status/doctor.
- Keep diagnostics out of hook stdout; expose health, profile, and recovery through explicit status surfaces.
- Treat stale binary/trust/asset recovery, linked-worktree Codex discovery, and delayed SessionStart as first-class UX states.
- Tighten workflow-policy safety as implementation-mutation owner readiness, while preserving read-only exploration and explicit low-risk exceptions.

Open Questions:
- None for planning. Implementation may split surfaces if TDD shows a narrower vertical slice is safer.

Next:
Action Bundle: run gwt-build-spec for #1935 Phase 22. Start with T-HUX2-001 through T-HUX2-006: RED tests and backend ManagedHookHealth read model before UI work.
