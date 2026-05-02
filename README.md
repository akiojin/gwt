# gwt

[日本語](README.ja.md)

gwt is a desktop GUI for managing Git worktrees and launching coding agents
such as `Claude Code`, `Codex`, `Gemini`, and `OpenCode`.

## Install

Download the release asset for your platform from
[GitHub Releases](https://github.com/akiojin/gwt/releases).

### macOS

- GUI-first installer: `gwt-macos-universal.dmg`
- Open `GWT.app` from the mounted DMG for the native desktop launch surface
- Use the install script when you want the `gwt` and `gwtd` CLIs in your `PATH`

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version <version>
```

### Windows

- GUI-first installer: `gwt-windows-x86_64.msi`
- Portable bundle: `gwt-windows-x86_64.zip`
- The public front door is `gwt.exe`; `gwtd.exe` is bundled for internal runtime use

### Linux

- Portable bundles:
  - `gwt-linux-x86_64.tar.gz`
  - `gwt-linux-aarch64.tar.gz`
- Extract `gwt` and `gwtd` into a directory on your `PATH`

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

CLI subcommands run through `gwtd` without opening a GUI window:

```bash
gwtd issue spec 1784 --section plan
gwtd pr current
gwtd board show
gwtd hook workflow-policy
gwtd daemon status            # inspect the per-project runtime daemon
```

Managed hooks and runtime delegation use `gwtd`. On macOS and Linux,
the first `gwt` command or GUI launch auto-bootstraps a per-project
runtime daemon (Unix-domain socket IPC) so events can fan out to every
`gwt` instance attached to the same project — for example, Board posts
you make in one window appear in another instance opened on the same
repo without a polling delay. The daemon keeps running in the
background after you close the GUI; subsequent `gwt` invocations on the
same project reuse it, and a stale entry from a crashed daemon is
cleaned up automatically on the next launch. `gwtd daemon status`
prints the live endpoint for diagnostics.

Windows currently has no long-running daemon: `gwtd daemon start`
exits with "not yet implemented", and managed hooks fall back to
synchronous `gwt hook ...` dispatch. Multi-instance fan-out is
therefore unavailable on Windows pending follow-up work; `gwtd
daemon status` still works there but always reports `stopped` until
the named-pipe path lands.

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

On Windows Host launches, Launch Agent lets you choose Command Prompt, Windows
PowerShell, or PowerShell 7. The selected shell applies to both `Shell` and
`Agent` windows; Docker launches continue to use the container shell.

In terminal windows, drag to select text and release the mouse button to copy.
On Windows and Linux, `Ctrl+Shift+C` also copies the current terminal
selection. `Ctrl+C` stays mapped to the running terminal process.

## Workspace Layout

gwt manages each project as a **Nested Bare + Worktree** layout under your
workspace directory:

```
<workspace>/<project>/
├── <project>.git/          # bare repository
├── develop/                # develop worktree (default working directory)
├── feature/<name>/         # additional worktrees by branch
└── .gwt/project.toml       # gwt-managed project metadata
```

`gwt` auto-creates this layout when you clone through the Initialization
wizard. Existing Normal Git repositories (`.git/` directly under the project
directory) are recognised so a migration to the Nested Bare + Worktree layout
can be run on demand. The migration safely backs up the original tree to
`.gwt-migration-backup/`, rebuilds the bare repo, recreates each worktree,
and rolls back automatically if any phase fails. Tracking work is captured in
[GitHub Issue #1934 (SPEC-1934)](https://github.com/akiojin/gwt/issues/1934).

To migrate an existing Normal Git project, open it from gwt's project
picker (or via `Reopen Recent`). gwt detects the layout and shows a
confirmation modal with three actions:

- **Migrate** — run the migration now. Progress is streamed phase by phase
  (Validate → Backup → Bareify → Worktrees → Submodules → Tracking →
  Cleanup → Done). On success the project tab reloads onto the new branch
  worktree without restarting the app.
- **Skip** — open the project as a Normal Git checkout. The modal will
  reappear the next time you open the project.
- **Quit** — close the app without touching the repository.

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
gwtd issue spec <number>
```

- Read one section:

```bash
gwtd issue spec <number> --section spec|plan|tasks
```

## Logs

- App logs:
  `~/.gwt/projects/<repo-hash>/logs/gwt.log.YYYY-MM-DD`
- Session state:
  `~/.gwt/session.json`
- Project workspace state:
  `~/.gwt/projects/<repo-hash>/workspace.json`

## Development

### Build

```bash
cargo build -p gwt --bin gwt --bin gwtd
```

### Run

```bash
cargo run -p gwt --bin gwt
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

### Release Asset Contract

```bash
npm run test:release-assets
```

### Frontend Bundle Contract

```bash
npm run test:frontend-bundle
```

### Release Flow Checks

```bash
npm run test:release-flow
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
`gwtd issue spec <n>` to inspect them locally through the cache-backed CLI.

## License

MIT
