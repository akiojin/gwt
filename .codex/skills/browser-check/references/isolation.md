## Workflow

1. Capture the launch checkout:
   - Treat the current working directory at skill start as the launch
     checkout.
   - Resolve the repository root and current branch from that same checkout.
   - Record the exact paths used by the launch and hook audit:

     ```bash
     REPO_ROOT="$(git rev-parse --show-toplevel)"
     CHECKOUT_GWT="$REPO_ROOT/target/debug/gwt"
     CHECKOUT_GWTD="$REPO_ROOT/target/debug/gwtd"
     ```

   - Run remaining commands from the repository root or with absolute paths
     under it.
   - Do not switch branches, switch worktrees, or reuse another checkout.

2. Build/resolve the binary:
   - Use only `<repo-root>/target/debug/gwt` for the GUI and
     `<repo-root>/target/debug/gwtd` for the read-only convergence audit.
   - If the user is verifying freshly edited code, run
     `cargo build -p gwt --bin gwt --bin gwtd` first even if the binaries
     already exist.
   - If either binary is missing, build both with the same command.

3. Prepare an isolated check home:
   - Create `CHECK_HOME="$(mktemp -d -t gwt-fresh-home.XXXXXX)"`.
   - Create `"$CHECK_HOME/.gwt"`.
   - If the real `$HOME/.gwt/runtime` exists, symlink it to
     `"$CHECK_HOME/.gwt/runtime"` so startup does not rebuild the runtime.
   - Symlink only credential/config inputs needed by the checkout, such as
     `.codex`, `.claude`, `.config`, `.docker`, `.ssh`, `.gitconfig`,
     `.git-credentials`, `.npmrc`, and `.bunfig.toml`, when they exist.
     `.docker` is required for Docker-runtime launches: on macOS Docker
     Desktop, `docker compose` is a CLI plugin discovered via
     `$HOME/.docker/cli-plugins`, so an isolated HOME without this symlink
     fails the launch preflight with `docker compose is not available`
     (Issue #3029).
   - If the visual check needs GitHub-backed actions, bridge authentication
     into the isolated process without printing secrets:
     - Prefer an existing non-empty `GH_TOKEN`.
     - Else prefer an existing non-empty `GITHUB_TOKEN`.
     - Else, if `gh` is available, run `gh auth token` from the real HOME and
       capture stdout into a shell variable.
     - Never echo, log, write, or Board-post the token. Pass it only as
       `GH_TOKEN` / `GITHUB_TOKEN` in the fresh process environment.
     - If no token can be obtained and the check requires a GitHub mutation,
       stop before sharing a URL and ask the user to run `gh auth login`.
   - Seed `"$CHECK_HOME/.gwt/session.json"` with one active project tab for
     the current repository root so the user lands in the actual app instead
     of the Open Project picker.
   - Resolve `CHECK_REPO_HASH` from the `project_store.hash` returned by the
     checkout `gwtd` JSON operation `issue.monitor.status` run in `REPO_ROOT`;
     do not substitute a hash of the worktree path. Before launching, seed
     the isolated project's preferences as follows. An isolated instance must
     not become a second resident PM or launch implementation agents.

     ```bash
     # browser-check-agent-seed-begin
     CHECK_PROJECT_STATE="$CHECK_HOME/.gwt/projects/${CHECK_REPO_HASH:?Resolve project_store.hash first}/project-state"
     mkdir -p "$CHECK_PROJECT_STATE"
     cat > "$CHECK_PROJECT_STATE/pm.json" <<'JSON'
     {"settings":{"auto_start":false}}
     JSON
     cat > "$CHECK_PROJECT_STATE/issue-monitor.json" <<'JSON'
     {"enabled":false,"max_active_agents":1,"priority_order":[]}
     JSON
     # browser-check-agent-seed-end
     ```

     Keep these files inside `CHECK_HOME`; never symlink the real
     `.gwt/projects` directory or copy its PM/Issue Monitor preferences.
