### Technical Context
- Target UI: `gwt-gui/src/lib/components/AssistantPanel.svelte`
- Target tests: `gwt-gui/src/lib/components/AssistantPanel.test.ts`

### Implementation Approach
- Preserve line breaks at the transcript layer by applying multiline-safe CSS to shared message content rendering.
- Show a local optimistic user message immediately after send, keep a temporary thinking indicator visible while the request is in flight, then replace the optimistic state with the backend response.
- On send failure, roll back the optimistic transcript entry and restore the original input text.
- Maintain a session-local history buffer for successfully sent user messages and navigate it only when the textarea caret is at the top/bottom boundary.

### Phasing
1. Extend AssistantPanel component tests for newline rendering, optimistic transcript updates, and failure rollback.
2. Update AssistantPanel transcript rendering and send-state handling.
3. Add composer history navigation state and keyboard handling.
4. Run targeted frontend verification and reflect the results in the spec.
