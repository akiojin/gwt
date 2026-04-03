# SPEC-8: Input Extensions -- Tasks

## Phase 1: Voice TUI Integration

### 1.1 Voice Hotkey Registration

- [x] **T-001**: Write test for Ctrl+G,v chord registration in keybinding system.
- [x] **T-002**: Write test for hotkey no-op when voice is disabled in config.
- [x] **T-003**: Implement Ctrl+G,v chord handler in `crates/gwt-tui/src/app.rs`.
- [x] **T-004**: Verify T-001, T-002 pass (GREEN).

### 1.2 Voice Recorder Backend (gwt-core)

- [P] [ ] **T-005**: Write test for `VoiceRecorder` trait interface (start, stop, transcribe).
- [P] [ ] **T-006**: Write test for mock recorder returning hardcoded transcription.
- [P] [ ] **T-007**: Write test for recording timeout at 30 seconds.
- [P] [ ] **T-008**: Write test for silence detection stopping recording after 3 seconds.
- [ ] **T-009**: Define `VoiceRecorder` trait in `crates/gwt-core/src/voice.rs`.
- [ ] **T-010**: Implement `MockVoiceRecorder` for testing.
- [ ] **T-011**: Implement `Qwen3AsrRecorder` with model loading and audio capture.
- [ ] **T-012**: Verify T-005 through T-008 pass (GREEN).

### 1.3 Status Bar Recording Indicator

- [x] **T-013**: Write test for status bar showing microphone icon during recording.
- [x] **T-014**: Write test for status bar clearing indicator when recording stops.
- [x] **T-015**: Implement recording indicator in status bar widget.
- [x] **T-016**: Verify T-013, T-014 pass (GREEN).

### 1.4 PTY Text Injection

- [x] **T-017**: Write test for transcribed text injection into PTY.
- [x] **T-018**: Write test for empty transcription producing no PTY input.
- [x] **T-019**: Implement PTY text injection from voice transcription result.
- [x] **T-020**: Verify T-017, T-018 pass (GREEN).

## Phase 2: File Paste

### 2.1 Clipboard File Path Extraction (gwt-core)

- [P] [x] **T-021**: Write test for extracting single file path from clipboard.
- [P] [x] **T-022**: Write test for extracting multiple file paths from clipboard.
- [P] [x] **T-023**: Write test for clipboard with text content (no file URIs) returning text as-is.
- [P] [x] **T-024**: Write test for empty clipboard returning None.
- [P] [x] **T-025**: Write test for file paths with spaces being shell-escaped.
- [x] **T-026**: Implement `ClipboardFilePaste` module in `crates/gwt-core/src/clipboard.rs`.
- [ ] **T-027**: Implement macOS NSPasteboard file URI extraction.
- [x] **T-028**: Implement Linux xclip/wl-paste file URI extraction.
- [x] **T-029**: Verify T-021 through T-025 pass (GREEN).

### 2.2 File Paste Hotkey and PTY Injection

- [x] **T-030**: Write test for Ctrl+G,p chord registration.
- [x] **T-031**: Write test for multi-path injection producing one path per line.
- [x] **T-032**: Implement Ctrl+G,p chord handler in `crates/gwt-tui/src/app.rs`.
- [x] **T-033**: Implement PTY injection of extracted file paths.
- [x] **T-034**: Verify T-030, T-031 pass (GREEN).

## Phase 3: AI Branch Naming Wizard Integration

### 3.1 Branch Name Suggestion Display

- [x] **T-035**: Write test for `BranchNameSuggester::suggest()` returning 3-5 valid names.
- [x] **T-036**: Write test for all suggestions passing `git check-ref-format` validation.
- [x] **T-037**: Write test for suggestion list rendering in wizard AgentSelect step.
- [x] **T-038**: Write test for "Manual input" option always present at bottom of list.
- [x] **T-039**: Implement suggestion list UI in wizard AgentSelect step.
- [x] **T-040**: Wire `BranchNameSuggester::suggest()` call with SPEC title / Issue description context.
- [x] **T-041**: Verify T-035 through T-038 pass (GREEN).

### 3.2 Timeout and Fallback

- [x] **T-042**: Write test for 10-second timeout triggering manual input fallback.
- [x] **T-043**: Write test for AI provider unavailable triggering fallback.
- [x] **T-044**: Implement timeout wrapper around suggestion generation.
- [x] **T-045**: Implement fallback to manual input with notification message.
- [x] **T-046**: Verify T-042, T-043 pass (GREEN).

### 3.3 Integration Verification

- [ ] **T-047**: Manual verification: voice input records, transcribes, and injects text into PTY.
- [ ] **T-048**: Manual verification: file paste extracts paths from clipboard and injects into PTY.
- [ ] **T-049**: Manual verification: wizard displays branch name suggestions and allows selection.
