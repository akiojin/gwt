# gwt

[日本語](README.ja.md)

gwt is a desktop control plane for agent-driven development. It brings coding
agents, project context, shared coordination, GitHub Issue-backed specs,
semantic search, and managed workflow automation into one native GUI and
browser-accessible workspace.

Git worktrees are the isolation substrate behind gwt. They let gwt materialize
safe per-task workspaces for agents, but the product flow starts from work,
Issues, SPECs, search, and Board context rather than from branch management.

## Why gwt

- **Agent workspace** — launch, resume, and monitor `Claude Code`, `Codex`,
  `Gemini`, `OpenCode`, `Copilot`, and custom agents from a shared canvas.
- **Shared Board** — keep user and agent communication in one repo-scoped
  timeline with `status`, `claim`, `next`, `blocked`, `handoff`, `decision`,
  and `question` posts.
- **Agent-to-agent coordination** — managed hooks remind agents to post
  reasoning milestones and inject recent Board context so parallel agents can
  see decisions, handoffs, blockers, and targeted requests.
- **Semantic Knowledge Bridge** — search Issues, SPECs, project source files,
  and docs through a ChromaDB / multilingual-e5 index instead of relying only
  on substring matches.
- **GitHub Issue-backed SPECs** — treat `gwt-spec` Issues as the source of
  truth while reading and editing sections through the local cache-backed CLI.
- **Managed workflow skills** — use bundled `gwt-*` skills for discussion,
  issue routing, planning, TDD implementation, PR work, architecture review,
  project search, and agent-pane management.
- **Operator canvas** — arrange Agent, Board, Issue, SPEC, Logs, Memo, Profile,
  File Tree, Branches, and PR surfaces in one mission-control style workspace.

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
running `gwtd daemon start` brings up a per-project runtime daemon
(Unix-domain socket IPC) that multi-instance event fan-out depends on
— for example, with the daemon running, Board posts you make in one
`gwt` window appear in another instance opened on the same repo
without a polling delay. The daemon keeps running in the background
until you stop it (Ctrl-C or SIGTERM). `gwtd daemon status` prints
the live endpoint for diagnostics. Without `gwtd daemon start`,
multi-instance fan-out is inactive but local file-based state and
the file watcher continue to work as before.

Windows currently has no long-running daemon: `gwtd daemon start`
exits with "not yet implemented", and managed hooks fall back to
synchronous `gwt hook ...` dispatch. Multi-instance fan-out is
therefore unavailable on Windows pending follow-up work; `gwtd
daemon status` still works there but always reports `stopped` until
the named-pipe path lands.

## Agent Workflow

1. Open a project directory or restore the previous project.
2. Use `Board`, `Issue`, `SPEC`, and Knowledge search surfaces to understand
   the current work, related owners, and prior decisions.
3. Choose `Start Work` from the Project Bar or Command Palette when the task is
   still work-shaped rather than branch-shaped.
4. Launch an `Agent` from Start Work, or launch directly from an Issue/SPEC
   detail when the owner is already known.
5. Let gwt materialize the backing `work/YYYYMMDD-HHMM[-n]` branch/worktree
   only when launch is confirmed.
6. Use the shared Board for status, claims, next steps, blockers, handoffs,
   and decisions while agents run.
7. Open `Branches` only when you need Git inspection, filtering, cleanup, or
   lower-level branch/worktree details.

Common windows include:

- `Agent` — live coding-agent process windows created through Start Work or
  Launch Agent
- `Board` — shared user/agent timeline for reasoning and coordination
- `Issue` and `SPEC` — cache-backed Knowledge Bridge windows with semantic
  search, detail panes, and Launch Agent handoff
- `Logs` — project diagnostics and live log surface
- `Memo` and `Profile` — repo-scoped notes and environment/profile management
- `File Tree` — live read-only repository tree
- `Branches` — branch inspection, filtering, cleanup, and Git details
- `Settings` — application and agent configuration
- `PR` — pull-request workflow surface; detailed list support depends on the
  cache-backed PR source as it lands

