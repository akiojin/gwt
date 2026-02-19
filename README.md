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

- `.dmg`, `.pkg`

### Windows

Download `.msi` from GitHub Releases and run the installer.

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
| Cmd+V | Ctrl+V | Paste |
| Cmd+Shift+C | Ctrl+Shift+C | Copy Screen Text |
| Cmd+Shift+K | Ctrl+Shift+K | Cleanup Worktrees |
| Cmd+, | Ctrl+, | Preferences |
| Cmd+Shift+[ | Ctrl+Shift+[ | Previous Tab |
| Cmd+Shift+] | Ctrl+Shift+] | Next Tab |
| Cmd+` | Ctrl+` | Next Window |
| Cmd+Shift+` | Ctrl+Shift+` | Previous Window |
| Cmd+M | --- | Minimize (macOS only) |

## Environment and requirements

### Required

- `git` command available in `PATH`.

### Optional (depends on use)

- AI provider keys in environment variables (or saved in gwt profile settings):
  - `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` or `GEMINI_API_KEY`
- `bunx` or `npx` for local agent launch fallback.

### Optional advanced toggles

- `GWT_AGENT_AUTO_INSTALL_DEPS` (`true` / `false`)
- `GWT_DOCKER_FORCE_HOST` (`true` / `false`)

## License

MIT
