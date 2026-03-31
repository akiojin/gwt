### Technical Context
- Target backend: `crates/gwt-tauri/src/commands/assistant.rs`
- Target engine: `crates/gwt-tauri/src/assistant_engine.rs`
- Target verification: Rust unit tests for startup analysis prompt handling and assistant state serialization

### Implementation Approach
- Add an engine entrypoint that queues a hidden system prompt for startup analysis and then runs the existing LLM loop.
- Ensure startup analysis begins automatically on session start and yields the first visible guidance message without requiring a manual user prompt.
- Keep the startup prompt hidden by reusing the existing transcript filter that omits system/tool messages.

### Phasing
1. Add RED tests for startup-analysis prompt insertion and transcript filtering.
2. Implement startup analysis execution.
3. Wire it into session startup and run targeted verification.
