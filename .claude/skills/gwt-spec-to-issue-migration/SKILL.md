---
name: gwt-spec-to-issue-migration
description: "Migrate GitHub Issue-based specs to local SPEC directories. Supports reverse migration from gwt-spec Issues to local specs/SPEC-{id}/ directories using the bundled migration script. Use when user says 'migrate specs', 'convert issues to local specs', 'move specs from issues', or asks to transform Issue-based specs into local SPEC directories."
---

# gwt Spec Migration (Issue to Local)

## Overview

Use this skill for spec migrations from GitHub Issues to local directories:

- existing `gwt-spec` Issues that keep spec artifacts as issue comments
- body-canonical `gwt-spec` Issues that keep the canonical bundle in the Issue body

Migrate Issue-based specs to local `specs/SPEC-{id}/` directories, then retire the Issue as the source of truth for spec artifacts.

This skill uses:
- `${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-to-issue-migration/scripts/reverse-migrate.py`
- `${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py`

## Preconditions

- Run in repository root
- `gh auth status` is authenticated for target repo (needed to read Issue data during migration)
- Branch policy is respected (no branch creation/switching unless user requests)
- `$GWT_PROJECT_ROOT` environment variable is available; prefer it over CWD for repo resolution

## Standard Workflow

1. List `gwt-spec` Issues to identify migration candidates
2. Run dry-run automatically and review planned migration count
3. If the user explicitly asked to migrate or convert, continue into actual migration after the dry-run
4. Verify migrated local SPEC directories exist under `specs/`
5. Confirm each migrated SPEC has `metadata.json`, `spec.md`, and other artifacts
6. Ask the user only when migration intent is unclear or the requested scope does not obviously include the detected changes

## Commands

### Dry-run

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-to-issue-migration/scripts/reverse-migrate.py" --dry-run
```

### Dry-run with specific issues

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-to-issue-migration/scripts/reverse-migrate.py" --dry-run --issues "42,55,78"
```

### Execute migration

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-to-issue-migration/scripts/reverse-migrate.py"
```

### Verify migrated SPECs

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --list-all
```

### Verify report

```bash
cat migration-report.json
```

Note: `migration-report.json` is deleted automatically after a fully successful migration cleanup. It remains available for dry-run and failure cases.

## Expected Behavior

- Fetches `gwt-spec` Issues and their artifact comments via `gh` CLI
- Creates local `specs/SPEC-{N}/` directories for each migrated Issue
- Populates `metadata.json` with Issue metadata (title, status, phase)
- Extracts `doc:*` artifacts from Issue comments into local files (`spec.md`, `plan.md`, `tasks.md`, etc.)
- Extracts `contract:*` and `checklist:*` artifacts into `contracts/` and `checklists/` subdirectories
- Writes per-spec result to `migration-report.json`
- Shows planned migrations during `--dry-run`
- Treats an explicit "migrate/convert" request as approval to execute after the dry-run summary, without an extra confirmation loop

## Target directory structure

```text
specs/SPEC-{N}/
  metadata.json
  spec.md
  plan.md
  tasks.md
  research.md
  data-model.md
  quickstart.md
  contracts/
  checklists/
```

## Notes

- For safety, always run `--dry-run` first.
- After migration, ongoing spec updates should use local SPEC operations (`gwt-spec-ops`).
- The original `gwt-spec` Issues are not closed or modified by default; they remain as historical references.
- This skill replaces the old `migrate-specs-to-issues.mjs` workflow which moved in the opposite direction.
