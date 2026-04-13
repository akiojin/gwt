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

Most TUI key bindings use the `Ctrl+G` prefix. Terminal text copy uses the
platform shortcut when a selection is active.

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

When terminal text is selected, use:

- macOS: `Cmd+C`
- Linux / Windows: `Ctrl+Shift+C`

To investigate Japanese IME candidate selection in a shell or agent terminal,
launch gwt with `GWT_INPUT_TRACE_PATH=/tmp/gwt-input-trace.jsonl`. The JSONL
trace records raw `crossterm` key events, keybind decisions, and forwarded PTY
bytes without adding a runtime input-mode toggle.

To compare that routed trace with raw terminal input, run
`cargo run -p gwt-tui --example keytest -- --mode raw` in the same terminal.
The probe logs every raw `crossterm::event::Event` to
`/tmp/gwt-crossterm-events.jsonl` by default, or to the positional output path
you pass.

To isolate redraw-related IME regressions, the same probe also supports
`--mode redraw` and `--mode ratatui`. `redraw` repaints the same committed-text
surface on a periodic tick using direct `crossterm` commands, while `ratatui`
uses the same surface through ratatui on the same tick. Use `--tick-ms <N>` to
change the redraw interval when comparing modes.

gwt also requests minimal kitty keyboard enhancements during terminal startup
(`DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES`) and pops them on shutdown.
Unsupported terminals fail open and continue with existing behavior. Repeated
key events from compatible terminals now stay on the same input path as normal
key presses, which matters when IME candidate navigation advances to another
page. While the terminal pane owns focus, idle 100 ms ticks now avoid repainting
the TUI unless an overlay or other explicit periodic UI surface still needs
animation, which reduces IME candidate interruption from background redraws.
PTY output still requests an immediate redraw, so committed text and normal
shell output are not delayed until the next keypress.

## Environment and requirements

### Required

- `git` command available in `PATH`.

### Optional (depends on use)

- AI provider keys in environment variables (or saved in gwt profile settings):
  - `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` or `GEMINI_API_KEY`
- `bunx` or `npx` for local agent launch fallback.
- Python 3.9+ on `PATH` when gwt needs to bootstrap or repair the shared project-index runtime
  (for example during startup or repository initialization).
  gwt bootstraps `~/.gwt/runtime/chroma-venv` automatically, then reuses that managed runtime.
  On Windows, make sure either `python` or `py -3` works in Command Prompt or PowerShell.
- Vector index data (SPECs / Issues / source files) is stored under `~/.gwt/index/<repo-hash>/`.
  Issues and SPECs are repo-scoped and shared across worktrees; source files are worktree-scoped.
  The TUI runs a per-Worktree filesystem watcher and refreshes the Issue index asynchronously
  (15-minute TTL) at startup. The first search after a fresh install downloads the
  `intfloat/multilingual-e5-base` embedding model (~440 MB) into `~/.cache/huggingface/`.
  SPECs live as GitHub Issues labeled `gwt-spec` and are cached locally at
  `~/.gwt/cache/issues/<repo-hash>/`; use `gwt issue spec <n>` to read and `gwt issue spec <n> --edit <section> -f <file>` to write.

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
│   ├── gwt-github/     # GitHub Issue SOT for SPEC management (SPEC-12)
│   └── gwt-tui/        # ratatui TUI frontend + CLI dispatch (`gwt issue spec ...`)
└── package.json        # Development scripts
```

## License

MIT
