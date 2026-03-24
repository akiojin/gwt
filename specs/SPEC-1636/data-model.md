# Data Model

## Backend

- `AssistantDeliveryMode`
  - `interrupt`
  - `queue`
- `AssistantRuntimeState`
  - `active_generation?: u64`
  - `active_kind?: startup | user`
  - `queued_inputs: VecDeque<String>`

## Frontend

- `AssistantState.queuedMessageCount: number`
- `AssistantState.currentStatus` supports `awaiting_user_choice`
