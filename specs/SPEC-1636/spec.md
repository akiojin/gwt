> **📜 HISTORICAL (SPEC-1776)**: This SPEC was written for the previous GUI stack (Tauri/Svelte/C#). It is retained as a historical reference. The gwt-tui migration (SPEC-1776) supersedes GUI-specific design decisions described here.

# Assistant Send Interrupt and Tab Queue

## Background

Assistant composer was blocked while inference was running, which made the transcript feel stuck even when the user already knew the next instruction. This change lets the user interrupt the active run with an immediate send and queue Tab-based sends for automatic follow-up delivery.

## User Scenarios

### S1: Enter/Send interrupts current inference
- Priority: P0
- Given: Assistant is already thinking
- When: The user presses Enter or clicks Send
- Then: The current run is cancelled/stale-invalidated and the new message is prioritized immediately

### S2: Tab queues a reply instead of interrupting
- Priority: P0
- Given: Assistant is already thinking
- When: The user presses Tab in the Assistant composer
- Then: The message is added to a FIFO queue and is sent automatically after the current run finishes

### S3: Startup analysis does not block a real user send
- Priority: P0
- Given: Assistant startup analysis is running
- When: The user sends a message
- Then: Startup analysis is interrupted and the user message becomes the active run

## Functional Requirements

- FR-001: Enter and Send button must use interrupt delivery and prioritize the new user message immediately
- FR-002: Tab key in the Assistant composer must use queued delivery and preserve FIFO order
- FR-003: Queued messages must be sent automatically when the active Assistant run completes
- FR-004: Assistant must ignore stale/cancelled run results so cancelled output does not overwrite the latest state
- FR-005: Startup analysis must be interruptible by a real user send
- FR-006: Assistant state response must expose queued message count to the frontend
- FR-007: The Assistant composer must remain enabled while inference is active unless AI is unavailable or startup has failed

## Non-Functional Requirements

- NFR-001: Interrupt send must return control to the UI immediately without waiting for the previous run to finish
- NFR-002: Queue processing must remain window-local and must not reorder queued messages
- NFR-003: Stale run suppression must prevent tool results or assistant replies from cancelled runs from being committed

## Success Criteria

- SC-001: Enter/Send during thinking starts a new active run instead of being blocked
- SC-002: Tab during thinking increments a visible queue and drains in FIFO order
- SC-003: Startup analysis can be interrupted by a user message without stale startup output taking over the transcript
- SC-004: Rust and frontend tests cover cancel/queue behavior and pass
