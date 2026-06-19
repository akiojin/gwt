# Discussions

This file is the canonical gwt discussion log. Entries are updated in place while active and indexed by the `discussions` semantic scope.

## 2026-06-19 — Codex resume panes show Error during session restore

Status: completed
Topics: gwt-discussion, session-resume, codex, pane-status, workspace-restore
Related SPECs: #2359, #2014, #1935
Related Issues: #2546, #2995
Related Works:
Promoted To: SPEC #2359 US-79B / T-651〜T-655

Summary:
セッション復元時に Codex pane が `CODEX Codex Error` や blank terminal になり、復元できないように見える場合がある。添付画像では複数の resumed Codex pane が Error 表示になっている一方、対応する session TOML / runtime sidecar / OS process では `codex resume <agent_session_id>` が生存している。現時点では「resume プロセスが起動できない」よりも、「live agent session があるのに stale な PTY Error state が hook Running state より優先され、GUI 表示だけ Error 固定になる」可能性が高い。

Evidence:
- 添付画像では `work/20260617-0255` と `work/20260617-0425` 系の Codex pane が Error/blank 表示。
- `~/.gwt/sessions/*` では、それぞれ `session_mode = "Resume"` かつ `agent_session_id` 付きで Running sidecar が存在。
- `ps` では `/Users/akiojin/.bun/bin/codex --no-alt-screen resume <agent_session_id>` が該当セッションで生存。
- 対象 worktree / branch は存在しており、SPEC #2359 US-79 の「missing worktree/branch」ケースとは現在のスクリーンショット上では一致しない。
- `gwtd pane.list` は pane websocket timeout になり、UI/pane 更新系の詰まりも併発している可能性がある。
- `crates/gwt/src/window_state.rs` の `compose_window_state_with_active_session` は PTY state が `Error` の場合、Agent hook state を見ずに PTY state を返す。
- `crates/gwt/src/app_runtime/runtime_events.rs` は PTY status `Error` / `Stopped` で active agent session tracking と runtime tracking を落とすため、その後の hook event が live session を GUI 上で復旧しにくい。

Approaches:
- Approach 1: Agent window に active session と matching live runtime hook がある場合、PTY Error より hook state を優先する。実際の process exit と stale UI state の境界を test で固定できるが、auto-close / stopped 表示の既存期待を壊さないよう Stopped は対象外にする必要がある。
- Approach 2: hook Running/Waiting/Idle を受けたタイミングで同じ window の PTY terminal state を clear する。局所的だが、status composition の意図が分散する。
- Approach 3: pane websocket timeout / render blank の retry と status refresh を追加する。表示更新の堅牢性は上がるが、Error 固定の根本原因を直接直さない。

Current Recommendation:
最小の修正対象は Approach 1 を中心に、Agent resume pane の status composition と runtime tracking を SPEC #2359 の resume/restore follow-up として扱うこと。Startup restore が missing worktree を skip する `crates/gwt/src/app_runtime/startup.rs` の US-79 ギャップは関連するが、今回の画像とは別症状として scope を分ける。

Open Questions:
- なし。

Next:
SPEC #2359 US-79B / T-651〜T-655 は完了。ユーザー視覚確認は 2026-06-19 に confirmed。

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