`Agent` is the live process window for coding-agent sessions. `Board` is the
coordination surface agents use to expose status, decisions, handoffs, and
requests. `Issue` and `SPEC` use the local cache and semantic index rather than
rendering direct GitHub API responses in the frontend.

On Windows Host launches, Launch Agent lets you choose Command Prompt, Windows
PowerShell, or PowerShell 7. Docker launches continue to use the container
shell.

In terminal windows, drag to select text and release the mouse button to copy.
On Windows and Linux, `Ctrl+Shift+C` also copies the current terminal
selection. `Ctrl+C` stays mapped to the running terminal process.

## Knowledge, Search, and Managed Skills

gwt keeps project knowledge close to the agent workspace:

- `gwtd issue spec <n>` reads GitHub Issue-backed SPECs from the local cache.
- `gwtd issue view <n>` and `gwtd issue comments <n>` provide cache-backed Issue
  access through the gwt CLI surface.
- `gwt-search` searches SPECs, Issues, source files, and docs through the shared
  ChromaDB runtime. Missing indexes are built on demand, and the desktop app can
  repair the managed Python search runtime when needed.
- The Issue/SPEC Knowledge Bridge windows combine cache-backed list/detail views
  with semantic ranking, exact-match priority, and match percentages.

Bundled workflow skills are materialized into `.claude/skills`,
`.claude/commands`, and `.codex/skills` for the active worktree. The public
entrypoints are:

- `gwt-discussion` — investigation-first discussion and design clarification
- `gwt-register-issue` / `gwt-fix-issue` — issue intake and issue-driven fixes
- `gwt-plan-spec` — implementation planning for an approved SPEC
- `gwt-build-spec` — TDD-oriented implementation from an approved task
- `gwt-manage-pr` — PR create/check/fix lifecycle
- `gwt-arch-review` — architecture review and improvement routing
- `gwt-search` — unified semantic search
- `gwt-agent` — running agent-pane inspection and control

Managed hooks preserve user hooks while adding gwt runtime behavior for agent
state, workflow guardrails, Board reminders, discussion/plan/build Stop checks,
and coordination-event summaries.

## Workspace Foundation

For isolation and repeatable agent sessions, gwt can manage each project as a
**Nested Bare + Worktree** layout under your workspace directory:

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

## Operator Design Language (SPEC-2356)

Starting with the Operator Design System update, gwt is themed as a single
mission-control surface with editorial-industrial typography (`Mona Sans` for
body, `Hubot Sans` condensed for display, `JetBrains Mono` for terminal /
counters). Every chrome surface — Project Bar, Sidebar Layers, Status Strip,
Command Palette, Hotkey Overlay, Drawer modals, floating windows — shares a
single token system that ships in two flagship themes:

- **Dark Operator** (Mission Control / carbon + neon) — the default, optimized
  for long sessions
- **Light Operator** (Drafting Table / bone + ink) — for bright environments

The active theme follows your OS `prefers-color-scheme`, but the **Theme**
toggle in the Project Bar lets you cycle `auto → dark → light → auto`. The
choice is persisted in browser storage and survives restarts. xterm terminal
panes follow the overall theme automatically. `prefers-reduced-motion: reduce`
disables the Living Telemetry pulse rim, status strip ticking, and Mission
Briefing intro reveal so the UI stays usable in motion-sensitive environments.
`forced-colors: active` (Windows High Contrast / macOS Increase Contrast)
falls back to system colors so accessibility is preserved.

### Hotkeys

| Combo | Action |
| --- | --- |
| `⌘K` / `⌘P` | Open the Command Palette (fuzzy search over all surface actions) |
| `⌘B` | Focus the Board surface |
| `⌘G` | Focus the Git (Branches) surface |
| `⌘L` | Focus the Logs surface |
| `⌘?` | Toggle the Hotkey Overlay (cheat sheet) |
| `Esc` | Close any open palette / overlay / drawer |

## SPEC and Runtime Quick Reference

- SPEC source of truth: GitHub Issues labeled `gwt-spec`
- Local cache path:
  `~/.gwt/cache/issues/<repo-hash>/`
- Managed agent integration files:
  `.claude/settings.local.json` and `.codex/hooks.json`
- List available SPECs:

```bash
gwtd issue spec list
```

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
