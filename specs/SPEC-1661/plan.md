### Technical Context
- Target UI: `gwt-gui/src/lib/components/AssistantPanel.svelte`
- Target test: `gwt-gui/src/lib/components/AssistantPanel.test.ts`

### Implementation Approach
- Reproduce the Windows/WebView2 timing gap in component tests by ending composition before Enter and asserting that IME-specific keydown metadata does not trigger send.
- Update Enter handling to prioritize `KeyboardEvent.isComposing`, keep the local `isComposing` state as a backup guard, and add a `keyCode === 229` fallback.
- Preserve plain Enter send and Shift+Enter newline semantics outside IME composition.

### Phasing
1. Add focused AssistantPanel tests for IME confirm Enter, fallback `229`, plain Enter, and Shift+Enter.
2. Update AssistantPanel key handling.
3. Run targeted frontend verification and record the results.
