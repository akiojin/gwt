# Progress: SPEC-5 - Local SPEC Management

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `43/43` checked in `tasks.md`
- Artifact refresh: `2026-04-04T04:31:56Z`

## Done
- The management shell now exposes a live Specs tab again.
- Startup loading now reads local `specs/SPEC-*/metadata.json` into `model.specs` in sorted SPEC order.
- The live Specs tab now supports `Enter` to open detail, `Esc` to return to the list, and `Shift+Enter` to open the wizard with SPEC id/title prefill.
- Specs detail now exposes `analysis.md` alongside the other local artifact tabs, and `Left` / `Right` now cycle detail sections in the live shell instead of switching management tabs.
- Tasks marked obsolete in `tasks.md` are now explicitly tracked as replaced by
  the live shell flow, `gwt-spec-search`, or simplified metadata editing.
- The execution task list is fully checked, but `spec.md` still intentionally
  records remaining gaps around semantic search, markdown-rendered detail
  parity, live edit keypaths for metadata/content changes, and richer SPEC
  context / branch-name parity for agent launch.

## Next
- Decide whether to implement or explicitly de-scope semantic search,
  markdown-rendered detail parity, live edit keypaths, and richer launch
  context / branch-name parity.
- Re-run the reviewer flow on the current branch after that scope decision and
  before any `Done` transition.
