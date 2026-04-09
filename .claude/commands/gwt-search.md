---
description: Unified semantic search over SPECs, Issues, and project source files
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Search Command

Unified semantic search over local SPECs, GitHub Issues, and project source files using ChromaDB vector embeddings. Use as mandatory preflight before creating new SPECs or Issues.

## Usage

```text
/gwt:gwt-search [query] [--specs] [--issues] [--files]
```

## Steps

1. Load `.claude/skills/gwt-search/SKILL.md` and follow the workflow.
2. Execute the search query against the specified targets.
3. Return ranked results with relevance scores.

## Examples

```text
/gwt:gwt-search "terminal emulation"
```

```text
/gwt:gwt-search --specs "terminal"
```

```text
/gwt:gwt-search --issues "crash on resize"
```
