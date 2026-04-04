# Quickstart: SPEC-5 - Local SPEC Management

## Reviewer Flow
1. Open the management shell and verify `Specs` appears in the tab set.
2. Move into `Specs`, confirm local `metadata.json` entries populate the list, and use `Enter` / `Esc` to move between list and detail.
3. Use `Shift+Enter` from Specs detail and verify the wizard opens with the selected SPEC id/title/spec.md context prefilled and a title-derived branch seed.
4. Switch detail sections with `Left` / `Right` and confirm `analysis.md` is available alongside the other local SPEC artifacts.
5. From Specs detail, press `e` to edit phase or `s` to edit status, use `Up` / `Down` to choose a value, and confirm metadata saves on `Enter`.
6. On the `spec.md` detail tab, use `Up` / `Down` to select a `##` section and press `Ctrl+e`; confirm only the selected section body opens for editing and nested headings remain within that section.
7. On the `spec.md` detail tab, press `E` and confirm the raw `spec.md` file still opens as a full-file edit buffer.
8. On a non-`spec.md` artifact tab, press `Ctrl+e` and confirm the raw artifact file still opens as a full-file edit buffer.
9. Move between `analysis.md`, `plan.md`, and `tasks.md` in read-only detail and confirm headings/lists render with markdown styling instead of raw plaintext bullets.
10. Return to the Specs list, press `/`, enter a free-text query that exists only in an artifact body, and confirm ranked results show a relevance score plus snippet while `Enter` still opens the matched SPEC detail.

## Repeatable Evidence
- `cargo test -p gwt-tui management_tab_labels -- --nocapture`
- `cargo test -p gwt-tui management_tab_all_has_nine_entries -- --nocapture`
- `cargo test -p gwt-tui load_initial_data_populates_specs_from_metadata -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_enter_opens_detail_and_escape_returns_list -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_left_right_cycle_sections_without_switching_tabs -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_shift_enter_opens_prefilled_wizard -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_e_starts_phase_edit_from_detail -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_s_starts_status_edit_from_detail -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_down_cycles_phase_selection_menu -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_ctrl_e_starts_section_edit_from_detail -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_search_to_detail_then_e_starts_phase_edit -- --nocapture`
- `cargo test -p gwt-tui prepare_wizard_startup_prefills_spec_context_and_versions -- --nocapture`
- `cargo test -p gwt-tui detail_sections_constant_includes_analysis_md -- --nocapture`
- `cargo test -p gwt-tui start_section_edit_reads_analysis_markdown_file -- --nocapture`
- `cargo test -p gwt-tui move_down_while_editing_phase_cycles_selection_menu_value -- --nocapture`
- `cargo test -p gwt-tui move_down_while_editing_status_cycles_selection_menu_value -- --nocapture`
- `cargo test -p gwt-tui render_detail_phase_edit_shows_selection_menu_hint -- --nocapture`
- `cargo test -p gwt-tui save_edit_updates_status -- --nocapture`
- `cargo test -p gwt-tui move_down_in_spec_detail_cycles_markdown_headings -- --nocapture`
- `cargo test -p gwt-tui start_section_edit_on_spec_md_reads_selected_heading_body -- --nocapture`
- `cargo test -p gwt-tui save_section_edit_replaces_only_selected_heading -- --nocapture`
- `cargo test -p gwt-tui save_section_edit_uses_selected_duplicate_heading_index -- --nocapture`
- `cargo test -p gwt-tui save_section_edit_errors_when_selected_section_disappears -- --nocapture`
- `cargo test -p gwt-tui extract_markdown_section_preserves_nested_headings -- --nocapture`
- `cargo test -p gwt-tui replace_markdown_section_preserves_nested_headings -- --nocapture`
- `cargo test -p gwt-tui markdown_section_headings_ignores_fenced_code_blocks -- --nocapture`
- `cargo test -p gwt-tui render_detail_spec_md_shows_selected_heading_hint -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_shift_e_starts_raw_file_edit_from_detail -- --nocapture`
- `cargo test -p gwt-tui render_detail_analysis_md_uses_markdown_bullet_rendering -- --nocapture`
- `cargo test -p gwt-tui render_detail_spec_md_section_uses_markdown_bullet_rendering -- --nocapture`
- `cargo test -p gwt-tui render_lines_with_prelude_inserts_separator_before_markdown -- --nocapture`
- `cargo test -p gwt-tui filtered_specs_ranks_artifact_hits_above_metadata_only_hits -- --nocapture`
- `cargo test -p gwt-tui selected_spec_uses_search_result_order -- --nocapture`
- `cargo test -p gwt-tui render_search_results_shows_score_and_snippet -- --nocapture`
- `cargo test -p gwt-tui search_start_ignored_while_detail_view_is_open -- --nocapture`
- `cargo test -p gwt-tui --lib`

## Expected Result
- The reviewer sees that local SPEC management is reachable from the live shell again.
- The reviewer sees that phase/status metadata edits now use a constrained selection menu, and that selected-section `spec.md` edits plus raw active-file edits are all reachable from Specs detail.
- The reviewer sees that `spec.md` section edits are scoped to the selected `##` section, survive duplicate headings, ignore fenced-code pseudo-headings, and fail loudly instead of appending a duplicate section when the target section disappears.
- The reviewer sees that read-only artifact detail renders headings and list bullets through the shared markdown widget instead of plain text.
- The reviewer sees that free-text Specs search ranks local metadata and artifact-body hits, shows score + snippet, and keeps detail navigation intact.
- Any remaining gap is logged against completion-gate review rather than stale execution gaps.
- No step should be treated as complete unless the code path is actually reachable today.
