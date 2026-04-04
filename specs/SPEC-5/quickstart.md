# Quickstart: SPEC-5 - Local SPEC Management

## Reviewer Flow
1. Open the management shell and verify `Specs` appears in the tab set.
2. Move into `Specs`, confirm local `metadata.json` entries populate the list, and use `Enter` / `Esc` to move between list and detail.
3. Use `Shift+Enter` from Specs detail and verify the wizard opens with the selected SPEC id/title/spec.md context prefilled and a title-derived branch seed.
4. Switch detail sections with `Left` / `Right` and confirm `analysis.md` is available alongside the other local SPEC artifacts.
5. From Specs detail, press `e` to edit phase, `s` to edit status, and confirm metadata saves on `Enter`.
6. On the `spec.md` detail tab, use `Up` / `Down` to select a `##` section and press `Ctrl+e`; confirm only the selected section body opens for editing and nested headings remain within that section.
7. On a non-`spec.md` artifact tab, press `Ctrl+e` and confirm the raw artifact file still opens as a full-file edit buffer.
8. Track semantic search, markdown-rendered detail parity, and the missing selection-menu UX for phase/status editing as the remaining execution steps.

## Repeatable Evidence
- `cargo test -p gwt-tui management_tab_labels -- --nocapture`
- `cargo test -p gwt-tui management_tab_all_has_nine_entries -- --nocapture`
- `cargo test -p gwt-tui load_initial_data_populates_specs_from_metadata -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_enter_opens_detail_and_escape_returns_list -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_left_right_cycle_sections_without_switching_tabs -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_shift_enter_opens_prefilled_wizard -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_e_starts_phase_edit_from_detail -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_s_starts_status_edit_from_detail -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_ctrl_e_starts_section_edit_from_detail -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_search_to_detail_then_e_starts_phase_edit -- --nocapture`
- `cargo test -p gwt-tui prepare_wizard_startup_prefills_spec_context_and_versions -- --nocapture`
- `cargo test -p gwt-tui detail_sections_constant_includes_analysis_md -- --nocapture`
- `cargo test -p gwt-tui start_section_edit_reads_analysis_markdown_file -- --nocapture`
- `cargo test -p gwt-tui save_edit_updates_status -- --nocapture`
- `cargo test -p gwt-tui move_down_in_spec_detail_cycles_markdown_headings -- --nocapture`
- `cargo test -p gwt-tui start_section_edit_on_spec_md_reads_selected_heading_body -- --nocapture`
- `cargo test -p gwt-tui save_section_edit_replaces_only_selected_heading -- --nocapture`
- `cargo test -p gwt-tui extract_markdown_section_preserves_nested_headings -- --nocapture`
- `cargo test -p gwt-tui replace_markdown_section_preserves_nested_headings -- --nocapture`
- `cargo test -p gwt-tui render_detail_spec_md_shows_selected_heading_hint -- --nocapture`
- `cargo test -p gwt-tui --lib`

## Expected Result
- The reviewer sees that local SPEC management is reachable from the live shell again.
- The reviewer sees that phase/status metadata edits and raw active-file edits are now reachable from Specs detail.
- The reviewer sees that `spec.md` section edits are scoped to the selected `##` section rather than replacing the full file.
- Any remaining gap is logged against the still-partial SPEC requirements rather than against stale unchecked task counts.
- No step should be treated as complete unless the code path is actually reachable today.
