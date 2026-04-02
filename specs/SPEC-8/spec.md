# Input Extensions -- Voice Input, File Paste, AI Branch Naming

## Background

gwt-tui extends terminal input with voice transcription (Qwen3-ASR), file paste from clipboard, and AI-assisted branch naming. The voice backend trait exists in gwt-core but TUI integration is minimal. The AI branch naming backend (`BranchNameSuggester`) exists in gwt-core. File paste from system clipboard is entirely new functionality.

## User Stories

### US-1: Dictate Commands via Voice Input (P1) -- NOT IMPLEMENTED

As a developer, I want to dictate commands using voice input so that I can interact with the terminal hands-free.

**Acceptance Scenarios**

1. Given voice input is enabled, when I press Ctrl+G,v, then audio recording begins and a recording indicator appears in the status bar.
2. Given recording is active, when I speak a command, then the Qwen3-ASR model transcribes my speech to text.
3. Given transcription completes, when the text is ready, then it is injected into the active PTY as keystrokes.
4. Given recording has been active for 30 seconds, when the timeout is reached, then recording stops automatically.
5. Given 3 seconds of silence during recording, when silence is detected, then recording stops and transcription begins.
6. Given voice input is disabled in settings, when I press Ctrl+G,v, then nothing happens.

### US-2: Paste File Paths from Clipboard (P1) -- NOT IMPLEMENTED

As a developer, I want to paste file paths from the system clipboard into the terminal so that I can quickly reference files without typing paths manually.

**Acceptance Scenarios**

1. Given one or more files are copied to the system clipboard, when I press Ctrl+G,p, then the absolute file paths are pasted into the active PTY.
2. Given multiple files are in the clipboard, when pasted, then each path appears on a separate line.
3. Given the clipboard contains text (not file references), when I press Ctrl+G,p, then the text is pasted as-is.
4. Given the clipboard is empty, when I press Ctrl+G,p, then nothing is pasted and no error is shown.

### US-3: Get AI-Suggested Branch Names in Wizard (P2) -- PARTIALLY IMPLEMENTED

As a developer, I want AI-suggested branch names when creating a new worktree so that I can quickly pick a well-formatted name.

**Acceptance Scenarios**

1. Given I am in the wizard at the AgentSelect step, when the SPEC title or Issue description is available, then 3-5 AI-generated branch name suggestions are displayed.
2. Given suggestions are displayed, when I select one, then it is used as the branch name.
3. Given I prefer a custom name, when I select "Manual input", then I can type a branch name freely.
4. Given the AI provider is unavailable or times out (10s), when suggestions fail, then manual input is shown as the fallback.
5. Given a suggestion is selected, when validated, then it conforms to Git branch naming rules (no spaces, no special chars except /-_).

## Edge Cases

- Microphone permission denied by the OS.
- Qwen3-ASR model file missing or corrupted at runtime.
- Audio device disconnected during recording.
- Clipboard contains binary data (images, non-text).
- File paths in clipboard contain spaces or special characters.
- AI branch name suggestion returns names that exceed Git's 255-byte limit.
- Network timeout during AI branch name generation.
- Multiple rapid Ctrl+G,v presses (debounce needed).
- PTY is not focused when hotkey is pressed.

## Functional Requirements

### Voice Input

- **FR-001**: Voice input activation via Ctrl+G,v hotkey (chord: Ctrl+G followed by v).
- **FR-002**: Qwen3-ASR as speech recognition backend, loaded from the model path configured in settings.
- **FR-003**: Audio capture from the configured input device (macOS: AVFoundation, Linux: PulseAudio/ALSA).
- **FR-004**: Transcribed text injected into the active PTY as keystrokes (one character at a time to respect PTY echo).
- **FR-005**: Status bar shows a recording indicator (microphone icon + elapsed time) during capture.
- **FR-006**: Voice timeout: 30 seconds maximum recording duration; auto-stop on 3 seconds of silence.

### File Paste

- **FR-007**: File paste activation via Ctrl+G,p hotkey (chord: Ctrl+G followed by p).
- **FR-008**: Extract file paths from system clipboard (macOS: NSPasteboard `public.file-url` type, Linux: xclip/wl-paste).
- **FR-009**: Paths injected as newline-separated absolute path strings into the active PTY.
- **FR-010**: Multi-file paste supported; one path per line, paths shell-escaped if they contain spaces.

### AI Branch Naming

- **FR-011**: AI branch name suggestion displayed in wizard AgentSelect step when creating a new worktree.
- **FR-012**: `BranchNameSuggester` generates 3-5 candidate names from the SPEC title or Issue description.
- **FR-013**: Fallback to manual text input if AI is unavailable or timeout exceeds 10 seconds.
- **FR-014**: All generated branch names validated against Git branch naming rules before display.

## Non-Functional Requirements

- **NFR-001**: Voice transcription completes within 5 seconds for 10-second audio input.
- **NFR-002**: File paste operation completes within 100ms from hotkey press to PTY injection.
- **NFR-003**: AI branch name suggestion completes within 10 seconds; timeout triggers fallback.
- **NFR-004**: Voice recording introduces no audible latency or glitches.
- **NFR-005**: All hotkeys use the Ctrl+G chord prefix to avoid conflicts with terminal applications.

## Success Criteria

- **SC-001**: Voice input records audio, transcribes via Qwen3-ASR, and injects text into PTY end-to-end.
- **SC-002**: Status bar recording indicator appears during voice capture and disappears on completion.
- **SC-003**: File paste correctly extracts and injects file paths from the system clipboard.
- **SC-004**: Multi-file paste produces one path per line with correct shell escaping.
- **SC-005**: AI branch naming displays 3-5 suggestions in the wizard and allows selection or manual override.
- **SC-006**: All generated branch names pass Git naming validation.
- **SC-007**: Timeout and fallback paths work correctly for both voice and AI branch naming.
