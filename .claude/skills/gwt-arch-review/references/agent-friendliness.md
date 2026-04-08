# Agent-Friendliness — Reference

## Why agent-friendliness matters

AI coding agents navigate codebases by reading files, searching for patterns, and building mental models of how code fits together. Codebases that are easy for agents to understand produce better agent output. The same properties that help agents also help human developers.

> Code that confuses an AI agent will also confuse a new team member.

## Concept Locality

### Definition

Concept locality measures how many files an agent must read to fully understand a single feature or concept.

### Good locality (1-3 files)

```text
feature_x/
  mod.rs        ← public API + types
  handler.rs    ← logic
  tests.rs      ← tests
```

An agent reads 3 files and understands the entire feature.

### Poor locality (scattered across 10+ files)

```text
types/feature_x.rs        ← types here
handlers/feature_x.rs     ← logic here
services/feature_x.rs     ← more logic here
utils/feature_x_helper.rs ← helpers here
config/feature_x.rs       ← config here
tests/feature_x_test.rs   ← tests here
errors/feature_x.rs       ← errors here
```

An agent must read 7+ files across different directories. Worse: it may not find them all.

### Detection

1. Pick a feature or concept
2. Count the files that must be read to fully understand it
3. If the count exceeds 5, locality is poor
4. If related files are in 3+ different directory subtrees, locality is poor

### Improvement strategies

- **Co-locate by feature**, not by layer/type
- Keep types, logic, and tests for a feature in the same module directory
- Use `mod.rs` to re-export the public API, keeping internals adjacent

## Naming Conventions

### File naming

- File names should communicate the primary concept they contain
- An agent searching for "config" should find files named `config.rs`, not `utils.rs` containing config logic
- Consistent suffixes: `*_handler`, `*_service`, `*_widget` help agents predict file contents

### Type naming

- Types should use domain vocabulary consistently
- Avoid generic names: `Manager`, `Helper`, `Util`, `Handler` without domain qualification
- Prefix or suffix with the bounded context when types might collide across modules

### Function naming

- Verb-first for actions: `create_branch`, `render_widget`, `validate_config`
- Consistent verb choices: don't mix `get_*`, `fetch_*`, `load_*`, `read_*` for semantically identical operations
- Boolean functions: `is_*`, `has_*`, `can_*`

### Detection checklist

- [ ] Can an agent find the right file by searching for the concept name?
- [ ] Do all files in a directory follow the same naming convention?
- [ ] Are there files named `utils.rs`, `helpers.rs`, or `misc.rs` that contain unrelated functions?
- [ ] Do type names clearly communicate their purpose without reading their implementation?

## File Organization

### Optimal file size

- **Sweet spot**: 50-300 lines per file
- **Warning**: > 300 lines suggests the file has multiple responsibilities
- **Critical**: > 500 lines almost certainly needs splitting
- **Too small**: < 20 lines may indicate unnecessary fragmentation (shallow modules)

### Directory structure patterns

**Feature-first** (agent-friendly):

```text
crates/core/
  git/
    mod.rs
    branch.rs
    commit.rs
    tests.rs
  config/
    mod.rs
    settings.rs
    tests.rs
```

**Layer-first** (agent-hostile):

```text
crates/core/
  types/
    git.rs
    config.rs
  services/
    git.rs
    config.rs
  tests/
    git_test.rs
    config_test.rs
```

### Treasure hunt anti-pattern

A "treasure hunt" is when understanding a concept requires following a chain of indirections:

```text
main.rs → app.rs → router.rs → handler.rs → service.rs → repository.rs → model.rs
```

Each file adds a thin layer. The agent must read all 7 to understand what happens when a user triggers an action. Deep modules reduce this: collapse unnecessary layers.

## Test Coverage as Documentation

### Tests as agent context

Well-written tests serve as executable documentation. When an agent reads tests, it learns:

- What the module's public API looks like
- What inputs are valid and invalid
- What outputs to expect
- What edge cases exist

### Test organization for agents

- Place unit tests adjacent to the code they test (`#[cfg(test)]` module or `tests.rs` in the same directory)
- Name test functions descriptively: `test_branch_creation_with_empty_name_returns_error`
- Group tests by scenario, not by function
- Integration tests in `tests/` directory with names matching the feature they exercise

### Coverage gaps that hurt agents

- **Public API without tests**: Agent can't verify its understanding of the API contract
- **Error paths without tests**: Agent may not handle errors correctly
- **State transitions without tests**: Agent can't reason about valid state sequences

### Detection

1. List all public functions/methods
2. Check each has at least one corresponding test
3. Flag modules with zero test files
4. Flag public functions that appear in no test
