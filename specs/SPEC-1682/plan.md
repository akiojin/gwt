### Technical Context
- Startup orchestration lives in `crates/gwt-tauri/src/commands/assistant.rs`
- Startup LLM execution lives in `crates/gwt-tauri/src/assistant_engine.rs`
- Assistant tool exposure is defined in `crates/gwt-tauri/src/assistant_tools.rs`
- Pending startup UI is consumed by `gwt-gui/src/lib/components/AssistantPanel.svelte`

### Implementation Approach
- Add a project-local startup-analysis cache under `.gwt/assistant/` keyed by current branch, HEAD revision, and working-tree status signature.
- Emit concrete startup status messages (`Inspecting repository state`, `Checking startup analysis cache`, `Using cached startup analysis`, `Running startup analysis`) through the existing assistant state response path.
- Restrict startup analysis to a read-only tool set so startup never executes shell commands, pane input, or spec writes.
- Render assistant startup summaries through the existing MarkdownRenderer path.

### Phasing
1. Add RED tests for startup status messaging, cache round-trip, and read-only startup tool filtering.
2. Implement startup fingerprint / cache helpers and startup status state.
3. Route startup analysis through cache hit/miss logic with read-only tools only.
4. Render assistant startup summaries through the existing MarkdownRenderer path.
5. Run targeted Rust/frontend verification and reflect the result in the spec.
