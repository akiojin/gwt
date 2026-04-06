# Module Depth Analysis — Reference

## Deep Module Theory (John Ousterhout)

From "A Philosophy of Software Design":

> The best modules are those that provide powerful functionality yet have simple interfaces. A deep module is a good abstraction because only a small fraction of its internal complexity is visible to users.

### The depth metaphor

```text
Deep module:              Shallow module:
┌──────┐                  ┌────────────────────────┐
│ API  │  ← small         │         API            │  ← large
├──────┤                  ├────────────────────────┤
│      │                  │    implementation       │  ← small
│      │  ← large         └────────────────────────┘
│ impl │
│      │
│      │
└──────┘
```

- **Deep module**: Simple interface, complex implementation. Hides complexity from callers.
- **Shallow module**: Interface complexity rivals implementation complexity. Callers must understand nearly as much as the module itself.

## Shallow Module Detection

### Indicators

1. **Pass-through functions**: A function that does nothing but call another function with the same or trivially transformed arguments
   ```rust
   // Shallow: adds no value
   pub fn get_config() -> Config {
       inner::load_config()
   }
   ```

2. **Wrapper types with no added behavior**: A struct that wraps another struct and exposes the same methods
   ```rust
   // Shallow: just forwarding
   pub struct ConfigManager(Config);
   impl ConfigManager {
       pub fn get(&self) -> &Config { &self.0 }
   }
   ```

3. **One-liner methods dominating a module**: If most public methods are 1-3 lines, the module probably isn't hiding meaningful complexity

4. **Interface mirrors implementation**: The number of public methods roughly equals the number of internal operations

5. **Testability-only extraction**: Pure functions extracted into separate modules solely to make them testable, when they could remain private methods with the owning type tested through its public API

### Measurement heuristic

```text
Depth Score = Implementation Lines / Public API Items

Deep:    > 20 (each API item hides ~20+ lines of complexity)
Normal:  5-20
Shallow: < 5  (each API item hides almost nothing)
```

This is a rough heuristic, not a precise metric. Use judgment.

## God Object Detection

A God object is a type or module that:

- Knows too much (too many fields/dependencies)
- Does too much (too many responsibilities)
- Is changed for too many reasons (violates Single Responsibility)

### Detection criteria

| Signal | Threshold | Severity |
|--------|-----------|----------|
| Public methods | > 10 | High |
| Public methods | > 15 | Critical |
| Direct dependencies (imports/fields) | > 5 | High |
| Direct dependencies | > 8 | Critical |
| Lines of code | > 500 | High |
| Responsibilities (distinct domain concepts) | > 2 | High |

### Common God object patterns in TUI applications

- **App struct**: Holds all application state, handles all events, manages all views
- **State manager**: A single struct managing state for multiple unrelated features
- **Renderer**: A render function that knows about every widget and screen

### Decomposition strategies

1. **Extract by responsibility**: Group methods by the domain concept they serve, create separate types
2. **Extract by lifecycle**: Separate short-lived operations from long-lived state
3. **Trait segregation**: Split a large trait into focused traits that clients can depend on individually

## Coupling Metrics

### Fan-in and Fan-out

- **Fan-in**: Number of modules that depend on this module (import it)
- **Fan-out**: Number of modules this module depends on (imports)

```text
High fan-in + Low fan-out  = Stable foundation (good)
Low fan-in  + High fan-out = Unstable integrator (watch carefully)
High fan-in + High fan-out = Coupling hotspot (bad — changes ripple everywhere)
Low fan-in  + Low fan-out  = Isolated module (fine if intentional)
```

### Coupling hotspot indicators

- Changing this module frequently requires changing other modules
- This module appears in many `use` statements across the codebase
- This module imports from many different peer modules (not just its dependencies)

### Healthy dependency patterns

- **Acyclic**: No circular dependencies between modules
- **Stable dependencies**: Modules should depend on modules that are more stable (less likely to change)
- **Abstract dependencies**: Depend on traits/interfaces, not concrete implementations
