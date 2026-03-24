## Canonical Ownership Model

- Workflow / registration: #1579
- Storage / API / artifact CRUD: #1327
- Viewer / Issue detail rendering: #1354
- Search / discovery: #1643

## Lifecycle Order

1. `gwt-issue-register`
2. `gwt-spec-register`
3. `gwt-spec-clarify`
4. `gwt-spec-plan`
5. `gwt-spec-tasks`
6. `gwt-spec-analyze`
7. `gwt-spec-ops`

## GitHub Transport Capability Matrix

- PR list / create / update / view metadata: REST-first
- Commit status / check-runs: REST-first
- PR comments / issue comments: REST-first
- Reviews / review comments: REST-first
- Unresolved review thread discovery: GraphQL-only for now
- Review thread reply / resolve: GraphQL-only for now
