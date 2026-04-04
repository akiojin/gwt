# Progress: SPEC-5 - Local SPEC Management

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `50/50` checked in `tasks.md`
- Artifact refresh: `2026-04-04T04:59:40Z`

## Done
- The management shell now exposes a live Specs tab again.
- Startup loading now reads local `specs/SPEC-*/metadata.json` into `model.specs` in sorted SPEC order.
- The live Specs tab now supports `Enter` to open detail, `Esc` to return to the list, and `Shift+Enter` to open the wizard with SPEC id/title/spec.md prefill and a title-derived branch seed.
- Specs detail now exposes `analysis.md` alongside the other local artifact tabs, and `Left` / `Right` now cycle detail sections in the live shell instead of switching management tabs.
- Specs detail now exposes live edit keypaths for metadata and content edits: `e` starts phase selection, `s` starts status selection, `Ctrl+e` edits the selected `spec.md` section body, and `E` opens a raw file edit buffer.
- Phase/status metadata editing now uses constrained selection menus in detail view, so `Up` / `Down` choose a valid metadata value and `Enter` persists it into `metadata.json`.
- `spec.md` detail now supports section-scoped editing with stable targeting: `Up` / `Down` select parsed `##` sections, fenced-code pseudo-headings are ignored, duplicate section titles are disambiguated by section order, and save now fails instead of appending a duplicate section when the selected section disappears.
- Read-only Specs detail now routes `spec.md` section bodies and the other artifact tabs through the shared markdown renderer, so headings and list bullets render consistently in-place.
- Specs search now ranks local metadata plus artifact-body hits, preserves detail/launch selection through search-result order, renders score + snippet inline in the list, and ignores `/` while detail view is open.
- Tasks marked obsolete in `tasks.md` are now explicitly tracked as replaced by
  the live shell flow, `gwt-spec-search`, or simplified metadata editing.
- The execution task list is fully checked, and there is no remaining
  code-side implementation gap tracked in `spec.md`.

## Next
- Run the reviewer flow on the current branch and capture completion-gate
  evidence before any `Done` transition.
