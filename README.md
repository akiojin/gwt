# gwt

[日本語](README.ja.md)

gwt is a desktop GUI for managing Git worktrees and launching coding agents
such as `Claude Code`, `Codex`, `Gemini`, and `OpenCode`.

## Install

Download the binary for your platform from
[GitHub Releases](https://github.com/akiojin/gwt/releases) and place it in
your `PATH`.

### macOS

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
```

### Windows / Linux

Download the binary from GitHub Releases and add it to your `PATH`.

### Uninstall (macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

## Requirements

- `git` available in `PATH`
- `gh auth login` completed for GitHub-backed features
- AI provider credentials when you use agents:
  - `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` or `GEMINI_API_KEY`
- Python 3.9+ when gwt needs to bootstrap or repair the shared project index runtime

Linux desktop builds also require WebKitGTK-related system packages. See
[docs/docker-usage.md](docs/docker-usage.md) for the dependency set used in CI.

## Usage

Launch the native GUI:

```bash
gwt
```

At startup you can restore the previous session or open a new project
directory. The app also starts a local HTTP/WebSocket server for the WebView
surface and prints a URL such as `http://127.0.0.1:<port>/` to stderr. You can
open that URL in a regular browser while the native app is running.

CLI subcommands run in the same binary without opening a GUI window:

```bash
gwt issue spec 1784 --section plan
gwt pr current
gwt board show
gwt hook workflow-policy
```

## Main Workflow

1. Open a repository directory or restore the previous project.
2. Use the canvas to arrange floating windows.
3. Open `Branches`, select a branch, and double-click to open Launch Agent.
4. Start `Shell` or `Agent` windows on the selected branch/worktree.
5. Inspect the repository with the read-only `File Tree` window.

Available windows include:

- `Shell`
- `Agent`
- `Branches`
- `File Tree`
- `Settings`
- `Memo`
- `Profile`
- `Logs`
- `Issue`
- `SPEC`
- `Board`
- `PR`

`Shell` and `Agent` are live process windows. `File Tree` is a live read-only
tree view. The remaining windows are currently mock surfaces where production
behavior has not been wired yet.

## Canvas Operations

- Zoom the canvas with the on-screen zoom buttons
- Pan the canvas by dragging the background
- Use `Tile` to arrange windows on a grid
- Use `Stack` to cascade windows with overlap
- Use `Cmd/Ctrl+Shift+Right` and `Cmd/Ctrl+Shift+Left` to cycle focus; the
  focused window is recentered

## Managed Hooks and SPEC Cache

- gwt regenerates `.claude/settings.local.json` and merges `.codex/hooks.json`
  for managed hooks
- SPECs are stored as GitHub Issues labeled `gwt-spec`
- Local cache path:
  `~/.gwt/cache/issues/<repo-hash>/`
- Read a SPEC:

```bash
gwt issue spec <number>
```

- Read one section:

```bash
gwt issue spec <number> --section spec|plan|tasks
```

## Logs

- App logs:
  `~/.gwt/logs/<repo-hash>/gwt.log.YYYY-MM-DD`
- Workspace state:
  `~/.gwt/workspace-state.json`

## Development

### Build

```bash
cargo build -p gwt
```

### Run

```bash
cargo run -p gwt
```

### Build a macOS app bundle

```bash
cargo install cargo-bundle
cargo bundle -p gwt --format osx
```

### Test

```bash
cargo test -p gwt-core -p gwt --all-features
```

### Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Format

```bash
cargo fmt
```

## Project Structure

```text
├── Cargo.toml          # Workspace configuration
├── crates/
│   ├── gwt/            # Desktop GUI + WebView server + CLI dispatch
│   ├── gwt-core/       # Core library
│   └── gwt-github/     # GitHub Issue SPEC cache / update layer
└── package.json        # npm package metadata and postinstall
```

## Specs

Detailed requirements live in GitHub Issues labeled `gwt-spec`. Use
`gwt issue spec <n>` to inspect them locally through the cache-backed CLI.

## License

MIT
