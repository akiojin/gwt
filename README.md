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
  `Antigravity CLI`, `Gemini CLI (legacy)`, `OpenCode`, `Copilot`, and custom
  agents from a shared canvas.
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

- GUI-first installer:
  - Apple Silicon: `gwt-macos-arm64.dmg`
  - Intel Mac: `gwt-macos-x86_64.dmg`
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
- Agent CLIs installed in `PATH` when you launch them from gwt. Antigravity CLI
  is provided by Google's native `agy` command:

  ```bash
  curl -fsSL https://antigravity.google/cli/install.sh | bash
  ```

  Gemini CLI remains available in gwt as a legacy option for eligible
  Standard/Enterprise or API-key workflows.
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
- **Copy URL** — copies the running tray process URL to the OS clipboard.
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
through `gwt open`. The complete URL is a process-local capability: keep it
private and copy the whole value, including its fragment. It rotates whenever
gwt restarts. The frontend exchanges the fragment for an HttpOnly,
SameSite-strict session cookie and immediately removes the fragment from
visible browser history.

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
server starts. The handoff file contains the same private capability and is
written by a same-directory crash-consistent replacement. gwt enforces mode
`0600` on Unix; on Windows, place it in a directory whose ACL is private to the
current user. Do not publish it as a CI artifact.

Trust boundary: **authenticated LAN only** (including VPN-extended LAN). Every
WebSocket requires the process-local browser session cookie (or the narrower
managed-pane bearer), and missing or foreign origins fail closed. The server
still does not provide TLS termination or rate limiting, so the capability and
session traffic are not safe on an untrusted network. The `--bind` flag is
opt-in; the default `127.0.0.1` remains the safest choice. For access from
another machine, use a trusted VPN (Tailscale, WireGuard, etc.) and the complete
capability URL rather than exposing the port to the public Internet.

Platform note: on Linux, `tao 0.35` still requires a display server (X11 or
Wayland) at EventLoop creation. macOS and Windows browser-server launches
need no additional display setup; Linux operators in pure-headless
environments (no DISPLAY) should use `Xvfb`/`xvfb-run` or wait for the
tao-detach follow-up tracked under SPEC-1942.

Every HTTP / WebSocket request is mirrored to `tracing::info!(target =
"gwt_access", ...)` so the operator can see *which* peer is connecting in
real time on stderr and in `~/.gwt/logs/<date>/`. `/healthz` is demoted to
`debug!` to avoid drowning the stream with health probes. URL fragments,
cookies, authorization values, and recovery handles are not written to the
access log.

Lifecycle: the running `gwt` process owns the agent / PTY lifetime. Closing a
browser tab does **not** stop running agents — only `Ctrl-C` / `SIGTERM` asks
the server to drain PTYs and exit gracefully. The tray-resident process is one
per OS-login user; a second `gwt` invocation prints the existing URL and exits
instead of starting a second server.

gwtd operations run through stdin JSON envelopes without opening a GUI window:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.section","params":{"number":1784,"section":"plan"}}
JSON

gwtd <<'JSON'
{"schema_version":1,"operation":"pr.current","params":{}}
JSON

gwtd <<'JSON'
{"schema_version":1,"operation":"board.show","params":{}}
JSON

gwtd <<'JSON'
{"schema_version":1,"operation":"daemon.status","params":{}}
JSON
```

Managed hooks and runtime delegation use `gwtd`. On macOS and Linux,
running JSON operation `daemon.start` brings up a per-project runtime daemon
(Unix-domain socket IPC) that multi-instance event fan-out depends on
— for example, with the daemon running, Board posts you make in one
`gwt` window appear in another instance opened on the same repo
without a polling delay. The daemon keeps running in the background
until you stop it (Ctrl-C or SIGTERM). JSON operation `daemon.status` prints
the live endpoint for diagnostics. Without JSON operation `daemon.start`,
multi-instance fan-out is inactive but local file-based state and
the file watcher continue to work as before.

Windows currently has no long-running daemon: JSON operation `daemon.start`
exits with "not yet implemented", and managed hooks fall back to
synchronous `gwt hook ...` dispatch. Multi-instance fan-out is
therefore unavailable on Windows pending follow-up work; JSON operation
`daemon.status` still works there but always reports `stopped` until
the named-pipe path lands.

## Agent Workflow

1. Open a project directory, clone from GitHub, or restore the previous
   project.
2. Use `Board`, `Issue`, and Knowledge search surfaces to understand
   the current work, related owners, and prior decisions.
3. In the **Curate** lane, choose `Intake` from the Command Rail or Command
   Palette to shape new work: a branchless, throwaway session that discusses,
   plans, and registers a GitHub Issue. Work that needs design gets the
   `gwt-spec` design-required label and SPEC artifacts on that Issue. Intake
   never creates a branch.
4. In the **Execute** lane, run the registered work: `Open Workspace` launches
   an `Agent` on an existing branch, the background `Issue Monitor` picks up
   registered Issues automatically, or launch directly from an Issue detail
   with the unified prompt form `gwt-execute #N` when the owner is already known.
