## Phase 4: Testability & Agent-Friendliness

**Goal**: Assess how well the codebase supports both automated testing and AI agent comprehension.

Reference: [references/agent-friendliness.md](agent-friendliness.md)

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
