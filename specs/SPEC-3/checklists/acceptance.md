# Acceptance Checklist: SPEC-3 - Agent Management

Mark each item only after direct verification evidence exists.
- [ ] Confirm agent detection results are surfaced consistently in the wizard and conversion picker.
- [ ] Confirm cached versions are shown immediately and stale entries mark refresh state.
- [ ] Confirm session conversion preserves the working directory when switching agent types.
- [ ] Confirm failed conversion leaves the original session intact and reports through notifications.
- [ ] Confirm the final implementation replaces the running PTY, not just session metadata, before closure.
