# Acceptance Checklist

- [ ] `#1644` is defined as the canonical owner for GitHub-free local Git backend behavior
- [ ] adjacent specs delegate local Git backend concerns to `#1644`
- [ ] `Local / Remote / All` is defined as ref inventory behavior
- [ ] remote-only refs are not treated as worktree instances
- [ ] same-name local/remote refs preserve canonical identity without ambiguity
- [ ] worktree create/focus resolution rules are explicit
- [ ] worktree display name / linkage / safety remain domain-owned
- [ ] shell consumers can use the domain without Sidebar-specific assumptions
