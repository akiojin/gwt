# SPEC-7: Settings and Profiles -- Implementation Plan

## Phase 1: Voice Settings UI

**Goal**: Add a Voice category to the Settings screen with all 5 configurable fields.

### Approach

Extend the existing `SettingsCategory` enum in `crates/gwt-tui/src/screens/settings.rs` with a `Voice` variant. Follow the same pattern used by General, Worktree, Agent, CustomAgents, Environment, and AISettings categories.

### Key Changes

1. **gwt-core**: Add `VoiceConfig` struct to the configuration model (`config.rs`).
   - Fields: `model_path: Option<PathBuf>`, `hotkey: String`, `input_device: String`, `language: String`, `enabled: bool`.
   - Default values: `hotkey = "Ctrl+G,v"`, `input_device = "system_default"`, `language = "auto"`, `enabled = false`.
   - Serialize/deserialize under `[voice]` section in `config.toml`.

2. **gwt-tui**: Add `Voice` variant to `SettingsCategory` enum.
   - Render 5 fields with appropriate input widgets (text input, toggle, dropdown).
   - Navigation: Voice appears as the 7th category in the sidebar.

3. **gwt-tui**: Wire save/load for voice settings through the existing settings persistence layer.

### Dependencies

- Existing settings infrastructure (category rendering, config persistence).

## Phase 2: Voice Settings Validation

**Goal**: Validate voice settings values before persisting.

### Key Changes

1. **gwt-core**: Add `VoiceConfig::validate()` method.
   - Check `model_path` exists on disk and is a directory.
   - Check `input_device` against system audio device list (platform-specific).
   - Check `hotkey` does not conflict with reserved keybindings.

2. **gwt-tui**: Display validation errors inline in the Voice settings form.
   - Red text below the invalid field.
   - Prevent save if validation fails.

3. **Platform abstraction**: Audio device enumeration.
   - macOS: Query `AVFoundation` audio input devices via `coreaudio-rs` or system command.
   - Linux: Query PulseAudio/ALSA device list.
   - Fallback: Accept any device name without validation if enumeration fails.

### Dependencies

- Phase 1 (voice settings UI must exist).
- Platform audio device enumeration (may require new crate dependency).

## Risk Mitigation

- **Audio device enumeration complexity**: Use best-effort validation; fall back to accepting any device name if platform API is unavailable.
- **Hotkey conflict detection**: Compare against the keybinding registry; warn but allow override.
