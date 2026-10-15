## When to use

### Mandatory preflight

This skill is a **mandatory preflight step** before:

- `gwt-discussion`
- `gwt-register-issue`
- `gwt-fix-issue`
- any visible workflow that must decide an existing SPEC/Issue owner

Establish sufficient duplicate/owner coverage before creating a new SPEC or Issue. Query variations help when terminology or ownership is uncertain; reuse current evidence when it already answers the question.

### General use cases

- **Spec integration**: find the canonical spec before creating or updating
- **Issue lookup**: find existing GitHub Issues before creating new ones
- **Memory lookup**: before fixing a bug, check whether a prior work-notes memory entry already records the prevention strategy
- **Task start**: search for specs, issues, files, and memory related to the assigned feature
- **Bug investigation**: find issues, files, and memory that might relate to the bug
- **Duplicate check**: verify no existing spec, issue, or memory covers the same scope
- **Architecture understanding**: discover how features are specified, implemented, and previously failed
- **Feature addition**: locate existing similar implementations and recurring failure modes across all scopes

### Trigger phrases

- "search specs / issues / files / memory"
- "find related specs / issues / files / memory"
- "check for duplicates"
- "which spec / issue handles X"
- "has this regression been recorded?"
- "既存仕様を探して"
- "関連 Issue を探して"
- "どの SPEC に統合するべきか"
- "重複する SPEC はないか確認して"
- "この機能の仕様は？"
- "過去 memory を引いて"
- "同じ失敗があるか確認して"

## Suggested query patterns

Useful query angles when broader coverage is needed:

- **Subsystem + purpose**: `project index issue search spec`
- **User-facing problem + architecture term**: `chroma persisted db recovery project index`
- **Workflow + discoverability**: `LLM should use search before spec creation`
- **Japanese keywords**: `ターミナル ナビゲーション キーバインド`
- **Domain concept**: `worktree management branch isolation`
- **Past failure**: `watcher debounce silent failure`, `spec section マーカー罠`

## Suggested search workflow

1. Resolve `GWT_BIN`, then query the relevant corpus with `search` JSON
   envelopes when current evidence does not establish owner coverage
2. A missing index is auto-built on the first call
3. Pick the canonical existing spec, issue, or memory if found
4. Only fall back to creating a new spec or issue when no suitable canonical match exists
