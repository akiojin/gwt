# Operations Guide

## tmux Command Snippets

```text
tmux new-session -d -s <session-name>    # create session
tmux split-window -h/-v -c <dir> <cmd>   # split pane
tmux select-pane -t <pane-id>            # focus pane
tmux list-panes -F "#{pane_id}:#{pane_pid}:#{pane_current_command}"  # list panes
tmux send-keys -t <pane-id> C-c          # interrupt
tmux kill-pane -t <pane-id>              # kill pane
```

## Ctrl-g Keybind

```text
tmux bind-key -n C-g select-pane -t 0    # focus gwt pane (pane 0)
```
