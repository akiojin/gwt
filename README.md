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
  `Grok Build`, `Antigravity CLI`, `OpenCode`, `Copilot`,
  and custom agents from a shared canvas.
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

### Upgrade floor

The upgrade floor for retiring one-shot migrations is **v9.72.1**, the latest
release on 2026-08-03 UTC (60 days before the 2026-10-02 change). For an older
installation, first install [v9.106.0](https://github.com/akiojin/gwt/releases/tag/v9.106.0),
open your projects to run their retained migrations, then install the new version.
Back up your gwt configuration and project state before upgrading.

Legacy Claude Code backend rows are an exception: no released startup path ran
their automatic migration. **旧 backend 設定は自動移行されません。Settings で provider を再登録してください**
(Old backend settings are not migrated automatically; re-register the provider in
Settings → Agent Backends.) Copy the endpoint, API key and model from the old
entry, then select the built-in Claude Code agent with the registered backend.
The old configuration remains readable and is not rewritten or deleted on launch.

Migrations introduced after this floor remain supported, including Session schema
5, PM scratch relocation, work-item projection rebuild v2 and ProjectKey migration.
The usage `window_minutes` contract and the Workspace projection backfill associated
with open SPEC #2359 are also retained. Old HOME / Workspace imports from
`workspace/current.json`, `work_items.json` and `journal.jsonl` are retired. If their
canonical replacements are missing, gwt refuses to load or publish Workspace state
and shows the legacy path with upgrade guidance. The original files stay untouched.
Use v9.106.0 to migrate each project before upgrading; creating empty replacement
files is not a migration. Current receipt and event recovery remains supported.
The coordination event import and discussion import also remain supported: they
serve the current recovery and session-specific Stop contracts. The obsolete agent
identity reset is retired; startup preserves saved purpose and focus values and
leaves `agent_identity.migration.json` unchanged (or absent).

The embedded frontend requires an operation ID for cleanup requests. Reload older
open tabs after upgrading. Launch Wizard always skips permission prompts and
launches with Fast mode off; it reads older saved choices without rewriting them.
Issue Monitor profiles and direct Session resume keep their existing preferences.

## Requirements

- `git` available in `PATH`
- `gh auth login` completed for GitHub-backed features
- Agent CLIs installed in `PATH` when you launch them from gwt. Antigravity CLI
  is provided by Google's native `agy` command:

  ```bash
  curl -fsSL https://antigravity.google/cli/install.sh | bash
  ```

  Gemini CLI is no longer a built-in agent. Legacy Gemini settings and saved
  sessions are ignored with a warning naming the unavailable entry; the files
  are left unchanged. User-defined external commands remain supported through
  custom agents.

  Grok Build is provided by xAI's official `grok` command. Install it with
  `npm install -g @xai-official/grok`, then authenticate on first launch or set
  `XAI_API_KEY` for API-key workflows.
- AI provider credentials when you use agents:
  - `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` or `GEMINI_API_KEY`
  - `XAI_API_KEY`
- Python 3.10+ when gwt needs to bootstrap or repair the shared project index runtime

Linux desktop builds also require WebKitGTK-related system packages. See
[docs/docker-usage.md](docs/docker-usage.md) for the dependency set used in CI.

### Supported built-in agents

gwt supports the following built-in agents. Launch Agent lists only installed
built-in agents that gwt detects; other CLI commands remain available through
custom agents.

**Settings > Supported Agents** shows the complete catalog, installation status,
and installed versions. Install missing CLIs there; update actions appear when
npm metadata confirms a newer version. Agents without supported version metadata
show that limitation and require manual updates.

Automatic agent updates are off by default. When enabled, gwt checks installed
npm agents at the next app startup and updates only known newer versions, with
no live agent panes (including the Project Manager) or pending launches. Manual
installs and updates also require those agents to be closed. Missing agents are
never installed automatically.

| Agent | CLI command |
| --- | --- |
| Claude Code | `claude` |
| Codex | `codex` |
| Grok Build | `grok` |
| Antigravity CLI | `agy` |
| OpenCode | `opencode` |
| OpenClaw | `openclaw` |
| Hermes Agent | `hermes` |
| GitHub Copilot | `gh copilot` |

## Usage

Launching `gwt` installs a system-tray icon (macOS menubar / Windows
notification area / Linux StatusNotifierItem-capable DE). Drive it from
the tray menu:

- **Open in browser** — launches the OS default browser at
  `http://127.0.0.1:<port>/`. The same URL can be opened in any other
  browser too.
- **Copy URL** — copies the running tray process URL to the OS clipboard.
- **Projects** — opens a project URL from the open projects followed by Recent,
  with running and error counts. The tray icon shows an error badge while an
  open project has an agent error.
- **About GWT** — opens the browser About / Version surface for the
  running tray process.
- **Quit** — gracefully shuts the tray icon, embedded server, and
  PTY children down in order.

Project browser tabs show agent RUN / BLOCK counts in their titles and a
status favicon. BLOCK includes waiting, stopped, and error states; shell
windows are excluded. An unread marker clears when the project tab is visible
and focused. Hub metadata stays fixed.

The root URL `http://127.0.0.1:<port>/` is the **Hub**: Open Folder, Clone
from GitHub, Recent projects, and the currently open projects. Every project
has its own URL, `http://127.0.0.1:<port>/p/<project-hash>`, and project links
open in a new browser tab, so one browser tab shows one project. Different
projects keep their windows and launch dialogs separate; two browser tabs on
the same project URL share its live workspace. Bookmarking or restoring a
project URL reopens a recent project automatically; an unknown project URL
shows "Project not found" with a link back to the Hub. The **Hub** link in a
project's header opens the Hub in a new tab.

Autostart lives in **Settings > System > Launch GWT at login**. Enabling it
installs an OS-native per-user entry (macOS LaunchAgent / Windows HKCU Run /
Linux XDG autostart) via the `auto-launch` crate, so `gwt` resumes at the next
OS login as a tray-resident process. The browser is not opened automatically.

```bash
gwt                                 # install tray + start embedded server (loopback)
gwt --bind 0.0.0.0 --port 60745     # bind the embedded server to a LAN/VPN-reachable address
gwt open                            # open the running tray's Hub URL in the OS default browser
gwt open ~/src/my-repo              # open that project (opening it first if needed) at its /p/<hash> URL
```

`--bind <ip>` defaults to `127.0.0.1`. When `--port` is omitted and no port has
been saved yet, gwt binds an available port, saves the actual port, and reuses
it on later launches. If that saved port is already in use, gwt selects another
port, updates the saved value, and emits a warning. An explicit `--port <n>`
applies only to that launch—including `--port 0` for an ephemeral port—and
never changes the saved implicit port. Pass `--bind 0.0.0.0` to make the embedded UI reachable
from other hosts on the same LAN or VPN-extended LAN; pair it with an explicit
`--port` when you need an operator-selected, well-known port. `--no-tray`
starts a temporary server without registering a tray icon. It exits when its
launching parent ends, or five seconds after its last browser session closes
(reloads can reconnect during that grace). Before any browser connects, the
server follows its parent's lifetime. `--no-open` explicitly suppresses browser
auto-open; startup already suppresses it by default.

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

If `workspace.update` loses its response, use the reported `operation_id` with
`workspace.receipt` (`params: {"operation_id":"<UUID>"}`) in the same Session.
This read-only query does not contact the Host or resend the update. `applied`
confirms durable publication; `unconfirmed` means proof is not yet available,
including with an older Host, and does not prove the update failed.

`board.show` returns the latest 20 entries visible to the selected workspace or
session, in chronological order. Set `params.limit` to a nonnegative integer
(for example, `15`; `0` returns no entries). `params.all: true` selects all
audiences and removes the default cap, but an explicit `limit` always wins.
Provider retention still applies: `all` does not load the full historical archive.
Unknown parameter keys are rejected with the accepted keys listed.
The existing `board` field is preserved; `page.total_entries` counts the visible
provider snapshot before the CLI limit, `page.returned_entries` counts returned
entries, and `page.truncated` indicates clipping by that limit.
Every `blocked` entry carries an `escalation` object: `resolved` (boolean),
`resolved_at`, and `resolved_by_entry_id`. A blocked entry missing from the
escalation index reports `indexed: false` and `resolved: null`. Set
`params.unresolved: true` (default `false`) to return only blocked entries whose
escalation is still open; the filter applies before `limit`.

Response cost is roughly the entry count times serialized entry size, plus
metadata: 20 entries averaging 2 KiB are about 40 KiB. There is no fixed byte cap;
long posts increase the size, and `all: true` can return hundreds of KiB or more.

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

On Windows the daemon runs the same way: the GUI's Issue Monitor starts
and supervises it as a user-session child process, and JSON operation
`daemon.start` starts one by hand. The transport is a named pipe
(`\\.\pipe\gwtd-<scope>-<hash>`, local clients only; the endpoint file
under `~/.gwt` carries the auth token). `daemon.status`,
`daemon.subscribe`, Issue Monitor controls, and multi-instance fan-out
behave as on macOS / Linux. A hand-started daemon stops on Ctrl-C,
Ctrl-Break, or console close; logoff and shutdown run the same cleanup,
and a daemon terminated by the GUI is reclaimed by the liveness checks on
the next start. gwt does not install a Windows Service: the daemon only
scans and claims — agent panes are still created by the GUI — so a
service would not enable headless autonomous runs and would fight the
per-user `~/.gwt` state. Headless autonomous execution is not a goal of
the daemon.

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
  open this same Work Item view. Issue Monitor launches do not open a window on
  the canvas: the agent is mirrored read-only in the Issue window's right pane,
  and `Windowize` promotes it to a normal window when you want to type into it.
  Each row also carries its Work's lifecycle, attention reason, and PR state,
  with `Continue work` / `Resume` / `Clean Up` available in place.
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

## Issue surface and Issue Monitor

Open `Issue` from Add Window to browse cached GitHub Issues in four columns:
Backlog, Queued, Active, and Done. Each row preserves its execution state and
actions. Drag Backlog items into Queued to schedule them, drag them back to remove
them, or reorder items within Queued. Selecting multiple items sends one queue
change. Active and Done follow the execution lifecycle and cannot be changed by
dragging. Queued items show who added them, including `auto-refill`.

Search and the Kanban / Split switch share one row. Monitor status and controls
are separate, with visible labels for Settings, Autonomous, Auto-refill, its
limit, and Start monitor / Stop. Auto-refill is **off by default**; enabling it
opts into adding eligible open Issues up to the configured queue limit. An empty
queue starts no new work; already-running work continues. Monitor errors appear
in the notification center. The detail pane shows acceptance-criteria progress
and state-specific actions. Select a card and use **Issue / Output** to switch
between its body and acceptance criteria and its agent's read-only output.
**Windowize** moves the agent to Canvas. **Hide preview / Show preview** gives
the board the full width or restores the detail pane; columns scroll horizontally
instead of shrinking. The legacy `issue_monitor` preset opens this same Issue surface.

**Max active** uses **Auto** for new settings. Its recommendation reflects CPU,
free memory and disk space, the GUI's CPU use, and live agents in other projects.
Registered projects without live agents consume no share. **Machine budget**
shows the limiting resource and distinguishes the Monitor's implementation/review
limit from the total including PM agents. Auto pauses new admissions while required
measurements are unavailable; running agents continue.
Initial measurements of large `target` directories can take several minutes;
the same pause applies when a previous measurement expires during refresh.
Enter a positive number to keep a **Manual** override, or select **Use Auto** to
follow the recommendation again. Existing saved limits remain Manual. Values above
the recommendation are allowed, with a warning that verification may not finish and
timing-dependent test failures may block unrelated PRs. Automation uses
`issue.monitor.config.set` with `{"max_active_mode":"auto"}` for Auto or
`{"max_active":4}` for a Manual limit of four. `issue.monitor.status` reports the effective limit,
`max_active_agents_override`, and the shared `agent_capacity` measurement.

**Allowed labels** controls which Issues this terminal's Monitor admits. Add or
remove one label at a time; an Issue needs any label in the saved list. Matching
ignores case and surrounding whitespace. An empty list allows all labels and
preserves the existing admission rules. Changes apply on the next scan without
cancelling running agents. The control shows the saved labels and excluded Issue
count/numbers. Automation can set the same list with `issue.monitor.config.set`
and `{"allowed_labels":["agent:mac"]}`; `issue.monitor.status` reports
`allowed_labels`, `label_excluded_count`, and `label_excluded_issues`.

Open GitHub Issues remain in Backlog until explicitly queued, added by enabled
auto-refill, or admitted with an `urgent` label. Queue membership authorizes the monitor to consider an Issue; normal
readiness, claim, and capacity checks still apply. `Launch now` on a row opens the
launch flow, which creates the `work/issue-N` branch/worktree at launch time and
starts the agent with `gwt-execute #N`. Failed launches remain visible on their
Issue rows.

Anyone can apply `urgent`. Eligible urgent Issues enter the queue automatically,
even with Auto-refill off; an explicit queue removal still wins. Up to two urgent
Issues lead the queue in assignment order by default, without changing the saved
normal order or `max_active`. Set the head limit with
`issue.monitor.queue.urgent_limit` (`limit: 0` disables priority, not membership).
Overflow follows normal order. `issue.monitor.queue.demote` (`number`) persistently
returns an Issue to normal priority, overriding its urgent label across scans and
restarts. Queue cards and details distinguish urgent, overflow, and demotion;
`issue.monitor.queue.list` and `issue.monitor.status` include the reason and
observed GitHub label actor/time. Missing audit data is shown as unknown.

Agents and automation can inspect the queue with `issue.monitor.status` and
change membership/order with `issue.monitor.queue.push`,
`issue.monitor.queue.remove`, and `issue.monitor.queue.move`. The
`issue.monitor.queue.auto_refill` operation sets opt-in refill and its limit.
`issue.monitor.launch_now` explicitly adds the Issue at the front of the terminal
queue and requests a scan. Existing `issue.monitor.priority.move` and
`issue.monitor.priority.set` operations remain available. The
`issue.monitor.config.set` operation can stop processing, disable autonomous
mode, or set a positive `max_active` limit. For safety, it rejects
`enabled=true` and `autonomous_mode=true`; enabling either capability requires
an explicit action in the GUI. Idle agent windows free their slot on
their own: each scan classifies every launched window as
`review_verdict_published`, `execution_settled`, `binding_dead`, or
`stuck_unknown` (visible per row and in `idle_windows` in
`issue.monitor.status`), releases the first three and closes their panes. A
released Issue stays out of the queue, except when its window died before the
agent settled its execution — an app restart that took the pane with it, for
example — in which case the Issue is requeued so the next scan relaunches it
on its existing branch. Only `stuck_unknown` — a window that is idle while its
execution record is still active — stays for a human, and it asks for a
decision once it has been idle for twice the stuck timeout.
`issue.monitor.release_idle` runs the same release by hand for one Issue or
every idle row, and `dry_run: true` reports the targets without touching
anything. `issue.monitor.profiles` reads the launch
candidate pool and `issue.monitor.profiles.set` replaces it; with two or more
candidates the Monitor launches each Issue with the first eligible candidate
(rate-limit holds and `prefer_for` routing decide eligibility; a provider
leaves the pool when it refuses a launch, not when a usage reading predicts it
will; the exact rules are specified in SPEC
[#3914](https://github.com/akiojin/gwt/issues/3914)), so one rate-limited
provider no longer stops the queue. `issue.monitor.status` reports each
provider's latest usage reading under `provider_usage`, or why there is none.
Every rate-limit refusal immediately holds its provider. If all candidates
are held, the queue resumes at the earliest known reset; if every reset is
unknown, `needs_human_fleet` reports `launch_candidates_exhausted` instead
of periodically retrying. In the GUI, the Issue Monitor settings form
(`⚙ Settings`) lists the same pool as Agent Settings sets: `＋` adds a set, `−`
removes one, the arrows reorder them, and the saved order is the launch order.
All operations accept an optional
`project_root` and otherwise target the current worktree. Priority and
daemon-absent configuration changes become visible to running instances on the
next scan/rebase.

Automatic profile selection is opt-in with `issue.monitor.tiers.set` and
`{"auto":true}`. It supplies three tiers without requiring a profile pool:
Codex Luna / Claude Haiku, Codex Sol / Claude Sonnet, then Codex Astra / Claude
Opus. Tier indices start at 0. The selected tier is the maximum of the agent
failure count, the Issue's retained floor, and 1 for a `gwt-spec` Issue, capped
at the last tier. Retry admission and terminal handling retain their existing
rules. Eligible providers are selected within each tier using the existing
pool rules; if none is eligible, the next tier is tried. Normal automatic
launches start a fresh session to apply the current model and effort. Answered
handoffs still return to their original session without changing tier history.
Infrastructure failures and terminations without exit evidence do not raise
the tier. They still count toward the existing retry budget and backoff.

`issue.monitor.tiers.set` also accepts `tiers`, an ordered array of profile
arrays. Omitting `tiers` restores the defaults. `{"auto":false}` restores the
existing manual pool. `issue.monitor.tier.set` with `number` and `tier` raises
an Issue's persistent minimum tier. `issue.monitor.tiers` reports the
configuration, Issue history and lowest-tier landing rate (`null` until a
landing is observed), plus the count of unclassified terminations.
`issue.monitor.status` exposes `launch_tier`, `landing_tier`, total `attempts`,
excluded `non_agent_attempts`, and their difference `tier_input` on each
observed Issue row.
Landing means a successful Work `done` update from the matching owner and
session; closing or cancelling an Issue alone does not count. It measures
agent-declared completion, so the rate can be optimistic if the PR fails later.
PR merge is not required for this metric.

Host free space is part of the same snapshot: `disk_space` in
`issue.monitor.status` lists the volumes the worktrees and the verification
coordinator live on and carries a `warning` once one of them falls below
20 GiB or 5% free, so a filling host is visible before `verify.run` fails with
`No space left on device`. The `worktree.gc_build_artifacts` operation
reclaims the space: it removes the `target/` build cache of every worktree
whose HEAD is merged into `origin/<base>` (`base` defaults to `develop`) and
that has neither a running process nor a live gwt launch. An unqualified call
is a dry run that lists the candidates with their sizes and every kept
worktree with its reason (`active process …`, `tracked launch …`, `not
merged …`); pass `dry_run: false` to delete, `include_unmerged: true` to
also reclaim idle unmerged worktrees, and `include_protected_workspaces: true`
to also reclaim the shared base-branch workspaces (`develop`, `main`), which
are kept by default because their rebuild lands on whoever opens them next.
Running worktrees, the main worktree, the calling worktree, and the worktree
hosting the running `gwtd` are never touched, whatever the flags say.

Automatic low-disk GC reclaims merged idle caches first. If disk space still
falls below the configured `[build_artifact_gc]` thresholds, it then reclaims
idle unmerged caches, checking disk space before each one. Live processes and
launches remain protected in both stages. A sweep that reclaims zero bytes
reports `build_artifact_gc.outcome: no_reclaim`, a `warning`, and
`kept_by_reason` in `issue.monitor.status`; successful enumeration alone is
not reported as successful reclaim. If every cache is in use, GC cannot
guarantee that the disk will not fill.

The Workspace panel's `Clean Up Ready` count uses the same idea for whole
worktrees: a merged or change-free Workspace stays cleanup-ready when its only
uncommitted difference is something gwt itself wrote — its `.gwt/` namespace,
the materialized `gwt-*` skills and commands, or a `.codex/hooks.json` /
`.claude/settings.local.json` that still carries no hand-written content.
Anything else you have not committed keeps the Workspace out of the count.

### Free provider resets

`provider.reset.proposals` reads provider holds and suggests checking a free
Codex reset when the remaining wait exceeds `min_reset_wait_secs` (default:
86400). Claude holds instead suggest switching providers: paid `/extra-usage`
is never enabled by gwt.

`provider.reset` takes `provider: "codex"` and the exact `window_id` from
`pane.list`. It checks the available free credits, then displays an OS
confirmation dialog. Choose **Redeem free reset** to consume one free reset
for that account; Cancel is the default. Confirmation is mandatory even in
autonomous mode. JSON approval flags and past approvals cannot replace it.
Only canvas windows using a directly installed Host Codex and its default
provider are supported; Docker, package-runner launches and custom backends
are refused. The dialog identifies the authentication root recorded when the
target window launched and its source (host, profile or caller environment).
The helper uses that same root. Relaunch older windows that lack this proof;
unresolved authentication environments are refused. On Windows, set an explicit
`CODEX_HOME` before launching the target window.

After a confirmed reset, gwt rereads account availability and releases the
provider hold automatically. Failure or an unconfirmed outcome retains the
hold. Approval, execution and results are recorded under
`~/.gwt/provider-resets/<request_id>.jsonl`; the operation returns that path
and any failure reason. If only the final audit write fails after a successful
reset and hold release, the result remains successful with an `audit_warning`.
No credits are purchased, and no paid-usage fallback
exists. See `gwtd --help provider` for parameters.

### Autonomous mode (opt-in)

Autonomous mode runs the whole loop unattended: eligible issue → auto-launch →
implementation → independent review → strong automated gate → auto-merge. It
is **off by default** and requires a **two-stage opt-in**:

1. Enable the `Autonomous` toggle in the Issue surface toolbar (per project).
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

Once a work branch merges into `develop`, the monitor settles the delivered
Issue itself (`Closes #N` only fires on the default branch). When every
acceptance criterion is checked — or the PR body / an Issue comment records
that the remaining criteria were delegated to another Issue
(`残 AC は別 Issue に委譲`) — it posts a comment carrying the PR number and
merge SHA and closes the Issue. Unchecked criteria leave the Issue open with a
`merge 済み・未達 AC あり` comment and a `NeedsHuman` state; a `gwt-spec` Issue
is closed only after every task phase is complete. Auto-close follows the
`Autonomous` toggle by default; `issue.monitor.config.set` with
`auto_close_merged_issues=true|false` overrides it, and when it is off the
monitor only records a `merge 済み・close 待ち` comment. An Issue a human
reopened is never closed again by the same merge.

Unattended lifecycle events (merge completed, retry scheduled, gate passed,
needs-human escalations) surface as toasts and accumulate in a persistent,
scrollable notification stack so nothing is lost while you are away.

Agent state notices say **stopped**, **error**, or **needs human**; an idle agent
is not evidence of completed work. Runtime desktop notices require five minutes
of continuously observed Running and an unfocused/hidden page. A different state
or reconnection resets that duration. Monitor NeedsHuman uses its existing notice
stream immediately, including issues without an agent window; it does not create
another notice from inbox snapshots. Session Interrupted is outside this notice
stream because the resume picker exposes historical snapshots only.

Native permission adapters distinguish macOS authorization settings, Windows
notification settings, and Linux's unavailable authorization query. Unknown,
default, denied, or failed queries do not authorize delivery, and permissions are
never requested automatically. Linux GetCapabilities describes server features,
not user consent. These adapters do not yet add zero-tab native delivery: that
transport remains a separate part of SPEC #3287. Debug-binary tests cover policy
and browser behavior; signed-bundle macOS permission/delivery and Windows/Linux
native interaction require separate host verification.

Tunable bounds (attempt cap, stuck/idle timeout, retry backoff, review model)
persist per project. The human-gated baseline is SPEC
[#3165](https://github.com/akiojin/gwt/issues/3165).

## PM agent

Each project also runs one resident **PM agent** pane. It is the single
conversational window: you describe what you want in natural language, and the
PM decomposes it into Issues, registers them, plans the design-required ones,
decides the semantic execution order, and tells the Issue Monitor which Issue
to take next. It reports progress and brings `NeedsHuman` escalations back to
you in the same conversation.

The PM never launches implementation agents itself — it moves an Issue to the
front of the queue and asks for a scan, and the Issue Monitor's existing
claim/slot path does the launching, so the duplicate-launch protections are
unchanged.

- It starts automatically when you open a project, and there is a per-project
  opt-out.
- PM settings offers **Pause / Resume** for its autonomous loop. Pause persists
  across restarts while the PM remains available for conversation; Issue Monitor
  and running agents keep working. Resume reconciles the latest status. JSON
  operations `pm.pause` / `pm.resume` provide the same control, and `pm.status`
  reports `paused` separately from registration.
- Closing the PM pane stops it; it will not restart itself. A crash does
  auto-resume, with a backoff so a crash loop cannot spin.
- Only the PM may turn the Issue Monitor's `enabled` / `autonomous_mode` on
  from the CLI; every other agent session must use the GUI. Merges are
  unaffected — the strong automated gate above still decides every merge.
- There is one PM per **repository**, not per project store, so a repository
  whose state resolved into two stores still gets exactly one. JSON operation
  `pm.status` lists every registration in the repository, and `pm.stop` retires
  one from the CLI — a registered PM can retire an orphan or stand down itself
  without a GUI click.

The design lives in SPEC
[#3431](https://github.com/akiojin/gwt/issues/3431).

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

### Hook file ownership

- gwt regenerates `.claude/settings.local.json` as a local machine file and
  manages its Git exclusion.
- gwt creates or merges `.codex/hooks.json`, but does not add it to `.gitignore`
  or `info/exclude`.
- Whether `.codex/hooks.json` is version-controlled is a repository decision.
  When the file already exists, gwt replaces only gwt-managed hook entries and
  keeps user hooks plus unrelated top-level settings.
- The gwt repository itself ignores `.codex/hooks.json` and generates it locally
  when gwt prepares an agent session. Windows uses a PowerShell EncodedCommand;
  macOS and Linux use a POSIX shell command. Keeping this generated file untracked
  prevents platform-specific changes from dirtying the checkout.
- A version-controlled `.codex/hooks.json` should keep the portable `gwtd`
  fallback so a machine-local absolute path is never committed. Regenerate it
  with
  `GWT_HOOK_BIN=gwtd cargo run -p gwt-skills --example regenerate_hook_settings -- worktree-local`.
- Outside a launch, gwt owns both Codex hook discovery locations — the
  worktree-local `.codex/hooks.json` and the workspace-home copy at the repo
  root — so hook health reporting and self-heal always target the same files.

### Codex recommended config

On every GUI startup gwt makes sure the host Codex config
(`$CODEX_HOME/config.toml`, default `~/.codex/config.toml`) carries
gwt's recommended `features.context_management.experimental_mode = true`, which
keeps accumulated context as notes and searchable history instead of repeated
single-summary compaction. gwt writes the key only when it is absent; every
other table in the file is preserved and a config that already has the key is
never rewritten. To opt out, set it explicitly in `config.toml`:

```toml
[features.context_management]
experimental_mode = false
```

gwt respects any explicit value (`true` or `false`) and does not change it. A
config that cannot be parsed or written never blocks startup; the path and
cause are recorded in the error ledger (`errors.list`).

`errors.list` returns project-scoped errors by default. Set `project_root` to
filter by project. Use `scope: "host"` for machine-wide errors such as startup
configuration failures, `scope: "unknown"` for unattributed records, or
`scope: "all"` to inspect every scope. A `project_root` filter always excludes
host and unknown records; gwt never guesses their project.

Codex CLIs before 0.153.0 cannot load a table under `[features]`: a single
`[features.context_management]` table makes the whole config unreadable
(`invalid type: map, expected a boolean`), which also stops `codex login`. The
codex gwt launches and the `codex` on your `PATH` can be different versions, so
gwt checks the `PATH` one (`codex --version`) at startup. When it is older than
0.153.0, or its version cannot be read, gwt does not write the key and removes
an existing `[features.context_management]` table so that codex keeps working.
After you upgrade the `PATH` codex to 0.153.0 or later, the next gwt startup
writes the key again.

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

## PM project configuration

The resident PM starts in a gwt-owned runtime directory. Repository skills,
hooks, `AGENTS.md`, and `CLAUDE.md` remain readable as project data; they are
not loaded as PM configuration. Implementation agents keep their normal
project configuration. Existing PM conversations are not migrated automatically;
the isolated directory is used on the next PM session launch.

To explicitly supply project policy, add `settings.project_policy_files` to
`~/.gwt/projects/<project-hash>/project-state/pm.json`, preserving its other
fields. For example, `"project_policy_files": ["docs/pm-policy.md"]` selects a
file relative to the PM project checkout. The default is an empty list.
Selected text is copied into the existing generated `gwt-pm` skill during
managed asset refresh; no project symlink is added to the runtime. Removing
an entry removes its copied policy on the next refresh.

## Canvas Operations

- Zoom the canvas with the on-screen zoom buttons
- Pan the canvas by dragging the background
- Use `Tile` to arrange windows on a grid
- Use `Stack` to cascade windows with overlap
- Use `Align` to arrange windows on a grid without changing their size
- Use `Cmd/Ctrl+Shift+Right` and `Cmd/Ctrl+Shift+Left` to cycle Canvas Agent
  windows by activity: running/starting first, waiting/idle next, then the
  remaining Agents. Non-Agent surfaces are skipped, hidden Agent tabs are
  activated when selected, and the focused Agent is recentered

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

- Lint a SPEC artifact before handing it to a reviewer. The run checks FR / AS
  / T numbering, traceability-table consistency, supersede inline annotations,
  and section marker / roundtrip health, records the result in the Intake
  Inspection Snapshot, seeds the Finding Disposition Ledger, and prints the
  reviewer checklist. It exits non-zero when a critical finding is present.
  It also reports missing indexed comment IDs and unindexed artifact comments
  (including section and part numbers), without rewriting or deleting them.
  Section writes serialize writers sharing the host cache and check fresh body
  content before replacing the index; cross-host/external writes and unknown
  network outcomes are outside this guarantee.

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.lint","params":{"number":1784}}
JSON
```

- Check whether the SPEC may be declared complete. Every section needs a
  GitHub-entity readback matching the snapshot, and every critical finding
  needs a disposition:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.inspection.complete","params":{"number":1784}}
JSON
```

## Logs

Open the Logs surface and select **Project** for that project’s events, or
**Global** for startup and diagnostics without a project. Background events
remain with their originating project when you switch projects.

- App logs:
  `~/.gwt/projects/<repo-hash>/logs/gwt.log.YYYY-MM-DD`
- Startup and global diagnostics:
  `~/.gwt/logs/gwt.log.YYYY-MM-DD`
- Session state:
  `~/.gwt/session.json`
- Project workspace state:
  `~/.gwt/projects/<repo-hash>/workspace.json`

### Session history

gwt keeps recent Session history in `~/.gwt/sessions/`. Background cleanup
runs on the first ledger view and at most once every 24 hours, removing
stopped history with startup restore disabled after 30 days of inactivity.
Saved windows and Sessions needed by runtime, recovery, or unfinished work
remain protected. Old abandoned write temporaries are also removed.
See [Issue #5025](https://github.com/akiojin/gwt/issues/5025) for the
retention policy and unreadable-record handling.

### macOS filesystem activity and Spotlight

The per-worktree index watcher excludes the root `target/` directory's
descendants from its own macOS FSEvents stream, including when `target/` is
created after watching starts. A change to the directory entry itself can
still arrive from its parent and is filtered by the index path policy.
This controls only that gwt stream; it does not disable system-wide FSEvents
or other applications' subscriptions. The index watcher currently has no
production startup caller, so this exclusion alone does not establish the
cause of high `fseventsd` CPU usage.

For an existing worktree, open **System Settings → Spotlight → Search Privacy**
and add its `target` directory. For a new worktree, add `target` after the
first build creates it. Follow Apple's
[Spotlight privacy instructions](https://support.apple.com/en-gb/guide/mac-help/mchl1bb43b84/mac)
for your macOS version. gwt does not change Spotlight settings automatically.

To inspect host CPU alongside gwt diagnostics:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"diagnostics.cpu","params":{}}
JSON
```

The `host_cpu` result includes the latest `fseventsd` process sample and the
sampling count and interval. On macOS, it samples three times one second apart
and warns when the same process exceeds 100% CPU in all three samples. Missing
processes or unavailable samples do not imply low CPU usage. A warning is an
observation, not proof that a particular worktree caused the load; inspect
active filesystem consumers and Spotlight privacy settings before attributing it.

Spotlight's own indexing daemon is reported from the Issue Monitor snapshot
instead: `spotlight` in `issue.monitor.status` lists every `mds_stores`
process with its CPU percentage and carries a `warning` once one of them
exceeds 100%. On a host with hundreds of worktrees the daemon can outrank the
agents themselves, which otherwise only reads as a slow host. The block is
present with no processes and no warning on platforms without Spotlight.

## Pending application updates

A downloaded update can remain staged while agents finish their work. The
existing drain evaluates safety on the 15-second terminal convergence tick:
two quiet observations start a 60-second grace period before applying. The
`autonomous_tuning.update_drain_notify_after_secs` setting controls the repeated
waiting notification (default: 1800 seconds); it is not a forced restart deadline.
Agents are never terminated merely because that interval elapsed.

`~/.gwt/logs/update-YYYY-MM-DD.log` records the stage, wait/refusal reason and,
when another automatic evaluation is scheduled, `next_evaluation_at`. Unchanged
wait reasons do not create a log line every tick. The latest per-project
observation is refreshed on each evaluation; its timestamp is an observation,
not a guarantee that a stalled or stopped app will run the next tick.

The `release.status` JSON operation distinguishes `pending_update_version`
(the locally saved manifest version) from `pending_version` (a remote
release branch's unreleased version bump). `update_wait` reports the latest
matching local wait observation. If the payload is missing, `update_stage` is
`payload_missing` and the recovery action asks for a fresh download. `last_apply_result` and `last_apply_failure`
report the latest completed attempt; `attempt` is the resume marker's counter,
not a lifetime retry count. A successful result replaces the previous failure.
A failed install may restart into the old version: restarting alone does not
prove the update was applied. Follow the reported recovery action and check
`observed_version`. Requests made while an apply is already resolving or
committing are coalesced; a failed request permits an explicit retry.

## Development

### Build

```bash
cargo build -p gwt --bin gwt --bin gwtd
```

The `browser-check` skill (isolated GUI verification of this checkout) also
needs `jq` on `PATH` to read `hook.doctor` evidence. It is not required to run
gwt itself.

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
cargo install cargo-nextest --locked --version 0.9.146
cargo nextest run -p gwt-core -p gwt --all-features --test-threads=1
cargo test -p gwt-core -p gwt --all-features --doc
```

Nextest runs each test in a separate process, times out a test after 120 seconds, and continues with the remaining tests. Doctests use rustdoc separately.

### CI throughput measurements

With Python 3.11+, authenticated `gh`, and local Git history for the merged PRs,
collect the latest 25 develop merges and save their input data:

```bash
python scripts/ci_throughput.py --repo akiojin/gwt --limit 25 --save target/ci-throughput.json
python scripts/ci_throughput.py --input target/ci-throughput.json
```

Run collection from this checkout (or specify `--root`). Fetch missing history
before collecting; for a shallow clone, use `git fetch --unshallow origin develop`.
`--before 2026-10-08T00:30:00Z` fixes the inclusive merge cutoff, and
`--workflow lint.yml` measures Lint using the same collection and replay path.
Replay needs neither GitHub access nor Git history. The saved baseline is:

```bash
python scripts/ci_throughput.py --input scripts/fixtures/ci-throughput-2026-10-08.json
```

The JSON reports PR creation-to-merge time, each PR's latest successful final-head
workflow attempt, base synchronizations per merge, runner waits, and per-job
durations. Durations are in minutes; p50 is the median and p90 is nearest rank.
Workflow duration is `run_started_at` to `updated_at`; job duration is `started_at`
to `completed_at`. Runner wait is job `created_at` to `started_at`, after dependency
scheduling. All jobs and required jobs have separate wait distributions. Missing
samples remain unavailable; rerun jobs whose creation time follows their reused
execution time retain their duration but have unavailable runner waits.

The fixed 25-PR baseline reproduces p50 **130.27 minutes** from creation to merge
and **32.43 minutes** per Test attempt. Base synchronizations use merges whose
second parent belongs to the base's first-parent history: **115/25 = 4.6**.
The report also shows the historical `Merge ... develop` subject filter's
**110/25 = 4.4**, which omits five synchronizations with custom subjects.

### Shared frontend state (SPEC-5016)

Migrated frontend domains use `web/ui-state-store.js` to own immutable data. Receive
handlers update the model; views subscribe to selectors and render the committed
snapshot. Retain each unsubscribe function for views that can be removed. Keep
DOM nodes and renderer functions outside the model. Notifications also run for
unfocused windows, without a focus or animation-frame trigger. The shared
`ui-content.js` renderer selects plaintext or backend-sanitized Markdown from
the content type. See [SPEC-5016](https://github.com/akiojin/gwt/issues/5016) for
the migration inventory and acceptance criteria.

### Capacity for heavy verification

Only canonical `verify.run` acquires the host-wide verification lease.
Register the verification matrix with `verify.plan`, then run it with
`verify.run`; it acquires and releases the lease for each Heavy command.
Light commands can overlap other runs and outstanding Light commands run before
Heavy commands. Heavy commands retain their relative order; gwt artifact
restoration runs last. An admission timeout before the first remaining command
starts preserves any predecessor without writing a replacement record. Later
timeouts retain completed results in an incomplete, non-PASS deferred record.

The following short non-Cargo gates are Light and run without a Heavy lease:

| Command | Resource bound |
| --- | --- |
| `git diff --check` (including `--cached`) | Checks whitespace in a diff |
| `node scripts/check-coverage-threshold.mjs <summary> <threshold> ...` | Reads an existing coverage JSON; does not run tests |
| `actionlint` without custom checker options, `shellcheck`, `yamllint` | Static analysis of workflow, shell, or YAML files |
| `taplo check`, `taplo fmt --check` | TOML validation or formatting checks |
| `typos` | Static spelling checks |

These gates have a 60-second execution timeout on both local and daemon hosts.
A timeout records a failure (exit 124), preserves diagnostic output, and stops
the command's process tree. Fix the reported command and rerun the full matrix.
Existing markdownlint and scoped Cargo classification is unchanged. Unknown
commands, script wrappers, the coverage producer `coverage-summary.mjs`, Cargo
builds or broad tests, and headed Playwright remain Heavy.
`actionlint -shellcheck` / `-pyflakes` overrides also remain Heavy because they
can launch arbitrary wrappers. Bounded commands reclaim descendants on normal
completion as well as on timeout.
The Node reader also remains Heavy when its effective `NODE_OPTIONS` is nonempty,
because those options can preload arbitrary modules. An explicit `NODE_OPTIONS=`
disables inherited options and retains Light classification.

Retry with the same full requested matrix and headed E2E nominations. `verify.run`
automatically resumes only a valid admission-deferred record with identical
owner, session, execution authority, plan content hash, source fingerprint and
requested commands. All preceding commands must have passed without a signal.
Other records, including failed, killed and crashed runs, start fresh; a
registered plan mismatch still requires `verify.plan`. Resumed evidence refers
to its immutable predecessor by id/hash and retains its original start time and
per-command headed E2E, nextest and admission evidence. The full matrix and
required headed Chromium results in dark/light must pass before Overall PASS
or Ready.
Heavy Cargo commands in independent worktrees and build directories share a
bounded host pool. Its default capacity is the smallest of one slot per eight
logical CPUs, one per 16 GiB of memory, and four, with a minimum of one. To override
the capacity, set the following in `~/.gwt/config.toml`:

```toml
[verification]
slots = 4
```

The same worktree or effective Cargo target directory stays serialized.
Unknown command wrappers and older binaries retain exclusive admission.
Each child gets its own temporary directory. Admission reserves disk space for
the target and temporary volumes above the configured build-artifact GC floor;
`verification.disk_budget_bytes` overrides the measured per-run byte budget.
The default reservation is 5,904,433,337 bytes (about 5.5 GiB) on each distinct
volume, based on measured target and temporary growth plus 20% headroom.
`verify.lease.status` reports `capacity`, `running`, `available`, each holder in
`slots`, and the shared FIFO queue with holder ETAs. Inspect it before retrying:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"verify.lease.status","params":{}}
JSON
```

Each `verify.run` publishes an attempt before waiting for admission. Inspect
your latest attempt with `verify.status`, or pass `params.attempt_id` to inspect
an exact attempt. Its JSON output includes the attempt ID, lifecycle status,
interruption reason and whether its FIFO reservation remains:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"verify.status","params":{}}
JSON
```

Cancel a superseded attempt using the returned ID and a reason:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"verify.cancel","params":{"attempt_id":"<attempt-id>","reason":"superseded matrix"}}
JSON
```

Cancellation requires the same project, worktree, session and execution
authority. Foreign attempts are refused with `not your verification attempt`.
It records `interrupted`, releases only that attempt's reservation immediately,
and stops its owned command tree. Runner death also releases its reservation
without waiting for the reservation TTL. An interruption is neither PASS nor a
test failure; rerunning the same matrix creates a fresh attempt. A cancellation
before the first command starts preserves the previous verification record.

Initial `cargo build -p gwt --bin gwtd`, ordinary Cargo builds, TDD tests,
lint, coverage, direct headed browser checks, and pre-push checks run
directly without a verification lease. Completion still requires canonical
verification evidence.

`verify.lease.status` returns `holder_project_relation` (`same_project`,
`other_project`, or `unknown`), `holder_reclaim_candidate`, and
`holder_intervention`. Another project's holder, an unidentified owner, or an
inconclusive activity reading is protected: `holder_intervention: forbidden`.
Additional sampling does not authorize stopping an `unknown` holder. Only a
same-project reclaim candidate or legacy control channel reports
`canonical_release_only`; canonical
release rechecks its state and refuses requests from another project. A refusal
must not be bypassed with `kill` or `pkill`.

`estimated_remaining_ms_uncertain: true` accompanies the ETA: it is a batch
estimate or lease TTL, not a live progress counter. An unchanged value does not
prove a stall. `waiter_action: wait` means waiting for canonical admission is
expected; a pending queue position grants no permission to stop the holder.

`verify.run` saves an unfinished record before starting commands. If the runner
is terminated externally, a companion records the interruption, the last active
command, and any completed command results. `execution.status` distinguishes
`running` and `interrupted` from `missing_record`; interrupted evidence cannot
authorize completion or a PR and requires a new run. The diagnostic copy at
`.gwt/tmp/verify-run.json` is written atomically; the machine-local trusted record
remains authoritative. The interruption reason says `signal unknown` when the
exact signal cannot be observed.

The `pre-push` hook deliberately runs only checks that do not compile the
workspace: `cargo fmt --all -- --check`, Markdownlint, and the SKILL.md
frontmatter validation. A Git hook runs under `git push` rather than under
`gwtd`, so it cannot take the verification lease, and a heavy Cargo job
started there saturates the host while another worktree holds the lease.
Clippy, the test suites, and the 90% coverage threshold are enforced per
pull request by the Lint, Test, and Coverage workflows instead.

**Migration:** `verify.lease.acquire`, `verify.lease.hold`, and
`verify.lease.extend` now return an error without creating a holder or
reservation. Replace manual acquisition around canonical verification with
`verify.run`; remove acquisition around ordinary Cargo commands. Existing
legacy holders can be drained explicitly from their owning project without
killing their processes:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"verify.lease.release","params":{"lease_id":"<lease-id>"}}
JSON
```

Lease transitions are recorded in
`~/.gwt/runtime/verification-coordinator/lease-events.jsonl`. Verification
has its own coordinator lane: semantic search and index builds keep excluding
each other on `~/.gwt/runtime/index-coordinator` (one model-loaded runner at
a time), and neither lane waits for the other.

### PR head verification

To correct a PR targeting the wrong branch, use `pr.edit` with `params.number`
and `params.base` (for example, `develop`). Base alone is a valid update; the
existing editing authority checks still apply. To retire an incorrect PR, use
`pr.close` with `params.number` and optional `params.comment`. A supplied comment
is recorded before closing; if it fails, the PR stays open. Closing preserves
the branch and is available without producing authority or verification evidence.

Use `pr.head_check` with `params.base` (for example, `develop`) and optional
`params.head` to compare a canonical passing verification record with the live
remote head without creating or editing a PR. The JSON diagnostic reports the
record ID, verified/remote/base SHAs, product commits/files, and whether local
verification is still fresh. A base-only comparison does not refresh stale
evidence; missing, incomplete, failed, or corrupt records are unprovable.

Before creating a Ready PR, `pr.create` compares the live remote branch with
the HEAD recorded by `verify.run`. Its response and the PR body preserve both
SHAs, the base SHA, and the comparison result. Bookkeeping under `.gwt/` and
base synchronization alone are allowed. Commits outside the verified history
and the target base that change product files, including changes later reverted,
are refused; extra source changes introduced by a merge are also refused.
Unavailable remote history or an ambiguous comparison cannot authorize Ready.

The refusal lists the product commits and files. Fetch the named remote branch,
fast-forward the local branch with `git merge --ff-only <remote-head-sha>`, then
register the affected verification matrix with `verify.plan` and rerun it with
`verify.run`. Retry `pr.create` after verification passes. A diverged local
branch needs conflict resolution before that fast-forward can succeed.

For an existing PR, `pr.view` compares its current remote head with the verified
SHA retained in its body and reports drift. An older PR for the current branch
can use its passing local verification record; missing evidence is unknown.
To preserve the diagnostic, copy the reported head comparison into `pr.comment`
(`params.number`, `params.body`) or `issue.comment` for the owning Issue. Viewing
a PR does not change its body or confirm that its current head is verified.

### GitHub API budget

Every `gh` call gwt makes shares one GitHub account budget across all
machines, worktrees, and agents. The `pr.list` inventory is cache-first: a
snapshot under `~/.gwt/projects/<hash>/pr-inventory-cache.json` answers
repeated reads for 5 minutes without touching GitHub, the bulk query stays
light, and `statusCheckRollup` / `body` are fetched per PR only when that PR
changed. Pass `params.refresh:true` when a decision needs the live state and
`params.include` (`["checks","body"]`, default `["checks"]`) to choose the
heavy fields. Every answer reports `source`, `cache_age_secs`, `throttled`,
`github_calls`, `hydrated` (successful per-PR fetches), and `skipped_unchanged`
(unchanged PRs skipped during a live read; zero on cache hits); when the budget is below its reserve the last snapshot is
served and `throttled` says why.

Empty checks on unchanged Draft/CI-not-started PRs are reused after snapshot
expiry too. Changes to `updatedAt` or the head commit invalidate their data;
running checks are polled every 10 minutes by default. Hydration runs with at
most five concurrent requests and 30 requests per read. Configure both refresh
intervals independently in `~/.gwt/config.toml` (zero disables that interval):

```toml
[pr_inventory]
cache_ttl_secs = 300
checks_refresh_secs = 600
```

Observe the budget with a free endpoint:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"github.budget","params":{}}
JSON
```

The answer lists the primary windows GitHub reports (`graphql` / `core`),
a local estimate of the per-minute secondary limit (GitHub does not expose
it; the estimate comes from this machine's spawn ledger under
`~/.gwt/github-budget/`), the newest rate-limit refusal, and the throttle
decision a periodic read would get right now.

### Releasing

To cut a release, trigger the **Prepare Release** workflow from GitHub
Actions (Actions → `Prepare Release` → `Run workflow`). It runs on `develop`
and bumps the version, regenerates the `CHANGELOG`, and opens a
`release/vX.Y.Z → main` Release PR from that frozen develop commit. Later
develop merges leave the release head and its CI unchanged. You can release from any branch without
switching to `develop` locally. The `bump` input is `auto` (default),
`patch`, `minor`, or `major`. `auto` never produces a major release:
breaking markers in commits are only listed in the Release PR body, and a
major bump requires choosing `major` explicitly. Review and merge the
generated Release PR;
merging to `main` then runs the release pipeline (tag, GitHub Release,
cross‑platform binaries). Release recovery instructions live in
`.claude/commands/release.md`.

The Release PR body is reference-only: it lists delivered Issues as bare
`#N` references and never carries a closing keyword, because `main` is the
default branch and `Closes #N` there would close an Issue whose acceptance
criteria are still open. Issues are settled when their work merges into
`develop` (see above). After the merge, `release.yml` runs
`scripts/release_close_guard.py`, which reopens any Issue the Release PR
merge itself closed and leaves a marker comment.

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
