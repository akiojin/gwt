# Data Model: SPEC-1787

## Modified Enums

### RepoType (gwt-core)

```
Before:  Normal | Bare | Worktree | Empty | NonRepo
After:   Normal | Worktree | Empty | NonRepo
```

### ActiveLayer (gwt-tui)

```
Before:  Main | Management
After:   Main | Management | Initialization
```

### CloneType (gwt-tui)

```
Before:  Bare | BareShallow
After:   (removed — single clone mode, no enum needed)
```

### CloneStep (gwt-tui)

```
Before:  UrlInput | TypeSelect | Cloning | Done | Failed
After:   UrlInput | Cloning | Done | Failed
```

## New Structures

### Pre-commit Hook Script

```bash
#!/bin/sh
# gwt-develop-guard-start
branch=$(git symbolic-ref HEAD 2>/dev/null)
if [ "$branch" = "refs/heads/develop" ]; then
  echo "ERROR: Direct commits to develop are not allowed."
  echo "Create a feature branch first: git checkout -b feature/feature-{N}"
  exit 1
fi
# gwt-develop-guard-end
```

## Modified Methods

### Model::reset(new_repo_root: PathBuf)

Resets the entire Model state for a new repository:
- Updates `self.repo_root`
- Clears `self.session_tabs`
- Calls `load_all_data()` to reload branches, specs, issues, versions, logs, settings
- Sets `active_layer = ActiveLayer::Management`
- Sets `management_tab = ManagementTab::Specs`

### Model::load_all_data(repo_root: &Path)

Extracted from `app::run()` data loading section:
- `load_branches(repo_root)`
- `load_settings()`
- `load_log_entries(repo_root)`
- `load_specs(repo_root)` (for Issues tab)
- `load_specs(repo_root)` (for SPECs tab)
- `load_tags(repo_root)`

## Deleted Structures

- `BareProjectConfig` (bare_project.rs) — entire struct and file
- `MigrationDialogState` (migration_dialog.rs) — entire struct and file
- `WorktreeLocation::Sibling` — variant removed, Subdir only
