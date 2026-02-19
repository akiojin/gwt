# Windows Verification Handoff

## Context
- Date: 2026-02-19
- Branch: `feature/enable-terminal-selection`
- Latest commit on branch: `9ce8c814`
- Purpose: verify Windows-specific behavior after shell selection and WSL launch regression fixes.

## Fixed Items
- Frontend now sends `terminalShell` (camelCase), so backend receives shell override correctly.
- `spawn_shell` now reads `terminal.default_shell` from settings when `shell` is not explicitly provided.
- WSL prompt-detection reader no longer stops streaming when detector channel receiver is dropped.
- WSL launch path now uses merged `env_vars` (profile/env overrides included), same as host launch path.
- `cmd.exe /C` command expression now quotes tokens with spaces/metacharacters.

## Automated Checks Already Passed
- `cargo fmt --all`
- `cargo test -p gwt-core resolve_spawn_command_cmd_shell -- --nocapture`
- `cargo test -p gwt-core build_cmd_command_expression_escapes_embedded_quotes -- --nocapture`
- `cargo test -p gwt-tauri resolve_shell_id_for_spawn_ -- --nocapture`
- `cargo test -p gwt-tauri build_wsl_inject_command_ -- --nocapture`
- `cd gwt-gui && pnpm test src/lib/components/AgentLaunchForm.test.ts src/lib/components/AgentLaunchForm.glm.test.ts`

## Manual Verification on Windows (Required)

### 1) Launch Agent shell override (UI -> backend request)
- Open Launch Agent dialog.
- In advanced options, choose each shell override:
  - `PowerShell`
  - `cmd`
  - `WSL`
- Launch each and confirm the actually opened shell matches selection.
- Expected: override takes effect immediately (no fallback to auto shell).

### 2) New Terminal respects Settings default shell
- Open Settings.
- Set default shell to `wsl`, save.
- Click `New Terminal` (without explicit shell parameter).
- Expected: terminal opens via WSL.
- Repeat with `powershell` and `cmd` as default shell values.

### 3) WSL output stream continuity after prompt detection
- Launch an agent with `WSL` shell override.
- Run a command that emits multiple output chunks over time (or a longer agent interaction).
- Expected: output continues normally after initial prompt detection; no early truncation.

### 4) WSL environment propagation parity with host launch
- Configure profile env vars in settings/profile (for example `FOO=bar`).
- Launch same agent once on host shell, once on WSL shell.
- Inside each, print env (`echo $FOO` in WSL/PowerShell equivalent).
- Expected: profile-derived and override env vars are present in both host and WSL launches.

### 5) cmd /C quoting robustness
- On Windows, run a launch path that includes spaces in executable or args
  - Example path under `C:\Program Files\...`
  - Example arg containing spaces or `&`
- Expected: command starts successfully and arguments are not split/corrupted.

## If Failure Is Found
- Capture:
  - exact UI action
  - expected vs actual behavior
  - shell type used
  - minimal reproduction steps
  - relevant logs/snippets
- Attach failure note to this file or report with same section numbering (1-5).
