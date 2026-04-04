# Quickstart: SPEC-5 - Local SPEC Management

## Reviewer Flow
1. Open the management shell and verify `Specs` appears in the tab set.
2. Move into `Specs`, confirm local `metadata.json` entries populate the list, and use `Enter` / `Esc` to move between list and detail.
3. Use `Shift+Enter` from Specs detail and verify the wizard opens with the selected SPEC id/title/spec.md context prefilled and a title-derived branch seed.
4. Switch detail sections with `Left` / `Right` and confirm `analysis.md` is available alongside the other local SPEC artifacts.
5. Track semantic search, markdown-rendered detail parity, and live edit keypaths as the remaining execution steps.

## Repeatable Evidence
- `cargo test -p gwt-tui management_tab_labels -- --nocapture`
- `cargo test -p gwt-tui management_tab_all_has_nine_entries -- --nocapture`
- `cargo test -p gwt-tui load_initial_data_populates_specs_from_metadata -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_enter_opens_detail_and_escape_returns_list -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_left_right_cycle_sections_without_switching_tabs -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_shift_enter_opens_prefilled_wizard -- --nocapture`
- `cargo test -p gwt-tui prepare_wizard_startup_prefills_spec_context_and_versions -- --nocapture`
- `cargo test -p gwt-tui detail_sections_constant_includes_analysis_md -- --nocapture`
- `cargo test -p gwt-tui start_section_edit_reads_analysis_markdown_file -- --nocapture`
- `cargo test -p gwt-tui --lib`

## Expected Result
- The reviewer sees that local SPEC management is reachable from the live shell again.
- Any remaining gap is logged against the still-partial SPEC requirements rather than against stale unchecked task counts.
- No step should be treated as complete unless the code path is actually reachable today.
