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

## Development

Prereqs:

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

## Repository Layout

- `crates/gwt-core/`: core logic (Git/worktree/config/logs/docker/pty)
- `crates/gwt-tauri/`: Tauri v2 backend (commands + state)
- `gwt-gui/`: Svelte 5 frontend (UI + xterm.js)
- `installers/`: installer definitions (e.g. WiX)

## License

MIT
