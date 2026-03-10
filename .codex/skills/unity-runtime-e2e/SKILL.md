---
name: unity-runtime-e2e
description: Run real Unity runtime E2E checks with unity-cli by entering Play Mode, driving input, inspecting scene/UI state, and capturing screenshots. Use this when verifying user-visible Unity behavior beyond EditMode/PlayMode Test Runner only.
---

# Unity Runtime E2E

Use this skill when a Unity change affects visible behavior and Test Runner coverage is not enough.

## Goal

Validate actual runtime behavior in the Unity Editor by:
- entering Play Mode
- driving keyboard/mouse/UI actions
- inspecting scene/UI/GameObject state
- capturing screenshots when useful

This is not the same as only running `EditMode` / `PlayMode` tests.

## Mandatory sequence

1. Clear stale console noise and verify compile state

```bash
cd gwt/gwt
unity-cli raw clear_console --json '{}'
unity-cli raw get_compilation_state --json '{}'
```

Require:
- `errorCount == 0`
- `consoleErrorCount == 0`

2. Enter Play Mode

```bash
cd gwt/gwt
unity-cli raw play_game --json '{}'
unity-cli raw get_editor_state --json '{}'
```

Require `isPlaying == true` before continuing.

3. Drive the actual user flow

Examples:

```bash
cd gwt/gwt
unity-cli raw input_keyboard --json '{"key":"backquote","action":"press"}'
unity-cli raw find_gameobject --json '{"namePattern":"ProjectSwitchOverlayPanel","includeInactive":true}'
unity-cli raw find_ui_elements --json '{"namePattern":"Project Switcher","includeInactive":true}'
unity-cli raw get_gameobject_details --json '{"path":"/Canvas/ProjectSwitchOverlayPanel"}'
unity-cli raw capture_screenshot --json '{"captureMode":"game","width":1280,"height":720}'
```

4. Exit Play Mode

```bash
cd gwt/gwt
unity-cli raw stop_game --json '{}'
unity-cli raw get_editor_state --json '{}'
```

## Useful commands

### Play control

```bash
unity-cli raw play_game --json '{}'
unity-cli raw pause_game --json '{}'
unity-cli raw stop_game --json '{}'
unity-cli raw get_editor_state --json '{}'
```

### Runtime input

```bash
unity-cli raw input_keyboard --json '{"key":"space","action":"press"}'
unity-cli raw input_mouse --json '{"action":"click","button":"left","x":400,"y":300}'
unity-cli raw create_input_sequence --json '{"sequence":[{"type":"keyboard","params":{"action":"press","key":"backquote"}}],"delayBetween":100}'
```

### Runtime inspection

```bash
unity-cli raw find_gameobject --json '{"namePattern":"UIManager","includeInactive":true}'
unity-cli raw get_gameobject_details --json '{"path":"/UIRoot/UIManager"}'
unity-cli raw analyze_scene_contents --json '{}'
unity-cli raw find_ui_elements --json '{"namePattern":"Project Switcher","includeInactive":true}'
unity-cli raw get_ui_element_state --json '{"elementPath":"/Canvas/SomeButton"}'
```

### Media capture

```bash
unity-cli raw capture_screenshot --json '{"captureMode":"game","width":1280,"height":720}'
unity-cli raw analyze_screenshot --json '{"imagePath":"<returned path>"}'
```

## Reporting

When reporting verification:
- separate `Test Runner` results from `runtime E2E` results
- state the exact runtime flow you executed
- mention whether verification was done by scene/UI inspection, screenshot, or both

## Rules

- Do not claim E2E coverage if you only ran Unity Test Runner.
- Run `EditMode` / `PlayMode` tests separately from runtime E2E.
- Prefer one concrete, user-visible flow over many shallow checks.
