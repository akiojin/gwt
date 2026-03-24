1. Use #1579 to reason about workflow order and registration.
2. Use #1327 to reason about artifact storage/API behavior.
3. Use #1354 to reason about Issue detail rendering.
4. Use #1643 to reason about search and discovery only.
5. Treat migration as part of the redesign, not a separate cleanup task.
6. For embedded GitHub skills, choose REST first for metadata/check/comment paths, and keep GraphQL only for unresolved review-thread operations that still lack practical REST coverage.
