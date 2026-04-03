# Data Model: SPEC-7 - Settings and Profiles

## Primary Entities
### SettingsFormState
- Role: Current editable settings state for the active category.
- Invariant: Form state must map cleanly back to persisted config.

### ProfileRecord
- Role: Named profile with environment and tool preferences.
- Invariant: Profile changes must be reversible and explicit.

### VoiceConfig
- Role: Voice-input configuration such as model path, device, language, and hotkey.
- Invariant: Validation must enforce both existence and expected kind for file-system paths.

## Lifecycle Notes
- `metadata.json`, `tasks.md`, and `progress.md` must stay aligned.
- Completion cannot be claimed from implementation alone; the checklists must agree.
