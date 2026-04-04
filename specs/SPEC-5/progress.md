# Progress: SPEC-5 - Local SPEC Management

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `43/43` checked in `tasks.md`
- Artifact refresh: `2026-04-04T04:27:18Z`

## Done
- The management shell now exposes a live Specs tab again.
- Startup loading now reads local `specs/SPEC-*/metadata.json` into `model.specs` in sorted SPEC order.
- The live Specs tab now supports `Enter` to open detail, `Esc` to return to the list, and `Shift+Enter` to open the wizard with SPEC id/title/spec.md prefill and a title-derived branch seed.
- Specs detail now exposes `analysis.md` alongside the other local artifact tabs, and `Left` / `Right` now cycle detail sections in the live shell instead of switching management tabs.
- Specs detail now exposes live edit keypaths for metadata and raw file edits: `e` starts phase editing, `s` starts status editing, and `Ctrl+e` opens a raw edit buffer for the active artifact file.
- `spec.md` detail now supports section-scoped editing: `Up` / `Down` select `##` sections, the selected heading is shown in the detail view, and `Ctrl+e` edits only that section body while preserving nested headings and neighboring sections.
- Tasks marked obsolete in `tasks.md` are now explicitly tracked as replaced by
  the live shell flow, `gwt-spec-search`, or simplified metadata editing.
- The execution task list is fully checked, but `spec.md` still intentionally
  records remaining gaps around semantic search, markdown-rendered detail
  parity, and the missing selection-menu UX for phase/status editing.

## Next
- Decide whether to implement or explicitly de-scope semantic search,
  markdown-rendered detail parity, and the selection-menu UX for phase/status editing.
- Re-run the reviewer flow on the current branch after that scope decision and
  before any `Done` transition.
