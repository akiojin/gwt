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

## Phase 2: File Paste

**Goal**: Implement clipboard file path extraction and PTY injection via Ctrl+G,p hotkey.

### Key Changes

1. **gwt-tui**: Register Ctrl+G,p chord in the keybinding system.

2. **gwt-core**: Implement `ClipboardFilePaste` module.
   - macOS: Read `NSPasteboard` for `public.file-url` pasteboard type.
   - Linux: Read from xclip (`xclip -selection clipboard -t text/uri-list -o`) or wl-paste (`wl-paste --type text/uri-list`).
   - Parse `file://` URIs to absolute paths.
   - Fallback: If no file URIs found, paste clipboard text content as-is.

3. **gwt-tui**: Inject paths into active PTY.
   - One path per line, shell-escaped with quotes if paths contain spaces.
   - No trailing newline after the last path.

### Dependencies

- Platform clipboard access (no new crate needed; use `std::process::Command` for xclip/wl-paste, `objc` crate for NSPasteboard).

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
- **Platform clipboard differences**: Use conditional compilation (`#[cfg(target_os)]`) with fallback to text-only paste.
- **AI branch naming latency**: When the AI suggestion step is enabled, the 10-second timeout with manual fallback ensures the wizard never blocks indefinitely.
