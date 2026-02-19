## Summary
- Agent tab launch: stop buffering once job id is known to avoid missing pane attachment
- Add a clear placeholder when an agent/terminal tab lacks a pane id
- Respond to DSR cursor position queries from PTY output

## Context
- Launch events can arrive while `launchJobStartPending` is still true, causing `launch-finished` to be buffered and pane id never attached
- That leaves an agent tab visible but the terminal layer empty

## Changes
- Clear `launchJobStartPending` immediately after `start_launch_job` returns
- Render placeholders for detached agent/terminal tabs in main area
- Track DSR cursor position queries and emit a response in the PTY stream

## Testing
- TODO (not run)

## Risk / Impact
- Low: launch flow and UI rendering paths only

## Deployment
- None

## Screenshots
- N/A

## Related Issues / Links
- #1029

## Checklist
- [ ] Tests added/updated
- [ ] Lint/format checked
- [ ] Docs updated
- [ ] Migration/backfill plan included (if needed)
- [ ] Monitoring/alerts updated (if needed)

## Notes
- N/A
