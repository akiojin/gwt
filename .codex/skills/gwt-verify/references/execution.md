## Invocation Sequence

Canonical Heavy Cargo admission uses a bounded host slot pool (#5082).
Independent worktrees and effective Cargo target directories may run
concurrently, subject to CPU/memory capacity and disk reservations. Set
`[verification] slots` or `disk_budget_bytes` in the global config to override
the defaults. The same worktree or target remains serialized; unknown wrappers
and older exclusive holders serialize against the entire pool. Children use
isolated temporary directories. `verify.lease.status` lists capacity,
running/available slots, every holder's ETA, and the shared FIFO queue.

```text
agent → /gwt:gwt-verify [--mode quick|full|pre-pr] [--headed] [--skip-user-check]
  ↓
gwt-verify
  1. Establish baseline:
       git diff --name-only $(git merge-base HEAD origin/develop)..HEAD
     (fall back to origin/main when origin/develop is absent). For
     `--mode quick` also include `git diff --name-only` (working tree).
  2. Classify each path into an abstract surface using
     [references/surface-taxonomy.md](surface-taxonomy.md).
  3. Discover the project's test runners from its manifests using
     [references/runner-detection.md](runner-detection.md) (Cargo.toml → cargo,
     package.json → npm / pnpm / yarn scripts, pyproject.toml → pytest,
     go.mod → go test, ProjectSettings/ProjectVersion.txt → Unity Editor
     batch test runner, *.sln / *.csproj → dotnet test, Gemfile → rspec
     / minitest, pom.xml / build.gradle → mvn / gradle test,
     Makefile → make test targets). Multiple runners may coexist
     (e.g. Cargo + pnpm).
  4. Select appropriate test commands per changed surface. Record the
     selection rationale in the evidence bundle's `Plan` section. Honor
     project-local AGENTS.md / README testing instructions when present;
     the generic matrix is the fallback.
  5. Tooling presence check: verify each chosen runner / browser / editor
     is installed. Missing entries follow [references/tooling-bootstrap.md](tooling-bootstrap.md)
     (best-effort install attempt for runners with auto-install paths;
     otherwise exit with `failed: tooling-missing`).
  6. Run each selected command sequentially. Capture stdout / stderr /
     exit code. For browser-based UI runners, bring up the application
     server per [references/playwright-runbook.md](playwright-runbook.md) before invoking the
     runner. When the runner supports a structured reporter (JSON / TAP /
     list / JUnit XML), use it so test-level results can be extracted.
     In gwt-managed execution worktrees, first register the derived matrix
     with JSON operation `verify.plan` (`params.commands:[...]`), then run
     it in full through `verify.run` (one plain command per entry, no shell
     operators): gwtd executes the commands itself and writes the
     tool-generated Verification Run Record that `execution.complete` and
     Ready PR handoffs require (SPEC-3248 P8b/T-130). A run that does not
     cover the registered plan — or a run without a plan — never satisfies
     those gates, and prose summaries of test runs do not either.
  7. Extract a Test Inventory from each runner's output: test name,
     describe block, scenario / snapshot title, lint rule name, etc.
     If extraction fails for a runner, record
     `(inventory unavailable: <reason>)` rather than silently degrading.
  8. Emit the evidence bundle described below, then run the User
     Verification Handoff phase (unless skipped per the rules above) and
     finalize Overall once the user response is recorded.
```

The skill does not invoke Playwright — or any other UI runner (Cypress /
Selenium / WinAppDriver / Unity Editor headed) — when no user-facing surface
is in scope. A UI surface is in scope either by the diff touching WebView /
browser UI files, or by [references/surface-taxonomy.md](surface-taxonomy.md)'s acceptance-aware
escalation promoting a backend-only diff whose acceptance manifests in a
user-facing surface. A change with no user-facing surface in scope by diff or
acceptance does not invoke Playwright. This restraint is part of the
gwt-verify verification contract that prevents over-eager UI-runner
invocation; it is not an external SPEC requirement.
