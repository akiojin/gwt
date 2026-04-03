# Quickstart: SPEC-5 - Local SPEC Management

## Reviewer Flow
1. Open the management shell and verify `Specs` appears in the tab set.
2. Move into `Specs`, confirm local `metadata.json` entries populate the list, and use `Enter` / `Esc` to move between list and detail.
3. Use `Shift+Enter` from Specs detail and verify the wizard opens with the selected SPEC id/title prefilled.
4. Track semantic search and persistent editing as the remaining execution steps.

## Repeatable Evidence
- `cargo test -p gwt-tui management_tab_labels_include_specs -- --nocapture`
- `cargo test -p gwt-tui load_initial_data_populates_specs_from_metadata -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_enter_opens_detail_and_escape_returns_list -- --nocapture`
- `cargo test -p gwt-tui route_key_to_management_specs_shift_enter_opens_prefilled_wizard -- --nocapture`
- `cargo test -p gwt-tui --lib`

## Expected Result
- The reviewer sees that local SPEC management is reachable from the live shell again.
- Any missing behavior is logged against the remaining `25` unchecked tasks.
- No step should be treated as complete unless the code path is actually reachable today.
