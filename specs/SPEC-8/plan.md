# SPEC-8: Input Extensions -- Implementation Plan

## Phase 1: Voice TUI Integration

**Goal**: Wire the existing voice backend trait in gwt-core to the TUI layer with hotkey activation, status bar indicator, and PTY text injection.

### Key Changes

1. **gwt-tui**: Register Ctrl+G,v chord in the keybinding system.
   - On activation, start audio capture using the configured input device.
   - Delegate to `VoiceRecorder` trait in gwt-core.

2. **gwt-core**: Implement `VoiceRecorder` trait with Qwen3-ASR backend.
   - Load model from the path specified in `VoiceConfig` (SPEC-7).
   - Capture audio via platform-specific API (AVFoundation on macOS, PulseAudio/ALSA on Linux).
   - Silence detection: 3-second threshold using RMS amplitude.
   - Maximum recording duration: 30 seconds.

3. **gwt-tui**: Status bar recording indicator.
   - Show microphone Unicode symbol and elapsed time during recording.
   - Clear indicator when recording stops.

4. **gwt-tui**: PTY text injection.
   - Transcribed text written to the active PTY as synthetic keystrokes.
   - Respect PTY echo mode.

### Dependencies

- SPEC-7 Phase 1 (VoiceConfig must exist for model path and device configuration).
- Qwen3-ASR model binary (user-provided, not bundled).

## Phase 2: Terminal Paste

**Goal**: Make normal terminal paste behave like a real paste operation in the
active PTY, including bracketed-paste passthrough when requested by the PTY
application.

### Key Changes

1. **gwt-tui**: Enable bracketed paste support in the outer terminal runtime.
   - Turn on crossterm bracketed-paste handling during terminal setup.
   - Turn it off during shutdown and panic recovery.

2. **gwt-tui**: Route `Event::Paste(String)` through a dedicated paste path.
   - Keep the pasted payload intact instead of translating it into per-key input.
   - Ignore only truly empty payloads; preserve whitespace-only paste content.
   - When focus is on a non-terminal text input, route the pasted text into that
     field instead of dropping it.

3. **gwt-tui**: Inject pasted text into the active PTY.
   - Detect whether the active PTY screen has requested bracketed paste mode.
   - Wrap payloads with `ESC[200~ ... ESC[201~` only when that mode is enabled.
   - Remove the deprecated `Ctrl+G,p` file-paste keybinding from the UI surface.

### Dependencies

- crossterm bracketed-paste events and vt100 screen-mode tracking.

## Phase 3: AI Branch Naming Wizard Integration

**Goal**: Keep `BranchNameSuggester` integrated behind the wizard's dedicated
AI suggestion step while the standard Launch Agent flow from Branches, SPEC
detail, and Issue detail continues directly to manual branch input.

### Key Changes

1. **gwt-tui**: In the dedicated AI suggestion step, after SPEC/Issue context is available, call `BranchNameSuggester::suggest()`.
   - Display 3-5 suggestions as a selectable list.
   - Add "Manual input" option at the bottom.
   - Show a loading spinner while waiting for suggestions.

2. **gwt-core**: Ensure `BranchNameSuggester::suggest()` validates all generated names.
   - Apply `git check-ref-format` rules.
   - Strip or replace invalid characters.
   - Enforce 255-byte maximum length.

3. **gwt-tui**: Timeout handling for the dormant AI suggestion step.
   - 10-second timeout on suggestion generation.
   - On timeout, auto-select "Manual input" and show a brief notification.

### Dependencies

- Existing `BranchNameSuggester` in gwt-core.
- AI provider configuration (SPEC-7 AISettings).

## Risk Mitigation

- **Qwen3-ASR integration complexity**: Start with a mock recorder for TUI development; swap in real backend once model loading is verified.
- **Bracketed paste compatibility**: Respect the active PTY screen's input mode so shells that enable bracketed paste get wrapped payloads without showing escape sequences to programs that do not.
- **AI branch naming latency**: When the AI suggestion step is enabled, the 10-second timeout with manual fallback ensures the wizard never blocks indefinitely.
