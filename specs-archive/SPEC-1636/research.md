# Research

- `assistant_send_message` previously removed the engine from shared state and blocked until the full LLM run completed, which made concurrent sends impossible
- Assistant startup already used session generation to invalidate stale startup tasks, so the same concept can be reused for stale user-run suppression
- `AssistantPanel.svelte` blocked all sends while `isThinking` was true, and Tab had no send-specific behavior
- Transcript choice UI remains a separate follow-up and is not part of this implementation batch
