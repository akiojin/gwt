# Settings and Profiles -- Configuration UI, Environment Variables, Voice Settings

## Background

gwt-tui has a comprehensive Settings screen with 6 categories (General, Worktree, Agent, CustomAgents, Environment, AISettings) and profile-based environment management. All existing categories are fully implemented. This SPEC documents the complete settings domain and adds a new voice input settings category for configuring speech recognition parameters.

## User Stories

### US-1: Configure General Settings (P0) -- IMPLEMENTED

As a developer, I want to configure general application settings so that gwt behaves according to my preferences.

**Acceptance Scenarios**

1. Given I open the Settings screen, when I navigate to the General category, then I see configurable options for theme, default shell, and startup behavior.
2. Given I change a general setting, when I save and restart gwt, then the setting persists across sessions.
3. Given I enter an invalid value for a setting, when I attempt to save, then a validation error is displayed inline.

### US-2: Manage Environment Profiles (P0) -- IMPLEMENTED

As a developer, I want to manage environment profiles so that I can switch between different sets of environment variables for different projects.

**Acceptance Scenarios**

1. Given I open the Environment settings, when I create a new profile, then I can define key-value pairs for environment variables.
2. Given multiple profiles exist, when I select a profile, then the associated environment variables are applied to new agent sessions.
3. Given I edit an existing profile, when I save changes, then the updated variables take effect for subsequent sessions.
4. Given I delete a profile, when confirmed, then the profile and its variables are removed permanently.

### US-3: Configure AI Provider Settings (P0) -- IMPLEMENTED

As a developer, I want to configure AI provider settings so that gwt connects to my preferred AI service.

**Acceptance Scenarios**

1. Given I open the AISettings category, when I configure an API key and model, then agent sessions use the specified provider.
2. Given I change the AI provider, when I start a new agent session, then the new provider is used without requiring a restart.
3. Given an invalid API key is provided, when I test the connection, then a clear error message indicates the authentication failure.

### US-4: Configure Voice Input Settings (P1) -- PARTIALLY IMPLEMENTED

As a developer, I want to configure voice input settings so that I can customize speech recognition behavior including model path, activation hotkey, and input device.

**Acceptance Scenarios**

1. Given I open the Settings screen, when I navigate to the Voice category, then I see options for model_path, hotkey, input_device, language, and enabled toggle.
2. Given I set a Qwen3-ASR model path, when I save, then the path is validated to exist on disk before persisting.
3. Given I change the hotkey from the default Ctrl+G,v, when I save, then the new hotkey activates voice input.
4. Given I select a specific input device, when I start voice recording, then audio is captured from the selected device.
5. Given I disable voice input, when I press the voice hotkey, then nothing happens and no error is shown.
6. Given voice settings are saved, when I reopen Settings, then all voice configuration values are displayed correctly.

## Edge Cases

- Model path points to a file that exists but is not a valid Qwen3-ASR model.
- Selected input device is disconnected after configuration.
- Hotkey conflicts with an existing keybinding.
- Environment profile name contains special characters or is empty.
- config.toml is corrupted or has invalid TOML syntax.
- Voice settings section missing from config.toml (first-time setup).

## Functional Requirements

- **FR-001**: General settings category with theme, default shell, and startup behavior options.
- **FR-002**: Worktree settings category with default worktree root and naming conventions.
- **FR-003**: Agent settings category with default agent type, timeout, and retry configuration.
- **FR-004**: CustomAgents settings category with user-defined agent command templates.
- **FR-005**: Environment settings category with profile CRUD (create, read, update, delete) operations.
- **FR-006**: AISettings category with provider, model, API key, and endpoint configuration.
- **FR-007**: Voice settings category with the following fields:
  - `model_path`: Filesystem path to the Qwen3-ASR model directory.
  - `hotkey`: Activation key chord (default: `Ctrl+G,v`).
  - `input_device`: Audio input device (`system_default` or specific device name).
  - `language`: Recognition language (`auto` for auto-detect, or BCP-47 language tag).
  - `enabled`: Boolean toggle (default: `false`).
- **FR-008**: Voice settings persisted in `config.toml` under `[voice]` section.
- **FR-009**: Voice settings validation:
  - Model path must exist on disk and be a directory.
  - Input device must be available in the system audio device list.
  - Hotkey must not conflict with reserved keybindings.

## Non-Functional Requirements

- **NFR-001**: Settings screen renders within one frame (under 16ms) when switching categories.
- **NFR-002**: Config file read/write operations complete within 50ms.
- **NFR-003**: Settings changes take effect without requiring application restart (where technically feasible).
- **NFR-004**: All settings UI text in English only.

## Implementation Details

### Config File Schema (`~/.gwt/config.toml`)

```toml
# General
[general]
protected_branches = ["main", "develop"]
default_base_branch = "develop"
worktree_root = "~/.gwt/worktrees"
debug = false
profiling = false

# Agent defaults
[agent]
default_agent = "claude"
auto_install_deps = false

[agent.paths]
claude = "/usr/local/bin/claude"
codex = ""
gemini = ""

# Custom agents
[tools.customCodingAgents.my-agent]
id = "my-agent"
displayName = "My Agent"
agentType = "command"
command = "my-agent-cli"
defaultArgs = []
[tools.customCodingAgents.my-agent.modeArgs]
normal = []
continue = ["--continue"]
resume = ["--resume"]
[tools.customCodingAgents.my-agent.models]
default = { id = "default", label = "Default", arg = "" }

# Profiles
[[profiles]]
name = "default"
description = "Default profile"
active = true
[profiles.env]
MY_VAR = "value"
[profiles.disabled_env]
UNWANTED_VAR = true

# AI settings
[ai]
endpoint = "https://api.openai.com/v1"
api_key = ""
model = "gpt-4o"
language = "en"
summary_enabled = false

# Voice settings
[voice]
model_path = ""
hotkey = "Ctrl+G,v"
input_device = "system_default"
language = "auto"
enabled = false
```

### Config File Priority Order

1. CLI flags (highest)
2. Environment variables
3. `~/.gwt/config.toml`
4. Built-in defaults (lowest)

## Success Criteria

- **SC-001**: All 6 existing settings categories continue to function correctly.
- **SC-002**: Voice settings category renders with all 5 fields and correct defaults.
- **SC-003**: Voice settings round-trip: save to config.toml and reload produces identical values.
- **SC-004**: Validation rejects invalid model paths and unavailable audio devices with user-friendly messages.
- **SC-005**: Settings tests cover all voice settings fields including edge cases.
