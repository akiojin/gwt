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
