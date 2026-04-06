---
name: gwt-arch-review
description: "Use proactively after completing a major feature or weekly to maintain code quality. Scans domain boundaries (DDD), module depth (Ousterhout), testability, and agent-friendliness. Outputs prioritized improvement report. Triggers: 'review architecture', 'codebase review', 'コードレビュー'."
---

# gwt-arch-review — Codebase Architecture Review

Perform a structured architectural review of any codebase and produce a prioritized improvement report. This skill is read-only — it analyzes and reports but never modifies code.

## When to use

- Weekly or bi-weekly codebase health checks
- Before major feature work to identify structural debt
- After a series of rapid changes to detect architectural drift
- When agent output quality degrades (often signals codebase degradation)

## Core principle

> "If you have a garbage code base, the AI will produce garbage within that code base."
> — aihero.dev

Regular architectural review directly improves agent output quality. This skill closes the feedback loop:

```text
gwt-spec-design → gwt-spec-plan → gwt-spec-build → gwt-arch-review
     ↑                                    |
     └────────────────────────────────────┘
```

## Scope control

Before starting, determine the review scope with the user:

- **Full repository** (default): all production source code
- **Crate/package subset**: specific crates, packages, or modules
- **Changed-files only**: files changed since a base branch (`git diff --name-only <base>..HEAD`)

Exclude from analysis: test fixtures, generated code, vendored dependencies, build artifacts.

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

## Phase 2: Domain Boundary Analysis

**Goal**: Evaluate whether Bounded Contexts are respected and domain logic is properly contained.

Reference: `references/domain-analysis.md`

### Steps

1. **Identify domain concepts**: Extract the core domain vocabulary from type names, function names, module names
2. **Map concepts to modules**: Which modules own which domain concepts?
3. **Detect boundary violations**:
   - Domain logic in infrastructure/UI layers
   - Cross-boundary direct struct access (bypassing interfaces)
   - Shared mutable state across boundaries
4. **Ubiquitous Language check**: Are the same concepts named consistently across the codebase?
   - Same concept, different names (synonym drift)
   - Same name, different meanings (homonym collision)
5. **Dependency direction check**: Do dependencies flow inward (toward domain)?

### Severity classification

- **Critical**: Domain logic in UI/infra layer, circular dependencies between bounded contexts
- **High**: Inconsistent naming of core domain concepts, leaking internal types across boundaries
- **Medium**: Minor naming inconsistencies, slightly misplaced utility functions
- **Low**: Cosmetic naming improvements, documentation gaps

## Phase 3: Module Depth Analysis

**Goal**: Evaluate module design quality using John Ousterhout's Deep Module theory.

Reference: `references/module-depth.md`

### Steps

1. **Interface-to-implementation ratio**: For each public module, compare:
   - Public API surface (exported types, functions, traits)
   - Internal implementation complexity (lines, branching, state)
   - A deep module has a small API hiding significant complexity
   - A shallow module exposes complexity proportional to (or exceeding) its implementation
2. **Shallow module detection**: Flag modules where:
   - Public API is nearly as large as implementation
   - Functions are trivial pass-throughs
   - Abstractions add indirection without hiding complexity
   - Pure functions extracted solely for testability (unnecessary abstraction)
3. **God object detection**: Flag types/modules with:
   - > 10 public methods
   - > 5 direct dependencies
   - Responsibilities spanning multiple domain concepts
4. **Coupling analysis**:
   - Fan-in (how many modules depend on this one)
   - Fan-out (how many modules this one depends on)
   - High fan-in + high fan-out = coupling hotspot

### Severity classification

- **Critical**: God objects (> 15 methods or > 8 dependencies), modules with fan-in > 10 AND fan-out > 5
- **High**: Shallow modules in core domain, pass-through layers adding no value
- **Medium**: Slightly oversized interfaces, moderate coupling
- **Low**: Minor interface simplification opportunities

## Phase 4: Testability & Agent-Friendliness

