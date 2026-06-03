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
- **Operator canvas** — arrange Agent, Board, Issue, SPEC, Logs, Profile,
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
- If double-clicking the MSI appears to do nothing, run the diagnostic script
  from PowerShell and attach the output directory when reporting the issue:

```powershell
$diag = "$env:TEMP\diagnose-windows-msi.ps1"
Invoke-WebRequest `
  https://raw.githubusercontent.com/akiojin/gwt/main/scripts/diagnose-windows-msi.ps1 `
  -OutFile $diag
powershell -ExecutionPolicy Bypass -File $diag `
  -MsiPath "$env:USERPROFILE\Downloads\gwt-windows-x86_64.msi"
```

The script records the MSI SHA256, Authenticode signature, Zone.Identifier
download marker, Windows Installer `msiexec` verbose log, installed file layout,
and basic `gwt.exe` launch evidence.

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

Launching `gwt` installs a system-tray icon (macOS menubar / Windows
notification area / Linux StatusNotifierItem-capable DE). Drive it from
the tray menu:

- **Open in browser** — launches the OS default browser at
  `http://127.0.0.1:<port>/`. The same URL can be opened in any other
  browser too.
- **About GWT** — opens the browser About / Version surface for the
  running tray process.
- **Quit** — gracefully shuts the tray icon, embedded server, and
  PTY children down in order.

Autostart lives in **Settings > System > Launch GWT at login**. Enabling it
installs an OS-native per-user entry (macOS LaunchAgent / Windows HKCU Run /
Linux XDG autostart) via the `auto-launch` crate, so `gwt` resumes at the next
OS login as a tray-resident process. The browser is not opened automatically.

```bash
gwt                                 # install tray + start embedded server (loopback)
gwt --bind 0.0.0.0 --port 60745     # bind the embedded server to a LAN/VPN-reachable address
gwt open                            # open the running tray's URL in the OS default browser
```

`--bind <ip>` and `--port <n>` default to `127.0.0.1` and `0` (ephemeral). Pass `--bind 0.0.0.0` to make the embedded UI reachable from other hosts on the same LAN or VPN-extended LAN; pair it with `--port` when you need a stable, well-known port. `--no-tray` and `--no-open` are accepted today but currently no-op while the rest of SPEC #2920 Phase 4 lands.

`gwt open` is the Linux fallback for desktops that do not run a
StatusNotifierItem host (e.g. GNOME 3.26+ without the AppIndicator
extension). The embedded server still starts and prints
`gwt browser URL: ...` to stderr, so you can open the URL by hand or
through `gwt open`.

The tray-resident process is one per OS-login user. Launching `gwt`
twice for the same user makes the second invocation print the
existing URL to stderr and exit 0 instead of starting a second
server.

### `gwt serve` removal

