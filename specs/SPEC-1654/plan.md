# Plan: SPEC-1654 — branch-first workspace shell

## Summary

Rewrite the shell spec around `Branches` as the primary entry, `permanent multi-mode` as the only session model, and `Branches / SPECs / Issues / Profiles` as the management tabs. Keep terminal behavior, persistence, and launch contracts delegated to their canonical specs.

## Technical Context

- Parent UX direction: `SPEC-1776`
- Terminal behavior: `SPEC-1541`
- Interaction policy: `SPEC-1770`
- Session persistence: `SPEC-1648`
- Local git/worktree truth: `SPEC-1644`
- Agent launch contract: `SPEC-1646`

## Phased Implementation

1. Replace the old `tab-first` shell wording with `branch-first` entry and `no/one/many` enter behavior.
2. Define `equal grid` and `maximize + tab switch` as the two session workspace states.
3. Define the management workspace as `Branches / SPECs / Issues / Profiles`.
4. Clarify restore boundaries against `SPEC-1648` and remove stale assumptions about hidden panes or tab-only shell topology.
