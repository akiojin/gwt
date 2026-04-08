# Domain Boundary Analysis — Reference

## Bounded Context Detection Heuristics

A Bounded Context (BC) is a region of the codebase where a particular domain model applies consistently. In practice, BCs often align with:

- Top-level crates or packages (`gwt-core`, `gwt-tui`)
- Module directories with their own public API (`mod.rs` or `lib.rs`)
- Namespaces that group related types and behaviors

### Detection steps

1. **List all public types and traits** per module
2. **Cluster by domain vocabulary**: types that share naming prefixes or domain terms belong to the same BC
3. **Check import direction**: BC-internal imports are fine; cross-BC imports signal potential boundary issues
4. **Look for aggregates**: a type that owns other types and enforces invariants is likely an aggregate root, anchoring a BC

## Boundary Violation Patterns

### Domain logic in infrastructure

- **Symptom**: Business rules (validation, state transitions, calculations) appear in HTTP handlers, CLI parsers, TUI rendering code, or database access layers
- **Detection**: Search for conditional logic (`if`/`match` on domain enums) outside domain modules
- **Example**: A TUI widget that decides whether a Git operation is allowed, instead of delegating to a domain service

### Domain logic in UI layer

- **Symptom**: Rendering code contains business decisions
- **Detection**: UI modules importing domain types and calling methods that mutate state or make decisions
- **Fix direction**: UI should receive pre-computed view models; decisions belong in the domain layer

### Cross-boundary struct access

- **Symptom**: Module A directly accesses fields of a struct defined in Module B, bypassing B's public API
- **Detection**: Look for `module_b::SomeStruct { field: ... }` construction or `obj.field` access from outside the owning module
- **Risk**: Changes to B's internal structure break A

### Shared mutable state

- **Symptom**: Multiple BCs read/write the same global or static mutable state
- **Detection**: `static mut`, `lazy_static`, `once_cell` with interior mutability accessed from multiple modules
- **Risk**: Hidden coupling, race conditions, unpredictable behavior

## Ubiquitous Language Analysis

### Synonym drift

Same concept, different names across the codebase:

- Check: Do `Task` and `Job` and `WorkItem` refer to the same thing?
- Method: Collect all type names, group by semantic similarity, flag groups with > 1 name for the same concept

### Homonym collision

Same name, different meanings in different contexts:

- Check: Does `Config` mean the same thing in `gwt-core` and `gwt-tui`?
- Method: For each commonly-used name, compare its fields/methods across modules

### Consistency checklist

- Are abbreviations consistent? (`repo` vs `repository`, `cfg` vs `config`)
- Do function names follow a consistent verb pattern? (`get_*`, `fetch_*`, `load_*` for the same operation type)
- Do error types use consistent naming? (`*Error` vs `*Err` vs `*Failure`)

## Dependency Direction Rules

In a well-structured codebase:

```text
UI / CLI / TUI
      ↓
Application Services
      ↓
Domain (core business logic)
      ↓
Infrastructure interfaces (traits)

Infrastructure implementations → Domain interfaces
```

- **Inward is good**: outer layers depend on inner layers
- **Outward is bad**: domain depending on infrastructure details
- **Lateral is suspicious**: two modules at the same layer depending on each other may indicate a missing abstraction

### Quick check for Rust crates

1. Read `Cargo.toml` dependencies for each crate
2. Draw the dependency arrow
3. Verify domain crates don't depend on UI or infrastructure crates
