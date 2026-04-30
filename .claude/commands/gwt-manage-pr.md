---
description: Create, inspect, update, or unblock a PR through the visible PR workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Manage PR Command
=================

Public task entrypoint for PR lifecycle work.

Usage
-----

```text
/gwt:gwt-manage-pr [action or context]
```

Steps
-----

1. Load `.claude/skills/gwt-manage-pr/SKILL.md` and follow the workflow.
2. Resolve the daemon CLI before acting, then make sure `"$GWT_BIN" pr current` succeeds so auth and current-branch PR state are known:
   ```bash
   resolve_gwt_bin() {
     if [ -n "${GWT_BIN_PATH:-}" ] && [ -x "$GWT_BIN_PATH" ]; then
       printf '%s\n' "$GWT_BIN_PATH"
       return 0
     fi
     if command -v gwtd >/dev/null 2>&1; then
       command -v gwtd
       return 0
     fi
     repo_root="${GWT_PROJECT_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
     if [ -x "$repo_root/target/debug/gwtd" ]; then
       printf '%s\n' "$repo_root/target/debug/gwtd"
       return 0
     fi
     printf '%s\n' "gwtd not found; set GWT_BIN_PATH, install gwtd into PATH, or run cargo build -p gwt --bin gwtd." >&2
     return 127
   }
   GWT_BIN="$(resolve_gwt_bin)" || exit $?
   "$GWT_BIN" pr current
   ```
3. Use the current branch and PR state to choose create, status, or unblock actions.
4. If the PR is conflicting or behind, route directly into the fix flow.
5. Keep PR work behind this visible entrypoint.

Examples
--------

```text
/gwt:gwt-manage-pr
```

```text
/gwt:gwt-manage-pr check status
```

```text
/gwt:gwt-manage-pr fix
```
