## Phase 4: Quality Gate

Final readiness check before implementation. This is a pre-implementation gate only —
it does not certify that implementation is complete.

> See [references/quality-gate.md](quality-gate.md) for full check definitions.

### Mandatory checks

1. **Clarification completeness** — no critical `[NEEDS CLARIFICATION]` markers remain
2. **Spec completeness** — User Stories, Acceptance Scenarios, Edge Cases, Requirements,
   and Success Criteria exist
3. **Plan completeness** — Constitution Check exists, Technical Context and Phased
   Implementation are concrete
4. **Task traceability** — every user story has tasks, every acceptance scenario has
   verification coverage, every contract/data-model change has matching tasks
5. **Constitution alignment** — violations are either removed or explicitly tracked in
   Complexity Tracking

### Verdict

```text
## <Analysis Report in the current user's language>: SPEC-<id>

Status: CLEAR | AUTO-FIXABLE | NEEDS-DECISION

Blocking items:
- A1. <artifact gap>
- A2. <traceability gap>

Next:
- gwt-build-spec (on CLEAR)
- self-repair and rerun (on AUTO-FIXABLE)
- ask user for decision (on NEEDS-DECISION)
```

**Decision rule:**

- **CLEAR** — implementation may proceed through `gwt-build-spec`
- **AUTO-FIXABLE** — repair the artifact set in-place and rerun the quality gate
- **NEEDS-DECISION** — report points to the exact user decision or unresolved ambiguity

**Boundary:** CLEAR means artifacts are ready for execution. It does not mean the SPEC is
complete — completion requires post-implementation reconciliation in `gwt-build-spec`.

## Chain suggestion

On `CLEAR` verdict, suggest proceeding to `gwt-build-spec` for implementation.

## Lightweight mode

When invoked without a SPEC:

- Skip traceability checks (Phase 4 check 4)
- Skip constitution check if `.gwt/memory/constitution.md` is not found
- Produce `plan.md` and `tasks.md` in the current directory
- Omit supporting artifacts unless the user requests them

## Operations

Use gwtd JSON operations for artifact persistence:

```bash
# Read spec section
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.section","params":{"number":123,"section":"spec"}}
JSON

# Write plan section
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.edit","params":{"number":123,"section":"plan","body":"<plan markdown>"}}
JSON

# Write tasks section
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.edit","params":{"number":123,"section":"tasks","body":"<tasks markdown>"}}
JSON

# List all SPEC Issues
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.list","params":{}}
JSON
```
