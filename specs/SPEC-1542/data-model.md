### Code model

| Type | Responsibility |
|---|---|
| `Settings` | runtime application settings with env overrides applied |
| `ConfigToml` | serde DTO for canonical `~/.gwt/config.toml` |
| `ProfilesConfig` | extracted runtime model for the `[profiles]` section |
| `ProfilesSectionToml` | serde DTO for the `[profiles]` section inside `ConfigToml` |
| `Profile` | a single profile entry under `profiles.<name>` |

### `~/.gwt/config.toml`

| Section | Purpose |
|---|---|
| `profiles` | active profile と profile definitions |
| `profiles.<name>.ai` | endpoint / api_key / model / language / summary_enabled |
| `agent_config` | app-wide agent provider settings |
| `tools` | custom coding agent definitions |
| `recent_projects` | recent project list |
| `agent`, `docker`, `appearance`, `voice_input`, `terminal` | app-wide operational settings |

### `<project>/.gwt/project.toml`

| Key | Required | Purpose |
|---|---|---|
| `bare_repo_name` | Yes | bare repository resolution |
| `remote_url` | No | reference metadata |
| `location` | No | worktree location hint |
| `created_at` | No | creation timestamp |
