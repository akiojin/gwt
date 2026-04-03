# Data Model: SPEC-3 - Agent Management

## Primary Objects
- **Detected agents** - `DetectedAgent` carries `AgentId`, optional version, and binary path from detection.
- **Version cache** - `VersionCache` stores recent versions and fetched-at timestamps per agent key.
- **Wizard options** - `AgentOption` carries agent id, display name, cached versions, and cache staleness state.
- **Session conversion** - `PendingSessionConversion` links the active session index to the selected target agent.
