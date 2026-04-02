# Plan: SPEC-1648 — セッション保存・復元

## Summary

session metadata と restore contract を正本化し、raw PTY transcript の長期保存を対象外にする。

## Technical Context

- 設定全般は `SPEC-1542`、タブ構成は `SPEC-1654`。
- resume に必要な session id / tool / branch 情報の永続化を扱う。
- 一時 scrollback はセッション lifetime に限定する。

## Phased Implementation

### Phase 1: Persisted Data Boundary

1. 永続化対象と非対象を整理する。
2. session metadata schema を定義する。
3. branch-targeted launch では actual worktree path を metadata の canonical path とする。

### Phase 2: Restore Flow

1. 起動時 restore と invalid session の扱いを定義する。

### Phase 3: Verification

1. session file 欠損・schema 差分・無効 session id のケースを acceptance に落とす。