5. Let gwt materialize the backing `work/YYYYMMDD-HHMM[-n]` branch/worktree
   only when an Execute launch is confirmed (Intake sessions stay branchless and
   ephemeral).
6. Use the shared Board for status, claims, next steps, blockers, handoffs,
   and decisions while agents run. To mirror those Board posts into Slack or
   Teams, configure a remote Board provider first; see
   [Board providers](#board-providers-local--slack--teams).
7. Open `Branches` only when you need Git inspection, filtering, cleanup, or
   lower-level branch/worktree details.

Common windows include:

- `Agent` — live coding-agent process windows created through Intake, Open
  Workspace, the Issue Monitor, or Launch Agent
- `Board` — shared user/agent timeline for reasoning and coordination
- `Issue` — cache-backed Work Item Knowledge Bridge with semantic search, detail
  panes, design-required tags, and Launch Agent handoff. Legacy `SPEC` windows
  open this same Work Item view.
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
requests. The Work Item Knowledge Bridge uses the local Issue cache and semantic
index rather than rendering direct GitHub API responses in the frontend.

On Windows Host launches, Launch Agent lets you choose Command Prompt, Windows
PowerShell, or PowerShell 7. Docker launches continue to use the container
shell.

In terminal windows, drag to select text and release the mouse button to copy.
On Windows, `Ctrl+C` copies the current terminal selection and clears it; if no
selection exists, `Ctrl+C` stays mapped to the running terminal process. On
Linux, `Ctrl+Shift+C` also copies the current terminal selection.

### Session recovery

gwt creates a project-scoped recovery record before a recoverable Intake or
Execution agent starts. After an app or machine crash, a verified provider
conversation resumes automatically in its original worktree. If the provider
definitively reports that the exact conversation no longer exists, gwt starts
a new provider conversation with the latest bounded semantic checkpoint; it
does not silently start with a blank prompt. Retryable authentication,
transport, or protocol failures remain visible for operator attention.

`Recovery Center` in the Command Palette opens all current recovery candidates.
At startup it opens automatically only when a candidate needs a decision, so
successful exact restores do not interrupt normal work. Each candidate shows
its Intake/Execution kind, provider, worktree, checkpoint coverage, capture
health, Board delivery state, and exact-resume evidence. Available actions are:

- `Focus` — focus the already-running replacement
- `Confirm & Resume` — confirm an authoritative exact provider conversation
- `Continue Checkpoint` — start a new conversation with the durable checkpoint
- `Start Fresh` — explicitly start again with the original initial request;
  it is unavailable when that request did not survive legacy migration
- `Open Board` / `Details` — inspect the public milestone or recovery evidence
- `Discard` — remove gwt recovery content after a second confirmation

On the first startup after upgrading, gwt also inventories recoverable legacy
Session files and paused window projections. It imports each distinct session
as a candidate without replaying a saved command or guessing a provider root.
If the legacy Session omitted its provider root, gwt reads only bounded native
provider metadata and matches root kind, canonical worktree, and launch time;
it does not import provider messages. One uniquely strong match may resume
exactly. When several provider conversations could own the same Intake,
Recovery Center shows their recorded evidence and requires an explicit
selection. If neither exact evidence nor an earlier semantic checkpoint
survived, the candidate remains in Attention instead of reconstructing private
discussion content from argv or Board.

If an unresolved ephemeral Intake worktree itself is missing, gwt recreates
only its recorded `.intake` / `.intake-N` path at the immutable pinned base
commit before recovery. It never recreates an Execution or arbitrary user path,
and a missing or mismatched pin leaves the candidate in Attention.

Intake discussion checkpoints publish concise, idempotent milestones to Board.
They include confirmed decisions, open questions, affected SPECs, and the next
action, but never the private transcript, tool output, hidden reasoning, or
credentials. Existing legacy transcripts are not backfilled automatically;
only a current, user-confirmed summary may be published. Copied recovery
attachments are content-addressed. Recovery Store accepts up to 32 MiB per
attachment; managed Codex capture uses a stricter 24 MiB safety bound.
Unresolved recovery content has no time-based expiry. `Discard` or a completed
recovery removes the checkpoint and copied attachments immediately, keeps only
a minimal tombstone for 30 days, and leaves the provider's own conversation
history untouched.

In a current managed Intake, each structured `discussion.update` is also the
semantic checkpoint and deterministic Board-outbox boundary. Repeating the
same milestone creates neither a new checkpoint revision nor a duplicate Board
entry. The memo and Recovery checkpoint keep the same deterministic operation
marker, so Stop rejects a crash/concurrency mismatch until a retry converges.
`intake.checkpoint.current/update` is needed only to supplement that
milestone with allowlisted completed visible items or retained/new attachments.
For a pre-upgrade current Intake with no `GWT_RECOVERY_ID`, gwt resolves the
exact `GWT_SESSION_ID` ledger and performs a bounded metadata-only import on
demand. Missing or ambiguous roots remain in Recovery Center Attention; gwt
does not infer them from private conversation or Board content. Docker Claude
attachment capture is a known limitation: its structured discussion fields
remain durable, but attachment fallback is unavailable until an allowlisted
copy is captured by the managed runtime.

The Board's `Intake` filter shows these milestones by their recorded Intake
origin or explicit `intake` topic; titles are never guessed. Local Board replay
uses the original milestone ID and converges to one entry after a crash. Slack
and Teams do not currently preserve a caller-supplied idempotency ID, so gwt
keeps remote milestone delivery pending instead of risking a duplicate or a
false acknowledgement. The recovery checkpoint still commits, and Recovery
Center displays the bounded delivery error until a safe delivery path is
available.

Codex Host recovery uses a dedicated loopback-only bridge that is separate
from the browser server. Docker and Podman use the mounted `gwtd` sidecar and
the selected Compose service's own loopback; no published port, host gateway,
or token-bearing URL is required. Container recovery therefore requires the
normal gwt Linux bundle mount plus a Codex runner available in that service.
Runner, bridge, or control-channel mismatches fail closed into Recovery Center
instead of falling back to an unobserved direct Codex launch.

## Issue Monitor

The Issue Monitor watches the project's open GitHub Issues and turns them into
agent work. In the default (human-gated) mode it scans candidates into an
inbox, and you press `Launch` per issue: gwt then creates the
`work/issue-N` branch/worktree at launch time and starts the agent with
`gwt-execute #N`. Failed launches stay visible in the inbox with the error, and
`Launch now` retries explicitly.

### Autonomous mode (opt-in)

Autonomous mode runs the whole loop unattended: eligible issue → auto-launch →
implementation → independent review → strong automated gate → auto-merge. It
is **off by default** and requires a **two-stage opt-in**:

1. Enable the `Autonomous` toggle in the Issue Monitor toolbar (per project).
2. Label each issue you want handled autonomously with `auto-merge`.

An issue additionally qualifies only when it has machine-checkable acceptance
criteria (an `## Acceptance Criteria` checklist in the body), the base
branch's protection rules are verifiable, and its bounded attempt budget is
not exhausted. Anything else stays on the human-gated path unchanged.

Safety model in one line: the merge decision never belongs to the
implementing agent — an independent review plus a strong automated gate must
pass first, failures escalate to a visible `NeedsHuman` state, and the
`Autonomous` toggle is a kill switch that actively cancels any auto-merge the
monitor armed. The full gate design and threat model live in SPEC
[#3200](https://github.com/akiojin/gwt/issues/3200).

Unattended lifecycle events (merge completed, retry scheduled, gate passed,
needs-human escalations) surface as toasts and accumulate in a persistent,
scrollable notification stack so nothing is lost while you are away.

Tunable bounds (attempt cap, stuck/idle timeout, retry backoff, review model)
persist per project. The human-gated baseline is SPEC
[#3165](https://github.com/akiojin/gwt/issues/3165).

## Knowledge, Search, and Managed Skills

gwt keeps project knowledge close to the agent workspace:

- JSON operation `issue.spec.read` reads GitHub Issue-backed SPECs from the local cache.
- JSON operations `issue.view` and `issue.comments` provide cache-backed Issue
  access through the gwt CLI surface.
- `gwt-search` searches SPECs, Issues, source files, and docs through the shared
  ChromaDB runtime. Missing indexes are built on demand, and the desktop app can
  repair the managed Python search runtime when needed.
- The Work Item Knowledge Bridge combines cache-backed list/detail views for
  plain and `gwt-spec` tagged Issues with semantic ranking, exact-match
  priority, and match percentages.

Bundled workflow skills are materialized into `.claude/skills`,
`.claude/commands`, and `.codex/skills` for the active worktree. The public
entrypoints are:

- `gwt-discussion` — investigation-first discussion and design clarification
- `gwt-register-issue` — work intake; creates plain Issues or design-required
  `gwt-spec` Issues
- `gwt-plan-spec` — implementation planning for an approved SPEC
- `gwt-execute` — TDD-oriented implementation from `#N` or an approved task
- `gwt-build-spec` / `gwt-fix-issue` — one-release transition aliases to
  `gwt-execute`
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

Quick setup route: choose **Slack** or **Teams**, save the provider's
**Default channel**, sign in, then make sure the bot or signed-in user can access
that channel. The default channel is the primary Board association; posts that do
not have a more specific Workspace mapping go there.

### Associate Workspaces with Slack/Teams channels

Remote providers resolve each Board post's channel in this order:

1. A `channel_map` entry for the post's first Workspace audience.
2. The provider's `default_channel`.

Posts without a Workspace audience use `default_channel` and are placed under a
General thread. For each Workspace/channel pair, gwt creates one remote root
message and stores the root id in `.gwt/work/board-remote-roots.jsonl`; keep that
file, and the matching `.gitattributes` `merge=union` rule, in git so other
machines and agents reuse the same threads.

The Settings UI edits the default channel. For Workspace-specific routing, edit
`~/.gwt/config.toml`:

```toml
[board.slack]
channel_map = { "workspace-id" = "C0123456789" }

[board.teams]
channel_map = { "workspace-id" = "team_id/channel_id" }
```

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

#### 3. Copy the channel link

In Teams, open the channel -> **Get link to channel** and copy the link. gwt
parses `groupId=<GUID>` and the URL-decoded `19:...@thread.tacv2` segment after
`/channel/` when you save the form. If the Teams link is unavailable, use Graph
Explorer (`GET /me/joinedTeams`, then `GET /teams/{id}/channels`) and set
`[board.teams].default_channel = "team_id/channel_id"` in `config.toml`.

#### 4. Configure gwt and sign in

**Settings → Board provider → Teams** → enter **Application (client) ID** and
**Tenant ID**, paste the Teams link into **Teams channel link**, then
**Save** → **Sign in**. gwt stores the channel internally as the existing
`team_id/channel_id` format. Posts appear as the signed-in user (Graph
delegated; app-only channel posting is not supported). You must be a
**member** of the target team and channel — otherwise Graph returns `403` and
gwt shows an actionable hint.

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
labels. Every chrome surface — Project Bar, Command Rail, Status Strip,
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
| `Esc` | Close any open palette / overlay / drawer / dropdown |

The Command Rail on the left edge is always visible: Intake (Curate lane) and
Open Workspace (Execute lane) at the top, window operations (Tile / Stack /
Align / window list / Add) in the middle, and the Command Palette at the bottom. Board and Logs are not rail
items; reach them via the Add Window preset menu, the command palette, or the
`⌘B` / `⌘L` hotkeys. Hovering a rail item reveals its label and real shortcut. Closing a
window (titlebar × or tab ×) always asks for confirmation so a stray click
can never kill a running agent.

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
- Execute any Issue-backed Work Item with `gwt-execute #N`; design-required
  Issues must have `plan` and `tasks` before implementation.
- Local cache path:
  `~/.gwt/cache/issues/<repo-hash>/`
- Managed agent integration files:
  `.claude/settings.local.json` and `.codex/hooks.json`
- List available SPECs:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.list","params":{}}
JSON
```

- Read a SPEC:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.read","params":{"number":1784}}
JSON
```

- Read one section:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.section","params":{"number":1784,"section":"spec"}}
JSON
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

### Releasing

To cut a release, trigger the **Prepare Release** workflow from GitHub
Actions (Actions → `Prepare Release` → `Run workflow`). It runs on `develop`
and bumps the version, regenerates the `CHANGELOG`, and opens a
`develop → main` Release PR — so you can release from any branch without
switching to `develop` locally. The `bump` input is `auto` (default),
`patch`, `minor`, or `major`. Review and merge the generated Release PR;
merging to `main` then runs the release pipeline (tag, GitHub Release,
cross‑platform binaries). The manual fallback procedure lives in
`.claude/commands/release.md`.

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
JSON operation `issue.spec.read` to inspect them locally through the cache-backed CLI.

## License

MIT
