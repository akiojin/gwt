### Code model

| Type | Responsibility |
|---|---|
| `Settings` | runtime application settings with env overrides applied |
| `ConfigToml` | serde DTO for canonical `~/.gwt/config.toml` |
| `ProfilesConfig` | extracted runtime model for the `[profiles]` section |
| `ProfilesSectionToml` | serde DTO for the `[profiles]` section inside `ConfigToml` |
| `Profile` | a single profile entry under `profiles.<name>` |

### AI settings location

| Location | Meaning |
|---|---|
| `profiles.version` | schema version |
| `profiles.active` | active profile name |
| `profiles.<name>.ai.endpoint` | OpenAI-compatible endpoint |
| `profiles.<name>.ai.api_key` | API key |
| `profiles.<name>.ai.model` | selected model |
| `profiles.<name>.ai.language` | summary / response language |
| `profiles.<name>.ai.summary_enabled` | summary feature flag |

### Unsupported legacy fields

| Field / file | Status |
|---|---|
| `default_ai` | unsupported |
| `profiles.profiles.<name>` | unsupported |
| `profiles.toml` / `profiles.yaml` | unsupported |
