---
description: Run environment-aware verification (cargo / pnpm / Playwright-for-WebView-only) and emit an evidence bundle
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Verify Command

Public verification entrypoint for the gwt project.

`gwt-verify` selects the correct test matrix per changed surface in the
current worktree and runs the selected commands with evidence-bundle output.
Playwright is invoked **only** for WebView/browser UI surfaces; Rust crates,
the gwtd CLI, backend code, and release scripts each have their own
dedicated execution path.

## Usage

```text
/gwt:gwt-verify [--mode quick|full|pre-pr] [--headed]
```

If `--mode` is omitted, defaults to `quick`.

## Steps

1. Load `.claude/skills/gwt-verify/SKILL.md` and follow the workflow.
2. Classify changed paths via `references/test-matrix.md`.
3. Bootstrap missing tooling per `references/tooling-bootstrap.md` (auto
   install, otherwise emit `failed: tooling-missing`).
4. If a Browser surface is matched, follow `references/playwright-runbook.md`
   to bring up the GUI server, wait for HTTP 200, and run `pnpm test:visual`.
5. Emit the evidence bundle (`## Verification Report` block).

## Examples

```text
/gwt:gwt-verify
```

```text
/gwt:gwt-verify --mode full
```

```text
/gwt:gwt-verify --mode pre-pr --headed
```

## Chain Suggestion

- Called from `gwt-build-spec` Phase 3 (mode `full`).
- Called from `gwt-manage-pr` before PR create/update (mode `pre-pr`).
- Called manually for ad-hoc verification.
