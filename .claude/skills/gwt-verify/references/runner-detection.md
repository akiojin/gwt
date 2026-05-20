# Test-Runner Detection (project-agnostic)

`gwt-verify` autodetects the host project's test runners from manifest
signals before selecting any commands. A project may expose multiple
runners (e.g. a Rust workspace that also contains a pnpm-driven web
frontend); the agent picks one per surface based on which runner owns
that surface's files.

The host project's own AGENTS.md / CLAUDE.md / README always takes
precedence. Use this table only when the project did not state a testing
approach explicitly.

## Manifest → Runner Table

| Manifest / signal at project root | Primary runner family | Typical test invocation | Structured-output flag for inventory |
|---|---|---|---|
| `Cargo.toml` (workspace or single-crate) | Rust / `cargo test` (optionally `cargo nextest`) | `cargo test -p <crate>` for surface-narrow; `cargo test --workspace` for full | `--format=json` (unstable) or `cargo nextest run --message-format libtest-json` (preferred) |
| `package.json` with `scripts.test` | Node — read scripts to identify Jest / Vitest / Mocha / Playwright / Cypress / Storybook test | `pnpm test` (or `npm test` / `yarn test`) for the script named in `scripts.test`; surface-specific scripts (`test:unit`, `test:integration`, `test:visual`, `test:e2e`) when present | Jest / Vitest: `--reporter=json`; Mocha: `--reporter=json`; Playwright: `--reporter=json,list`; Cypress: `--reporter=mocha-junit-reporter` |
| `pyproject.toml` or `setup.cfg` / `setup.py` | Python — pytest / unittest / tox | `pytest -q` (surface-narrow with `-k <pattern>`); `tox -e <env>` when tox is present | `pytest --json-report` (via `pytest-json-report`) or `--junitxml=<file>` |
| `go.mod` | Go — `go test` | `go test ./<pkg>/...` for surface-narrow; `go test ./...` for full | `go test -json` |
| `*.sln` / `*.csproj` (and no Unity ProjectSettings) | .NET — `dotnet test` | `dotnet test <Project.csproj>` for surface-narrow; `dotnet test <Solution.sln>` for full | `--logger "trx;LogFileName=test_results.trx"` or `--logger "console;verbosity=detailed"` |
| `ProjectSettings/ProjectVersion.txt` (presence) | Unity — Editor batch test runner | `Unity -batchmode -runTests -projectPath <path> -testPlatform <EditMode\|PlayMode> -testResults <result.xml> -quit` | NUnit XML at `-testResults` |
| `Gemfile` | Ruby — RSpec / Minitest | `bundle exec rspec` or `bundle exec rake test` | `--format json` (RSpec) |
| `pom.xml` | Java / Kotlin — Maven Surefire / Failsafe | `mvn test` (unit), `mvn verify` (integration) | Surefire writes `target/surefire-reports/*.xml` (JUnit XML) |
| `build.gradle` / `build.gradle.kts` | Gradle — JUnit / Spek | `gradle test` or `gradle check` | `--scan` for build scans; per-task JUnit XML under `build/test-results/` |
| `Makefile` exposing a `test` target | Project-defined wrapper | `make test` (also check for `make verify` / `make check`) | Depends on wrapped runner |
| `pubspec.yaml` | Flutter / Dart | `flutter test` (unit) / `flutter test integration_test` | `--reporter json` |
| `composer.json` | PHP — PHPUnit / Pest | `vendor/bin/phpunit` or `vendor/bin/pest` | `--log-junit` |
| Standalone `tsconfig.json` without `package.json` (rare) | Likely embedded inside another project — defer to that project's runner | — | — |
| No recognized manifest | Fallback: read the project's AGENTS.md / README for instructions; otherwise emit `failed: tooling-missing` with rationale | — | — |

## Selection rules

1. **Multi-runner projects.** A workspace may declare more than one
   manifest (e.g. `Cargo.toml` + `package.json`). Detect all of them, then
   assign each changed surface to the runner whose manifest owns the
   surface's path. For example, in a Rust workspace with a pnpm-driven
   `web/` subdirectory, web changes go to pnpm-driven runners and Rust
   crate changes go to cargo.

2. **Prefer scripts over inferred conventions.** If `package.json`
   declares `scripts.test` and surface-specific scripts (`test:unit`,
   `test:integration`, `test:visual`, `test:release-*`), use the script
   names as they appear instead of guessing. Same for `Makefile` targets.

3. **Visual / UI runners.** Playwright / Cypress / Selenium /
   WinAppDriver / Unity Editor headed are only invoked when a UI surface
   is in scope. Even if the manifest is present, do not invoke a UI
   runner for non-UI changes.

4. **Long-running runners and `--mode quick`.** Some runners (Unity Editor
   batch, large `mvn verify`, full `dotnet test` solution) take many
   minutes. In `--mode quick`, narrow the invocation to a single
   project / module / `-k` filter relevant to the changed surface, or
   skip the runner with a recorded `skipped(quick-mode-budget)` reason.

5. **Plugin / module testing in Unity.** If the project ships under
   `Packages/<id>` Unity package layout, prefer scoping the test runner
   to that package's assemblies (`-testCategory` / `-testFilter`) instead
   of running the entire Editor test suite.

6. **Custom wrappers.** When the project defines a `make verify` /
   `make check` / `scripts/verify.sh` / `bin/check` wrapper, that wrapper
   is the canonical entry point. Use it instead of bypassing it; the
   wrapper exists precisely so contributors do not have to remember the
   runner-specific flags.

7. **Generic fallback.** When no manifest is recognized, surface the
   uncertainty: emit `failed: tooling-missing — no recognized test runner
   manifest found at <project root>` and stop. Do not invent commands.

## Inventory extraction hints

The `Test Inventory` section of the evidence bundle lists the actual
test names / scenarios / suite paths the runner executed. To populate it,
prefer the runner's structured output flag (the rightmost column in the
table above) and parse the following fields, in this order of preference:

1. Test identifier — fully qualified name when available
   (`module::path::test_name` for cargo; `describe > it` for Jest /
   Vitest / Mocha / Playwright; `ClassName.MethodName` for JUnit /
   NUnit / xUnit).
2. Outcome (`PASS` / `FAIL` / `SKIP`).
3. One-line failure summary (first line of the runner's error message
   when `FAIL`).
4. Skip reason when `SKIP`.

When extraction is not feasible (no structured output, parsing failure,
custom wrapper that swallows stdout), record
`(inventory unavailable: <reason>)` for that command and continue. Do not
silently degrade.
