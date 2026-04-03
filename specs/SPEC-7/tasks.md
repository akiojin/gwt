# SPEC-7: Settings and Profiles -- Tasks

## Phase 1: Voice Settings UI

### 1.1 VoiceConfig Data Model (gwt-core)

- [P] [x] **T-001**: Write test for `VoiceConfig` default values (hotkey="Ctrl+G,v", input_device="system_default", language="auto", enabled=false).
- [P] [x] **T-002**: Write test for `VoiceConfig` serialization to TOML `[voice]` section.
- [P] [ ] **T-003**: Write test for `VoiceConfig` deserialization from TOML, including missing `[voice]` section (defaults applied).
- [ ] **T-004**: Implement `VoiceConfig` struct with serde derive in `crates/gwt-core/src/config.rs`.
- [ ] **T-005**: Add `voice: VoiceConfig` field to the root `Config` struct with `#[serde(default)]`.
- [x] **T-006**: Verify T-001, T-002, T-003 pass (GREEN).

### 1.2 Voice Settings Category (gwt-tui)

- [ ] **T-007**: Write test for `SettingsCategory::Voice` variant existence and sidebar ordering (7th position).
- [x] **T-008**: Write test for Voice settings form rendering with 5 fields.
- [x] **T-009**: Add `Voice` variant to `SettingsCategory` enum in `crates/gwt-tui/src/screens/settings.rs`.
- [x] **T-010**: Implement Voice category rendering: model_path (text input), hotkey (text input), input_device (text input), language (text input), enabled (toggle).
- [x] **T-011**: Wire Voice settings load/save through existing settings persistence.
- [x] **T-012**: Verify T-007, T-008 pass (GREEN).

## Phase 2: Voice Settings Validation

### 2.1 Validation Logic (gwt-core)

- [P] [x] **T-013**: Write test for `VoiceConfig::validate()` rejecting non-existent model_path.
- [P] [x] **T-014**: Write test for `VoiceConfig::validate()` rejecting model_path that is a file (not directory).
- [P] [x] **T-015**: Write test for `VoiceConfig::validate()` accepting valid model_path directory.
- [P] [x] **T-016**: Write test for `VoiceConfig::validate()` accepting disabled config without model_path.
- [x] **T-017**: Implement `VoiceConfig::validate()` method.
- [x] **T-018**: Verify T-013 through T-016 pass (GREEN).

### 2.2 Validation UI (gwt-tui)

- [x] **T-019**: Write test for inline validation error display on invalid model_path.
- [x] **T-020**: Implement inline validation error rendering in Voice settings form.
- [x] **T-021**: Block save when validation fails; show error summary.
- [x] **T-022**: Verify T-019 passes (GREEN).

### 2.3 Integration Verification

- [ ] **T-023**: Manual verification: open Settings, navigate to Voice, configure all fields, save, reopen, verify persistence.
- [ ] **T-024**: Manual verification: set invalid model_path, attempt save, verify error displayed.
