# Input Extensions -- Voice Input, Terminal Paste, AI Branch Naming

## Background

gwt-tui extends terminal input with voice transcription (Qwen3-ASR), normal
terminal paste, and AI-assisted branch naming. The voice path now
routes start/stop/transcribe through a shared TUI runtime seam, but the
concrete Qwen3-ASR backend remains a stub that returns model-loading errors.
The AI branch naming flow remains implemented in the codebase, including
explicit manual-entry fallback in the suggestion list and normalization to
`3..=5` git-safe names, but the standard Launch Agent wizard currently skips
that step and opens manual branch input directly. Terminal paste now uses
`Event::Paste` and forwards payloads to the active PTY as a single paste
operation, wrapping with bracketed-paste control sequences only when the PTY
application has requested that mode.

## User Stories

### US-1: Dictate Commands via Voice Input (P1) -- PARTIALLY IMPLEMENTED

As a developer, I want to dictate commands using voice input so that I can interact with the terminal hands-free.

**Acceptance Scenarios**

1. Given voice input is enabled, when I press Ctrl+G,v, then audio recording begins and a recording indicator appears in the status bar.
2. Given recording is active, when I speak a command, then the Qwen3-ASR model transcribes my speech to text.
3. Given transcription completes, when the text is ready, then it is injected into the active PTY as keystrokes.
4. Given recording has been active for 30 seconds, when the timeout is reached, then recording stops automatically.
5. Given 3 seconds of silence during recording, when silence is detected, then recording stops and transcription begins.
6. Given voice input is disabled in settings, when I press Ctrl+G,v, then nothing happens.

### US-2: Paste Multi-line Text Like a Terminal (P1) -- IMPLEMENTED

As a developer, I want pasted terminal text to behave like a normal terminal
paste so that multi-line commands are forwarded as one paste operation instead
of being replayed as line-by-line Enter input.

**Acceptance Scenarios**

1. Given text is pasted from the host terminal, when gwt receives the paste,
   then the payload is forwarded to the active PTY as one paste operation.
2. Given the active PTY application has enabled bracketed paste mode, when the
   payload contains newlines, then gwt wraps the payload with
   `ESC[200~ ... ESC[201~` before PTY injection.
3. Given the active PTY application has not enabled bracketed paste mode, when
   text is pasted, then gwt forwards the raw payload without converting it to
   per-key `Enter` events.
4. Given the pasted text is whitespace only, when gwt receives the paste event
   for the active PTY, then the payload is still forwarded unchanged.
5. Given the pasted text is empty, when gwt receives the paste event, then
   nothing is injected and no error is shown.
6. Given the focused surface is a non-terminal text input (for example the
   initialization URL field, wizard text entry, branch/issue search, or
   settings edit field), when text is pasted, then gwt inserts the payload into
   that field instead of dropping it.

### US-3: Get AI-Suggested Branch Names in Wizard (P2) -- PARTIALLY IMPLEMENTED

As a developer, I want AI-suggested branch names when creating a new worktree so that I can quickly pick a well-formatted name.

**Acceptance Scenarios**

1. Given I create a new worktree through the standard Launch Agent flow from
   Branches, SPEC detail, or Issue detail, when I continue past branch type
   and issue selection, then the wizard opens manual branch input directly
   without requiring AI settings.
2. Given the AI suggestion step is explicitly re-enabled, when the SPEC title or Issue description is available, then 3-5 AI-generated branch name suggestions are displayed.
3. Given suggestions are displayed, when I select one, then it is used as the branch name.
4. Given I prefer a custom name or the AI provider is unavailable or times out (10s), when I choose manual input, then I can type a branch name freely.
5. Given a suggestion is selected, when validated, then it conforms to Git branch naming rules (no spaces, no special chars except /-_).
6. Given I use the current product surface, when I configure Launch Agent, then no public control is available to re-enable the dormant AI suggestion step.

## Edge Cases

- Microphone permission denied by the OS.
- Qwen3-ASR model file missing or corrupted at runtime.
- Audio device disconnected during recording.
- Clipboard contains binary data (images, non-text).
- Active PTY application has not enabled bracketed paste mode.
- Pasted payload contains multiple newlines or shell metacharacters.
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

### Terminal Paste

- **FR-007**: gwt enables outer-terminal bracketed paste mode while the TUI is running and disables it on shutdown.
- **FR-008**: gwt consumes `crossterm::event::Event::Paste(String)` and preserves the pasted payload as a single text block.
- **FR-009**: When the active PTY application has requested bracketed paste mode, gwt wraps the payload with `ESC[200~` and `ESC[201~` before PTY injection.
- **FR-010**: When the focused surface is a non-terminal text input, gwt routes pasted text into that input instead of discarding it.
- **FR-011**: `Ctrl+G,p` file-paste hotkey is not part of the product surface; normal paste is the canonical paste path.

### AI Branch Naming

- **FR-012**: The standard new-worktree Launch Agent flow from Branches,
  SPEC detail, and Issue detail skips AI branch suggestion and opens manual
  branch input without requiring active AI settings.
- **FR-013**: When the AI suggestion step is explicitly enabled, `BranchNameSuggester` generates 3-5 candidate names from the SPEC title or Issue description.
- **FR-014**: When the AI suggestion step is enabled, manual text input remains available if AI is unavailable or timeout exceeds 10 seconds.
- **FR-015**: All generated branch names are validated against Git branch naming rules before display.
- **FR-016**: The dormant AI suggestion step is implementation-only in this
  slice; no public UI or settings affordance re-enables it.

## Non-Functional Requirements

- **NFR-001**: Voice transcription completes within 5 seconds for 10-second audio input.
- **NFR-002**: Terminal paste injection completes within 100ms from paste event receipt to PTY injection.
- **NFR-003**: When enabled, AI branch name suggestion completes within 10 seconds; timeout triggers fallback.
- **NFR-004**: Voice recording introduces no audible latency or glitches.
- **NFR-005**: All hotkeys use the Ctrl+G chord prefix to avoid conflicts with terminal applications.

## Success Criteria

- **SC-001**: Voice input records audio, transcribes via Qwen3-ASR, and injects text into PTY end-to-end.
- **SC-002**: Status bar recording indicator appears during voice capture and disappears on completion.
- **SC-003**: Normal terminal paste forwards the pasted payload from the host terminal into the active PTY end-to-end.
- **SC-004**: When the active PTY application requests bracketed paste mode, multi-line paste reaches the PTY as one bracketed payload instead of line-by-line `Enter` input.
- **SC-005**: Standard Launch Agent new-branch flow reaches manual branch entry without AI configuration, and the dormant AI suggestion path still supports selection or manual override when explicitly enabled.
- **SC-006**: All generated branch names pass Git naming validation.
- **SC-007**: Timeout and fallback paths work correctly for both voice and AI branch naming.
