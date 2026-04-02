# Data Model: SPEC-1776 — Parent UX Model

## Core Runtime Surfaces

| Surface | Responsibility |
|---|---|
| `BranchDashboardState` | primary entry としての branch list、session count、selection、quick actions |
| `SessionWorkspaceState` | session index、equal grid / maximize layout、focus、tab switch |
| `ManagementWorkspaceState` | `Branches / SPECs / Issues / Profiles / Settings / Versions / Logs` の active tab と tab-local state |
| `LaunchFlowState` | branch enter selector、Quick Start、full Wizard、hooks confirm |
| `EnvProfilesState` | env profile の一覧、編集、切替、OS env 参照・置換 |
| `SettingsTabState` | env を除く global settings categories の表示・選択 |

## Branch Dashboard

| Entity | Fields |
|---|---|
| `BranchRowInfo` | `branch_name`, `session_count`, `pr_status`, `divergence`, `quick_start_available` |
| `BranchEnterAction` | `OpenSingleSession`, `ShowSessionSelector`, `OpenWizard` |
| `BranchSessionSelector` | `branch_name`, `session_ids`, `actions = [existing, add, full_wizard]` |

## Session Workspace

| Entity | Fields |
|---|---|
| `SessionRecord` | `session_id`, `pane_id`, `branch_name`, `tool_id`, `status`, `last_active_at` |
| `SessionLayoutMode` | `EqualGrid` or `Maximized` |
| `SessionWorkspaceState` | `records`, `focused_session_id`, `layout_mode`, `management_open`, `last_non_management_layout` |
| `SessionGridState` | visible session ordering for `4+` sessions |
| `SessionTabsState` | maximized 時の tab order と active tab |

## Management Workspace

| Entity | Fields |
|---|---|
| `ManagementTab` | `Branches`, `SPECs`, `Issues`, `Profiles`, `Settings`, `Versions`, `Logs` |
| `ManagementWorkspaceState` | `active_tab`, `last_tab`, `tab_states` |
| `SettingsCategory` | `General`, `Worktree`, `Agent`, `CustomAgents`, `Environment`, `AISettings` |
| `SettingsManagementCategory` | visible subset = `General`, `Worktree`, `Agent`, `CustomAgents`, `AISettings` |

## Env Profiles

| Entity | Fields |
|---|---|
| `EnvProfileSummary` | `name`, `is_active`, `entry_count` |
| `EnvEntry` | `key`, `value`, `source_kind = literal or os_reference` |
| `EnvProfilesState` | `profiles`, `selected_profile`, `editor_mode`, `os_env_catalog` |

## Persistence Boundaries

| Concern | Canonical Owner |
|---|---|
| session persistence | `gwt-core` session store / watcher |
| env profile persistence | `ProfilesConfig` and related config persistence |
| SPEC artifact storage | local SPEC storage (`SPEC-1579`) |
| Issue cache / linkage | issue exact cache and linkage (`SPEC-1714`) |

## Explicit Non-Goals in This Parent Model

- `AI summary` data model
- tmux pane data structures
