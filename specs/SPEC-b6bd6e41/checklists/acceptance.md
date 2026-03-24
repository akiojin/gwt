# Acceptance Checklist

- [x] Enter/Send during thinking interrupts the active run
- [x] Tab during thinking queues the message instead of interrupting
- [x] Queued messages drain in FIFO order
- [x] Startup analysis is interrupted by a real user send
- [x] Stale run output does not overwrite the latest Assistant state
- [x] Targeted Rust tests pass
- [x] AssistantPanel tests pass
- [x] `svelte-check` passes
