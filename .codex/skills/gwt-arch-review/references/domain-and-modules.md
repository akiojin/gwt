## Phase 2: Domain Boundary Analysis

**Goal**: Evaluate whether Bounded Contexts are respected and domain logic is properly contained.

Reference: [references/domain-analysis.md](domain-analysis.md)

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

Reference: [references/module-depth.md](module-depth.md)

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