**Goal**: Assess how well the codebase supports both automated testing and AI agent comprehension.

Reference: `references/agent-friendliness.md`

### Steps

1. **Test coverage gaps**:
   - Modules/files with no corresponding tests
   - Public API functions without test coverage
   - Error paths and edge cases without tests
2. **Untestable patterns**: Code that resists testing:
   - Hard-coded dependencies (no dependency injection)
   - Global mutable state
   - Side effects mixed with logic
   - Large functions that do too many things
3. **Concept locality** (agent-friendliness):
   - Is related code co-located or scattered across many files?
   - Can an agent understand a feature by reading 1-3 files, or does it need 10+?
   - Are there "treasure hunt" patterns where understanding requires jumping through many indirections?
4. **Naming consistency**:
   - Do file names reflect their contents?
   - Are naming conventions consistent (case, prefix/suffix patterns)?
   - Can an agent find the right file by name alone?
5. **Self-documenting structure**:
   - Do module/directory names communicate purpose?
   - Are public APIs documented with doc comments?
   - Do tests serve as living documentation of behavior?

### Severity classification

- **Critical**: Core business logic with zero tests, scattered concepts requiring 10+ files to understand
- **High**: Public API without tests, misleading file/module names, hard-coded dependencies in core
- **Medium**: Missing edge case tests, minor naming inconsistencies
- **Low**: Documentation gaps, test organization improvements

## Phase 5: Report Generation

**Goal**: Produce a single, actionable improvement report.

Reference: `references/report-format.md`

### Report structure

```markdown
# Architecture Review Report

**Repository**: <name>
**Scope**: <full | subset description>
**Date**: <date>
**Reviewed by**: gwt-arch-review

## Executive Summary

<2-3 sentences: overall health, top concern, recommended first action>

## Metrics

| Metric | Value | Status |
|--------|-------|--------|
| Total modules | N | — |
| Files > 300 lines | N | <ok/warn/critical> |
| Files > 500 lines | N | <ok/warn/critical> |
| Circular dependencies | N | <ok/warn/critical> |
| God objects | N | <ok/warn/critical> |
| Untested public APIs | N | <ok/warn/critical> |

## Findings

### Critical

- **[C1]** <title>
  - Files: <paths>
  - Issue: <description>
  - Impact: <why it matters>
  - Suggested action: <what to do>

### High

- **[H1]** ...

### Medium

- **[M1]** ...

### Low

- **[L1]** ...

## Improvement Proposals

Priority-ordered list of concrete improvements:

1. **<proposal title>** — Addresses [C1], [H2]
   - Estimated effort: <small/medium/large>
   - Suggested approach: <1-2 sentences>
   - Create SPEC via: `gwt-spec-design`

2. ...

## Feedback Loop

To act on these findings:
1. Pick the top 1-3 proposals
2. Use `gwt-spec-design` to create improvement SPECs
3. Follow the normal gwt-spec-plan → gwt-spec-build pipeline
4. Run `gwt-arch-review` again after implementation to verify improvement
```

## Execution rules

1. **Read-only**: Never modify source code, tests, or configuration
2. **Evidence-based**: Every finding must cite specific file paths and line ranges
3. **Actionable**: Every finding must include a suggested action
4. **Proportional**: Don't flag trivia in a codebase with critical issues — focus attention on what matters most
5. **Repository-agnostic**: This skill works on any codebase, not just gwt
6. **No SPEC dependency**: This skill operates independently of the SPEC pipeline — it reads source code directly
7. **Severity budget**: Aim for 3-5 Critical, 5-10 High, 10-15 Medium findings. More than that dilutes focus

## Limitations

- Static analysis only — no runtime profiling or performance measurement
- Heuristic-based — findings are informed opinions, not mathematical proofs
- Language-aware for Rust primarily; other languages get structural analysis without deep semantic checks
- Does not replace security audits, performance profiling, or compliance reviews
