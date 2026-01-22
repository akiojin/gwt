# gwt Operations Guide

This guide summarizes tmux-focused operational commands and keybinds used with gwt.
It targets quick recovery, inspection, and pane control in a running gwt tmux session.

## tmux Command Snippets

Placeholders:
- `<session-name>`: tmux session name (example: `gwt-main`)
- `<pane-id>`: pane identifier from `tmux list-panes` (example: `%1`)
- `<dir>`: working directory path
- `<cmd>`: command to run in the new pane (quote it if it contains spaces)

### Create a detached session

Use this to start a tmux session without attaching immediately.

```text
tmux new-session -d -s <session-name>
```

Example:

```text
tmux new-session -d -s gwt-main
```

### Split panes

Use `-h` for a horizontal split (side by side) or `-v` for a vertical split (stacked).
The `-c <dir>` option sets the working directory for the new pane.

```text
tmux split-window -h -c <dir> <cmd>      # split pane horizontally
```

```text
tmux split-window -v -c <dir> <cmd>      # split pane vertically
```

Examples:

```text
tmux split-window -h -c /repo bash
```

```text
tmux split-window -v -c /repo "bunx -p @akiojin/gwt@latest gwt"
```

### Focus a pane

Use this when you know the pane ID and want to jump focus.

```text
tmux select-pane -t <pane-id>
```

Example:

```text
tmux select-pane -t %1
```

### List panes

Use this to inspect pane IDs, PIDs, and the running command.

```text
tmux list-panes -F "#{pane_id}:#{pane_pid}:#{pane_current_command}"
```

Example:

```text
tmux list-panes -F "#{pane_id}:#{pane_pid}:#{pane_current_command}"
```

### Send keys (interrupt)

Use this to send Ctrl-C to a pane.

```text
tmux send-keys -t <pane-id> C-c
```

Example:

```text
tmux send-keys -t %2 C-c
```

### Kill a pane

Use this to close a pane that is no longer needed.

```text
tmux kill-pane -t <pane-id>
```

Example:

```text
tmux kill-pane -t %2
```

## Ctrl-g Keybind

`Ctrl-g` is intended as a quick return to the gwt TUI pane.
In the default layout, pane `0` hosts the gwt UI, so the keybind focuses pane `0`.
If you changed the layout or pane numbering, update the target pane ID accordingly.

```text
tmux bind-key -n C-g select-pane -t 0    # focus gwt pane (pane 0)
```