The legacy `gwt serve` / `gwt --headless` verbs were removed in
v10.0.0 (SPEC #2920 Q9). CI / automation scripts that relied on
the old command should use the current `gwt` invocation instead.
`gwt browser URL: ...` is still written to stderr and
`GWT_BROWSER_URL_FILE` still receives the bound URL after the embedded
server starts.

Trust boundary: **LAN only** (including VPN-extended LAN). The embedded
browser server does not ship TLS termination, an authentication gate, or rate
limiting. Anyone that can reach the bind address can drive the embedded UI,
which includes spawning terminals. The `--bind` flag is opt-in: the default
`127.0.0.1` keeps the same loopback-trust behaviour as the native GUI. For
external access, run the host behind a VPN (Tailscale, WireGuard, etc.) rather
than exposing the port to the public Internet.

Platform note: on Linux, `tao 0.35` still requires a display server (X11 or
Wayland) at EventLoop creation. macOS and Windows browser-server launches
need no additional display setup; Linux operators in pure-headless
environments (no DISPLAY) should use `Xvfb`/`xvfb-run` or wait for the
tao-detach follow-up tracked under SPEC-1942.

Every HTTP / WebSocket request is mirrored to `tracing::info!(target =
"gwt_access", ...)` so the operator can see *which* peer is connecting in
real time on stderr and in `~/.gwt/logs/<date>/`. `/healthz` is demoted to
`debug!` to avoid drowning the stream with health probes.

Lifecycle: the running `gwt` process owns the agent / PTY lifetime. Closing a
browser tab does **not** stop running agents — only `Ctrl-C` / `SIGTERM` asks
the server to drain PTYs and exit gracefully. The tray-resident process is one
per OS-login user; a second `gwt` invocation prints the existing URL and exits
instead of starting a second server.

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

1. Open a project directory, clone from GitHub, or restore the previous
   project.
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
- `Profile` — environment/profile management
- `File Tree` — live read-only repository tree
- `Branches` — branch inspection, filtering, cleanup, and Git details
- `Settings` — application and agent configuration. The `System` tab lets
  you choose the narrative output language (Auto / English / 日本語) used
  for Workspace summaries and Board post bodies. `Auto` resolves against
  the OS locale and falls back to English when the locale is `C` / `POSIX`
  or unavailable. The setting is global and persisted under `[ai].language`
  in `~/.gwt/config.toml`. UI labels stay English (see SPEC-1933 NFR-005).
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
On Windows, `Ctrl+C` copies the current terminal selection and clears it; if no
selection exists, `Ctrl+C` stays mapped to the running terminal process. On
Linux, `Ctrl+Shift+C` also copies the current terminal selection.

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

When an agent is launched by gwt with a live GUI/browser backend, managed hooks
also enable the local hook-forward bridge. The bridge posts hook events only to
the loopback endpoint and bearer token that gwt injects for that session, then
fans them out through the existing live event stream. Sessions started outside
gwt do not receive that target and `gwt hook forward` remains a silent no-op;
stale targets, refused connections, validation errors, and delivery timeouts are
fail-open diagnostics and do not block agent tool calls.

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

`gwt` auto-creates this layout when you choose `Clone from GitHub...` from
either the Project Picker (shown when no tab is open) or the top toolbar's
`Open Project ▾` split-button dropdown (always reachable from an active
project). The clone modal accepts a GitHub HTTPS/SSH URL or lets you search
repositories through `gh search repos`, then asks for a destination parent
folder. The new project is created at `<parent>/<project>/`, with a bare
`<project>.git/` repository and an initial worktree on `develop` when it
exists, otherwise on the remote default branch.

Existing Normal Git repositories (`.git/` directly under the project
directory) are recognised so a migration to the Nested Bare + Worktree layout
can be run on demand. The migration safely backs up the original tree to
`.gwt-migration-backup/`, rebuilds the bare repo, recreates each worktree,
and rolls back automatically if any phase fails. Tracking work is captured in
[GitHub Issue #1934 (SPEC-1934)](https://github.com/akiojin/gwt/issues/1934).

To migrate an existing Normal Git project, open it from gwt's project
picker (or via `Reopen Recent`). gwt detects the layout and shows a
Migrate confirmation modal.

Choose **Migrate** to run the migration now. Progress is streamed phase by
phase (Validate -> Backup -> Bareify -> Worktrees -> Submodules -> Tracking ->
Cleanup -> Done). On success the project tab reloads onto the new branch
worktree without restarting the app.

## Board providers (Local / Slack / Teams)

The coordination **Board** can be backed by one of three providers, selected in
**Settings → System → Board provider**:

- **Local** (default) — filesystem-backed, offline, per-worktree. No setup.
- **Slack** — posts/reads live in a Slack channel via the Slack Web API.
- **Teams** — Microsoft Teams channel via Microsoft Graph. *Experimental: the
  code is implemented but has not yet been verified end to end against a real
  tenant. Treat as preview.*

Switching the provider swaps the entire Board content: each provider is its own
store, so the previously shown entries become invisible while the new provider
is active (switching back restores them). Secrets and OAuth tokens are stored in
a permission-restricted credential store under `~/.gwt/credentials/`, never in
`config.toml`.

### Use Slack as the Board backend

> 📷 *Screenshot placeholders are marked below. The Slack admin screens live at
> `api.slack.com` (account-specific) and the gwt screen is under Settings →
> System; add captures at each marked step.*

#### 1. Create a Slack app

1. Go to <https://api.slack.com/apps> → **Create New App** → **From scratch**.
2. Name it (e.g. `gwt`) and pick the target workspace → **Create App**.
   - 📷 *Screenshot: Create App dialog.*

#### 2. Add the redirect URL

1. In the app, open **OAuth & Permissions → Redirect URLs → Add New Redirect URL**.
2. Enter **exactly** the gwt OAuth callback URL and **Save URLs**:

   ```text
   http://127.0.0.1:8765/oauth/callback
   ```

   - Use `127.0.0.1` (not `localhost`), keep the `/oauth/callback` path, and no
     trailing slash. This must match gwt's **OAuth callback port** (default
     `8765`, changeable in Settings — see step 5). gwt shows the exact URL to
     register next to the port field.
   - 📷 *Screenshot: Redirect URLs with the callback saved.*

#### 3. Add bot scopes

1. **OAuth & Permissions → Scopes → Bot Token Scopes** → add:
   `chat:write`, `channels:history`, `channels:read`.
2. **Install App → Install to Workspace** (re-install after changing scopes /
   redirect URLs so they take effect).
   - 📷 *Screenshot: Bot Token Scopes list.*

#### 4. Copy the credentials

From **Basic Information → App Credentials**, note the **Client ID** and
**Client Secret**. Also pick the **Channel ID** of the target channel (in Slack:
channel → **View channel details** → bottom of the dialog).

#### 5. Configure gwt

1. In gwt, open **Settings → System → Board provider** and select **Slack**.
2. Fill the form and **Save configuration**:
   - **Client ID**, **Default channel ID**, **Client secret** (the secret is
     stored securely and never written to `config.toml`; the field clears after
     saving and shows "✓ A client secret is saved").
   - Optionally change the **OAuth callback port** (default `8765`); the form
     shows the exact Redirect URL to register in step 2. Changing it takes
     effect on the next launch.
   - 📷 *Screenshot: gwt Settings → System → Board provider = Slack (config form).*
3. Click **Sign in** → the browser opens the Slack consent screen → **Allow**.
   The callback page shows "Signed in / Connected the slack Board provider" and
   gwt flips to "Signed in to slack".
   - 📷 *Screenshot: Slack consent screen and the "Signed in" result.*

#### 6. Invite the bot to the channel

A Slack bot can only read or post in channels it has joined. In the target
channel, run:

```text
/invite @gwt
```

(replace `gwt` with your app name). Until the bot is a member, the Board shows
`conversations.history error: not_in_channel`. After inviting, posts made from
the gwt Board appear in the Slack channel, and channel messages appear on the
Board.

> The OAuth callback port only matters during sign-in. Once a token is stored,
> Board reads/writes use the token alone, so the port can change or be busy
> afterward without affecting an existing session — only a fresh sign-in needs
> the registered redirect URL again.

### Use Microsoft Teams as the Board backend (experimental)

> Teams support is implemented but not yet verified end to end against a real
> tenant. The steps below reflect the Microsoft identity / Graph requirements.

#### 1. Register an Entra (Azure AD) app

1. <https://entra.microsoft.com> → **App registrations → New registration**.
2. Name it `gwt` (single-tenant is fine).
3. **Redirect URI**: choose the **Mobile and desktop applications** (public
   client) platform and enter **exactly**:

   ```text
   http://127.0.0.1:8765/oauth/callback
   ```

   - Use `127.0.0.1` (the host gwt sends) and match gwt's OAuth callback port
     (default `8765`; the port is ignored for loopback matching, so
     `http://127.0.0.1/oauth/callback` also works).
   - If the portal rejects an http-loopback value, add it via the app
     **Manifest** as `replyUrlsWithType` with `"type": "InstalledClient"`.
   - ⚠️ **Do not register it under "Web"** — the public-client token exchange
     sends no client secret and a Web registration fails with
     `AADSTS invalid_client`.
4. **Authentication → Advanced settings → Allow public client flows → Yes**.

#### 2. Grant Microsoft Graph delegated permissions

**API permissions → Add a permission → Microsoft Graph → Delegated**:
`ChannelMessage.Send`, `ChannelMessage.Read.All`, `Channel.ReadBasic.All`,
`offline_access`. Grant admin consent if your tenant requires it.

#### 3. Find the team_id / channel_id

In Teams, open the channel → **Get link to channel**. In the URL,
`groupId=<GUID>` is the **team_id**, and the URL-decoded `19:...@thread.tacv2`
(after `/channel/`) is the **channel_id**. gwt's **Default channel** is
`<team_id>/<channel_id>`. (Alternatively, Graph Explorer:
`GET /me/joinedTeams`, then `GET /teams/{id}/channels`.)

#### 4. Configure gwt and sign in

**Settings → Board provider → Teams** → enter **Application (client) ID**,
**Tenant ID**, and **Default channel** (`team_id/channel_id`) → **Save** →
**Sign in**. Posts appear as the signed-in user (Graph delegated; app-only
channel posting is not supported). You must be a **member** of the target team
and channel — otherwise Graph returns `403` and gwt shows an actionable hint.

## Canvas Operations

- Zoom the canvas with the on-screen zoom buttons
- Pan the canvas by dragging the background
- Use `Tile` to arrange windows on a grid
- Use `Stack` to cascade windows with overlap
- Use `Align` to arrange windows on a grid without changing their size
- Use `Cmd/Ctrl+Shift+Right` and `Cmd/Ctrl+Shift+Left` to cycle focus; the
  focused window is recentered

## Operator Design Language (SPEC-2356)

Starting with the Operator Design System update, gwt is themed as a single
mission-control surface with editorial-industrial typography (`Mona Sans` for
body, `Hubot Sans` condensed for display, `JetBrains Mono` for terminal /
counters). The default type scale is tuned for developer readability, so
terminal text, IDs, paths, counters, and dense work surfaces stay legible during
long sessions while display typography remains reserved for headings and chrome
labels. Every chrome surface — Project Bar, Sidebar Layers, Status Strip,
Command Palette, Hotkey Overlay, Drawer modals, floating windows — shares a
single token system that ships in two flagship themes:

- **Dark Operator** (Mission Control / carbon + neon) — the default, optimized
  for long sessions
- **Light Operator** (Drafting Table / bone + ink) — for bright environments

The active theme follows your OS `prefers-color-scheme`, but the **Theme**
control in the Project Bar lets you choose `auto`, `dark`, or `light`. The
choice is persisted in browser storage and survives restarts. xterm terminal
content stays on the Dark Operator palette with larger developer-readable font
metrics, while the terminal window chrome follows the overall theme.
Quiet Work UI surfaces such as Workspace Overview and Release Notes avoid
status-board layouts, bespoke fixed overlays, and display-font body copy:
Workspace Overview uses a List + Detail work surface, and Release Notes uses the
shared app-global window chrome. These guardrails are covered by SPEC-2356 and
the frontend UI contract tests.
`prefers-reduced-motion: reduce`
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
| `⌘\` | Collapse / expand the Sidebar Layers |
| `Esc` | Close any open palette / overlay / drawer / dropdown |

### Accessibility

Every modal dialog (Command Palette, Hotkey Overlay, branch cleanup,
worktree migration, launch wizard, Add Window) follows the WAI-ARIA
dialog convention: `role="dialog"` with an accessible name, `aria-modal`,
focus moves into the dialog on open and returns to the trigger on close,
Tab cycles within the dialog (no keyboard trap escape), and Escape
dismisses. Async loading stages signal `aria-busy="true"` so screen
readers track progress. Error regions use `role="alert"` for immediate
announcement. WCAG 2.1 AA contrast is asserted across every text /
surface combination in both themes.

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
node scripts/test_release_assets.cjs
```

### Frontend Bundle Contract

```bash
bash scripts/check-frontend-bundle.sh
```

### Release Flow Checks

```bash
bash scripts/check-release-flow.sh
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
└── scripts/            # Release, verification, and maintenance scripts
```

## Specs

Detailed requirements live in GitHub Issues labeled `gwt-spec`. Use
`gwtd issue spec <n>` to inspect them locally through the cache-backed CLI.

## License

MIT
