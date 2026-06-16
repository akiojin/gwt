---
description: Unified semantic search over SPECs, Issues, project source files, memory, and precision all-terms matching
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Search Command

Unified semantic search over SPEC Issues, GitHub Issues, project source files, and reusable project memory using ChromaDB vector embeddings. Use as mandatory preflight before creating new SPECs or Issues.

## Usage

```text
/gwt:gwt-search {"query":"...","scopes":["issues"],"match_mode":"semantic"}
```

Allowed `scopes` values include `specs`, `issues`, `files`, `files_docs`,
`memory`, `board`, and `discussions`. `match_mode` accepts `semantic` or
`all_terms`.

## Steps

1. Load `.claude/skills/gwt-search/SKILL.md` and follow the workflow.
2. Execute the search query against the specified targets.
3. Return ranked results with relevance scores.

## Examples

```text
/gwt:gwt-search "terminal emulation"
```

```text
/gwt:gwt-search {"query":"terminal","scopes":["specs"]}
```

```text
/gwt:gwt-search {"query":"crash on resize","scopes":["issues"]}
```

```text
/gwt:gwt-search {"query":"watcher debounce","scopes":["memory"]}
```

```text
/gwt:gwt-search {"query":"Workspace 置き換え","match_mode":"all_terms"}
```
