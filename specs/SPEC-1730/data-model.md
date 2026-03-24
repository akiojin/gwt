# Data Model

## Canonical workflow artifacts

- `doc:spec.md`: intended behavior and acceptance scenarios
- `doc:tasks.md`: execution slices and completion markers
- `checklist:tdd.md`: required test-first evidence model
- `checklist:acceptance.md`: user-visible acceptance completion
- `Progress / Done / Next` issue comments: execution history, not source of truth

## Completion gate inputs

- current code state
- executed verification commands
- `doc:spec.md`
- `doc:tasks.md`
- `checklist:tdd.md`
- `checklist:acceptance.md`
- latest progress comments

## Completion gate outputs

- `PASS`: completion may be declared
- `BLOCKED`: return to `gwt-spec-ops` and repair artifacts or implementation
