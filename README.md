# gwt

[日本語](README.ja.md)

gwt is a desktop GUI app for managing Git worktrees and launching coding agents
(Claude Code, Codex, Gemini, OpenCode).

## Install

### macOS (shell installer)

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

Or install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
```

### macOS (local `.pkg` installer)

Build a local package:

```bash
cargo tauri build
./installers/macos/build-pkg.sh
```

Install from local package:

```bash
./installers/macos/install.sh --pkg ./target/release/bundle/pkg/gwt-macos-$(uname -m).pkg
```

Or run both steps at once:

```bash
./installers/macos/install-local.sh
```

### Uninstall (macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

### Downloads

GitHub Releases are the source of truth for distribution.

Typical assets:

- macOS: `.dmg`, `.pkg`
- Windows: `.msi`
- Linux: `.AppImage`, `.deb`

## Usage (for users)

- Start the app after install, then open a repository from **Open Project...**.
- Select a branch to work with, then:
  - manage worktrees from the sidebar
  - launch an AI agent from the branch actions
- Open `Settings` to configure AI profiles if you plan to use agent or summary features.

The sections below are contributor/developer operations.

## Keyboard Shortcuts

| Shortcut (macOS) | Shortcut (Windows/Linux) | Action |
|---|---|---|
| Cmd+N | Ctrl+N | New Window |
| Cmd+O | Ctrl+O | Open Project |
| Cmd+C | Ctrl+C | Copy |
| Cmd+V | Ctrl+V | Paste |
| Cmd+Shift+K | Ctrl+Shift+K | Cleanup Worktrees |
| Cmd+, | Ctrl+, | Preferences |
| Cmd+Shift+[ | Ctrl+Shift+[ | Previous Tab |
| Cmd+Shift+] | Ctrl+Shift+] | Next Tab |
| Cmd+` | Ctrl+` | Next Window |
| Cmd+Shift+` | Ctrl+Shift+` | Previous Window |
| Cmd+M | --- | Minimize (macOS only) |

## Development

- Rust (stable)
- Node.js 22
- pnpm (via Corepack)
- Tauri system dependencies (per platform)

Run in dev:

```bash
cd gwt-gui
pnpm install --frozen-lockfile

cd ..
cargo tauri dev
```

Build:

```bash
cd gwt-gui
pnpm install --frozen-lockfile

cd ..
cargo tauri build
```

Playwright E2E (WebView UI smoke):

```bash
cd gwt-gui
pnpm install --frozen-lockfile
pnpm exec playwright install chromium
pnpm run test:e2e
```

CI runs the same Playwright suite in `.github/workflows/test.yml` (`E2E (Playwright)` job).

## AI Settings

Agent Mode and features like session summaries require AI settings.

Steps:

- Open `Settings`
- Select a profile in `Profiles`
- Enable `AI Settings`
- Set `Endpoint` and `Model` (API key is optional for local LLMs)
- Click `Save`

## Repository Layout

- `crates/gwt-core/`: core logic (Git/worktree/config/logs/docker/pty)
- `crates/gwt-tauri/`: Tauri v2 backend (commands + state)
- `gwt-gui/`: Svelte 5 frontend (UI + xterm.js)
- `installers/`: installer definitions (e.g. WiX)

## License

MIT
