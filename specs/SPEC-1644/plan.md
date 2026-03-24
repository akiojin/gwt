# Plan

## Summary

Expand `#1644` from ref/worktree domain into the canonical local Git backend spec. `#1654` owns shell composition, `#1647` owns project lifecycle orchestration, `#1714` owns issue linkage/exact cache, `#1643` owns GitHub integration, and `#1649` owns PR lifecycle. `#1644` owns GitHub-free local Git backend truth, projections, cache/invalidation, and worktree actions.

## Technical Context

- Backend projections and actions in `crates/gwt-tauri/src/commands/branches.rs`, `cleanup.rs`, and local Git helpers consumed from project/shell flows
- Shell/project/PR consumers must stop becoming accidental owners of branch inventory or refresh policy
- Supporting linkage/cache dependency: `#1714`

## Constitution Check

- Keep backend truth separate from shell composition and project orchestration
- Define ownership exactness before further Git backend fixes or performance work
- Do not duplicate local Git semantics across shell/project/PR specs
- Keep GitHub-linked metadata behind `#1714` / `#1643` boundaries

## Project Structure

- Local Git backend services: inventory snapshot, worktree projection, action resolution, detail hydration, cache invalidation
- Ref inventory projects local and remote refs into canonical entries
- Worktree instance projects realized worktrees and their metadata
- Adjacent specs consume or augment this domain but do not own it

## Complexity Tracking

- **Accepted**: explicit ownership split between local Git backend and shell/project/GitHub/PR/cache specs
- **Accepted**: detail hydration and cache invalidation belong to backend domain, not shell layout
- **Accepted**: remote-only refs remain inventory-only
- **Rejected**: distributing local Git backend rules across consumer specs

## Phased Implementation

1. Refresh `#1644` ownership and rename it as the local Git backend canonical spec
2. Update cross-spec boundaries in `#1654`, `#1647`, `#1714`, `#1643`, and `#1649`
3. Keep projection/resolution and metadata rules under `#1644`
4. Route future local Git backend performance/cache fixes through `#1644`
