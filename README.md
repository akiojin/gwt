# gwt

[日本語](README.ja.md)

gwt is a terminal-based (TUI) tool for managing Git worktrees and launching
coding agents (`Claude Code`, `Codex`, `Gemini`, `OpenCode`) on a project basis.

## Install

Download the binary for your platform from
[GitHub Releases](https://github.com/akiojin/gwt/releases) and place it in
your `PATH`.

### macOS

Run the installer:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
```

### Windows

Download the binary from GitHub Releases and add it to your `PATH`.

### Linux

Download the binary from GitHub Releases and add it to your `PATH`.

### Uninstall (macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

## Usage

Launch the TUI in the current directory:

```bash
gwt
```

### Terminal requirements

- 256-color terminal recommended (most modern terminals support this)
- Minimum 80x24 terminal size

## First-time usage

1. Run `gwt` in a Git repository.
2. Browse branches and worktrees in the sidebar.
3. Use branch actions to:
   - create/list/clean worktrees
   - launch an agent
4. Open **Settings** to set up AI profile settings if you use Agent or
   summary features.

## Key bindings

All TUI key bindings use the `Ctrl+G` prefix.

| Key binding | Action |
|---|---|
| `Ctrl+G`, `c` | New shell tab |
| `Ctrl+G`, `n` | New agent tab |
| `Ctrl+G`, `1`-`9` | Switch to tab N |
| `Ctrl+G`, `]` | Next tab |
| `Ctrl+G`, `[` | Previous tab |
| `Ctrl+G`, `x` | Close current tab |
| `Ctrl+G`, `w` | Worktree list |
| `Ctrl+G`, `s` | Settings |
| `Ctrl+G`, `?` | Help / key binding reference |
| `Ctrl+G`, `q` | Quit |

## Environment and requirements

### Required

- `git` command available in `PATH`.

### Optional (depends on use)

- AI provider keys in environment variables (or saved in gwt profile settings):
  - `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` or `GEMINI_API_KEY`
- `bunx` or `npx` for local agent launch fallback.
- Python 3.9+ on `PATH` when you use project search / issue indexing for the first time.
  gwt bootstraps `~/.gwt/runtime/chroma-venv` automatically, then reuses that managed runtime.
  On Windows, make sure either `python` or `py -3` works in Command Prompt or PowerShell.

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

### Optional advanced toggles

- `GWT_AGENT_AUTO_INSTALL_DEPS` (`true` / `false`)
- `GWT_DOCKER_FORCE_HOST` (`true` / `false`)

### Logging and profiling

Normal logs are stored as JSON Lines under `~/.gwt/logs/`. Performance profiling can be enabled in **Settings > Profiling**.
See [#1758](https://github.com/akiojin/gwt/issues/1758) for the logging specification.

## Development

### Build

```bash
cargo build -p gwt-tui
```

### Run (development)

```bash
cargo run -p gwt-tui
```

### Test

```bash
cargo test -p gwt-core -p gwt-tui
```

### Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Format

```bash
cargo fmt
```

## Project structure

```text
├── Cargo.toml          # Workspace configuration
├── crates/
│   ├── gwt-core/       # Core library (Git operations, PTY management, config)
│   └── gwt-tui/        # ratatui TUI frontend
├── specs/              # Local SPEC management (SPEC-{N}/)
└── package.json        # Development scripts
```

## License

MIT
