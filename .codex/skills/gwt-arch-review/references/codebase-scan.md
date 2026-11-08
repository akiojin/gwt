## Phase 1: Codebase Scan

**Goal**: Build a structural map of the codebase.

### Steps

1. **Directory tree**: List top-level directories and their purposes
2. **Module inventory**: Identify all modules/crates/packages with their public API surface
3. **File size distribution**: Flag files exceeding thresholds:
   - Warning: > 300 lines
   - Critical: > 500 lines
4. **Dependency graph**: Map inter-module dependencies (which module imports which)
5. **Entry points**: Identify main entry points, binary targets, library roots

### Output

```text
## Phase 1: Codebase Scan

Modules: <count>
Total source files: <count>
Median file size: <lines> lines
Files > 300 lines: <list with paths>
Files > 500 lines: <list with paths>

Dependency direction:
  <module-a> → <module-b> (N imports)
  ...

Entry points:
  - <path>: <purpose>
```
