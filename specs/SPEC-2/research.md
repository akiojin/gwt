# Research: SPEC-2 - Workspace Shell

## Context
- The workspace shell uses the Elm Architecture split across `model.rs`, `message.rs`, and `app.rs`.
- The management area now has 8 tabs: Branches, Issues, PRs, Profiles, Git View, Versions, Settings, and Logs.
- Branch detail replaced the old Specs tab, so branch actions and detail sections carry more workflow context now.
- Help overlay auto-collection and full session persistence restoration still need closure before the SPEC can be treated as done.
