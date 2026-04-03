# Data Model: SPEC-8 - Input Extensions

## Primary Entities
### VoiceCaptureSession
- Role: Transient state for microphone capture and transcription handoff.
- Invariant: Cancellation and completion must leave the active PTY in a known state.

### PastePayload
- Role: Clipboard-derived text or file references prepared for terminal injection.
- Invariant: Injection must target the active PTY and preserve user intent.

### BranchSuggestionRequest
- Role: Prompt context used to derive a branch name suggestion.
- Invariant: Suggestion failures must fall back to explicit user input.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
