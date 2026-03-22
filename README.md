# gwt

[日本語](README.ja.md)

gwt is a desktop app for managing Git worktrees and launching coding agents
(`Claude Code`, `Codex`, `Gemini`, `OpenCode`) on a project basis.

## Install

### macOS

Run the installer:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
```

Downloadable formats in Releases:

- `.dmg`

Build installers locally (one command):

```bash
pnpm run installer:macos
```

Fast local app install for iterative testing (without waiting for a GitHub Release):

```bash
pnpm run install:local:macos
```

Reinstall the already-built local `.app` bundle without rebuilding:

```bash
pnpm run install:local:macos:skip-build
```

This installs the local build directly to `/Applications/gwt.app`.

### Windows

Download `.msi` from GitHub Releases and run the installer.

Build installer locally (one command, PowerShell):

```powershell
pnpm run installer:windows
```

### Linux

Download one of:

- `.deb`
- `.AppImage`

Run with your OS standard installer method.

### Uninstall (macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

## First-time usage

1. Open gwt.
2. Click **Open Project...** and select a Git repository.
3. Open or switch branches in the sidebar.
4. Use the branch actions to:
   - create/list/clean worktrees
   - launch an agent
5. Open **Settings** to set up AI profile settings if you use Agent or summary features.

## Automatic updates

gwt checks for updates from GitHub Releases.

- On app startup, it checks updates automatically.
- If it cannot check at first, it retries a few times automatically.
- When an update is available, you get a notification.
- You can also trigger manual check from the menu: **Help → Check for Updates...**.

If a compatible installer/payload is available, gwt can apply it directly.
If automatic apply is not possible, the update dialog tells you to download from Releases manually.

## Keyboard shortcuts

| Shortcut (macOS) | Shortcut (Windows/Linux) | Action |
|---|---|---|
| Cmd+N | Ctrl+N | New Window |
| Cmd+O | Ctrl+O | Open Project |
| Cmd+C | Ctrl+C | Copy |
| Cmd+V | Ctrl+V | Paste (text fields / native menu fallback) |
| Cmd+V (terminal pane) | Ctrl+Shift+V (terminal pane) | Paste text into terminal pane |
| Cmd+Shift+C | Ctrl+Shift+C | Copy Screen Text |
| Cmd+Shift+K | Ctrl+Shift+K | Cleanup Worktrees |
| Cmd+, | Ctrl+, | Preferences |
| Cmd+Shift+[ | Ctrl+Shift+[ | Previous Tab |
| Cmd+Shift+] | Ctrl+Shift+] | Next Tab |
| Cmd+` | Ctrl+` | Next Window |
| Cmd+Shift+` | Ctrl+Shift+` | Previous Window |
| Cmd+M | --- | Minimize (macOS only) |

### Terminal pane note (Windows/Linux)

- In terminal-based tabs (`Agent` / `Terminal`), text paste is `Ctrl+Shift+V`.
- `Ctrl+V` is intentionally passed through to the terminal application (for example, Codex image paste).
- This behavior is terminal-level and is shared across PowerShell, WSL, and Cmd.

## Environment and requirements

### Required

- `git` command available in `PATH`.

### Optional (depends on use)

- AI provider keys in environment variables (or saved in gwt profile settings):
  - `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` or `GEMINI_API_KEY`
- `bunx` or `npx` for local agent launch fallback.

### GitHub Token (PAT) requirements

gwt uses `gh` CLI for GitHub operations. Authenticate with:

```bash
gh auth login
```

#### Fine-grained PAT recommended permissions

| Permission | Access | Used for |
|---|---|---|
| **Contents** | Read and Write | Repository browsing, branch operations, releases |
| **Pull requests** | Read and Write | PR create / edit / merge / review |
| **Issues** | Read and Write | Issue create / edit / comment |
| **Metadata** | Read | Implicitly granted |

#### Read-only minimum

For browse-only usage (no PR creation or branch management):

| Permission | Access |
|---|---|
| **Contents** | Read |
| **Pull requests** | Read |
| **Issues** | Read |
| **Metadata** | Read |

### Voice Accuracy Evaluation

You can measure WER/CER with a local speech dataset.

```bash
cp tests/voice_eval/manifest.template.json tests/voice_eval/manifest.json
scripts/voice-eval.sh
```

See `tests/voice_eval/README.md` for details.
For a versioned benchmark snapshot, see `docs/voice-eval-benchmarks.md`.

### Voice Input Runtime (Qwen3-ASR)

Voice input uses Qwen3-ASR via a local Python runtime.

- Required: Python 3.11+ available on `PATH` (or set `GWT_VOICE_PYTHON`).
- Not required manually: `qwen_asr` package installation.
- On first voice use, gwt auto-creates `~/.gwt/runtime/voice-venv` and installs runtime deps there.
- The selected Qwen model is then downloaded into Hugging Face cache on demand.
- Push-to-talk is fixed to `Cmd+Shift+Space` on macOS and `Ctrl+Shift+Space` on Windows/Linux.
- In the terminal overlay, hold the Voice button to capture speech.

### Optional advanced toggles

- `GWT_AGENT_AUTO_INSTALL_DEPS` (`true` / `false`)
- `GWT_DOCKER_FORCE_HOST` (`true` / `false`)

## License

MIT
