# Tooling Bootstrap (project-agnostic)

`gwt-verify` checks that the test runners chosen via `runner-detection.md`
are actually installed before invoking them. When a runner is missing, the
skill follows a bounded best-effort auto-install path. If the auto-install
path is exhausted, the skill records `failed: tooling-missing: <component>`
in the evidence bundle. Callers (`gwt-build-spec` Phase 3,
`gwt-manage-pr` Pre-PR) treat that as a hard completion blocker.

This contract is the same regardless of project language. The auto-install
attempts and detection heuristics differ per runner but the *shape* of the
contract (single attempt, no sudo, no OS package managers, deterministic
failure mode) is identical.

## Auto-install Contract (per runner family)

The skill attempts the install paths in the rightmost column at most once
per invocation. On failure, `failed: tooling-missing: <component>` is
recorded. The component name should be the canonical runner identifier so
callers can react.

| Runner family | Missing tool detection | Best-effort install attempt | On failure |
|---|---|---|---|
| Node-based (npm / pnpm / yarn scripts) | `command -v <pm>` fails; or `node_modules` is absent | For pnpm: `corepack enable pnpm` then `pnpm install --frozen-lockfile`. For yarn: `corepack enable yarn` then `yarn install --frozen-lockfile`. For npm: `npm ci`. | `failed: tooling-missing: <pm>` or `<pm>-dependencies` |
| Browser UI runner (Playwright / Cypress / WebDriver-based) | Runner CLI present but browsers / drivers absent | Playwright: `pnpm exec playwright install --with-deps <browser>`. Cypress: `pnpm exec cypress install`. WebDriver: rely on the project's own setup script if present. | `failed: tooling-missing: <runner>-browsers` |
| Rust (`cargo`) | `command -v cargo` fails | — (not auto-installed; this is a developer environment requirement) | `failed: tooling-missing: cargo` |
| Python (`pytest`, `tox`, runner declared in `pyproject.toml`) | `python -c "import <runner>"` fails | When a `requirements*.txt` / `pyproject.toml` declares dev dependencies, attempt `python -m pip install -r requirements-dev.txt` *only when running inside an active virtualenv*. Never install into a non-venv interpreter. | `failed: tooling-missing: <runner>` |
| Go (`go test`) | `command -v go` fails | — | `failed: tooling-missing: go` |
| .NET (`dotnet test`) | `command -v dotnet` fails, or `dotnet --list-sdks` is empty | — | `failed: tooling-missing: dotnet-sdk` |
| Unity Editor batch runner | The detected Unity Editor executable is missing or version mismatch with `ProjectSettings/ProjectVersion.txt` | — (Unity Editor is provisioned out-of-band) | `failed: tooling-missing: unity-editor-<version>` |
| Ruby (`bundle exec rspec`) | `command -v bundle` fails | — | `failed: tooling-missing: bundler` |
| Java / Kotlin (`mvn` / `gradle`) | `command -v mvn` / `command -v gradle` fails | — | `failed: tooling-missing: maven` / `gradle` |
| Lint / docs (`markdownlint`, etc.) | `command -v markdownlint` fails | — | `skipped: markdownlint-not-installed` (non-blocking) |
| `git` | `git rev-parse --show-toplevel` fails | — | `failed: tooling-missing: git` |

## Guardrails

- **No sudo.** The skill must not invoke `sudo` or any privilege escalation.
- **No OS package managers.** Do not call `apt`, `brew`, `pacman`,
  `dnf`, `pkg`, `pip` (outside an active virtualenv), `choco`, `winget`.
- **No global mutations.** Installations target the worktree
  (`node_modules`, `Pipfile.lock`-managed venvs, browser caches under
  the standard per-user cache directories) and must not modify other
  repositories.
- **Cache reuse.** When the runner's browser / driver / package cache
  already contains the needed version, the skill reuses it instead of
  forcing a reinstall.
- **No retries.** Each auto-install attempt runs at most once per
  `gwt-verify` invocation. Repeated transient failures escalate to
  `failed: tooling-missing`.
- **Project wrapper precedence.** When the project ships its own setup
  wrapper (`make setup`, `scripts/bootstrap.sh`, `bin/setup`,
  `package.json` `scripts.bootstrap`), prefer it over invoking the
  runner-specific install path.

## Evidence Bundle Shape on Failure

When `failed: tooling-missing` is produced, the evidence bundle MUST
include the per-attempt status so the caller can distinguish "the
project's tests are broken" from "the local environment isn't ready to
run any tests at all":

```text
Tooling installed during run:
- pnpm install --frozen-lockfile: FAIL (exit code <n>)
- pnpm exec playwright install --with-deps chromium: not attempted (blocked by previous failure)
Overall: FAIL
failed: tooling-missing: <component>
```

## Detection Heuristics (recap)

| Tool | Detection |
|---|---|
| `pnpm` / `npm` / `yarn` | `command -v <pm>` succeeds and `<pm> --version` returns a string. |
| Node `node_modules` | `package.json` exists AND a marker file (`node_modules/.modules.yaml` for pnpm, `node_modules/.package-lock.json` for npm, `.yarn/install-state.gz` for yarn) matches the lockfile. When in doubt, run the project's frozen-lockfile install (idempotent). |
| Playwright browsers | `pnpm exec playwright --version` succeeds AND the bundled `browsers.json` version matches the cache under `~/.cache/ms-playwright` / `~/Library/Caches/ms-playwright` / `%LOCALAPPDATA%\\ms-playwright`. When in doubt, dry-run `pnpm exec playwright install --with-deps <browser> --dry-run`. |
| Cypress browsers | `pnpm exec cypress info` succeeds and lists at least one browser. |
| `cargo` / `rustc` | `command -v cargo` succeeds. |
| `python` runner libraries | `python -c "import <runner>"` succeeds inside the project's interpreter. |
| `go` | `command -v go` and `go version` succeed. |
| `dotnet` | `command -v dotnet` and `dotnet --list-sdks` returns ≥ 1 line. |
| Unity Editor | The path resolved by the project's `ProjectSettings/ProjectVersion.txt` exists and reports the expected version when run with `-version`. |
| `git` | `git rev-parse --show-toplevel` succeeds inside the worktree. |
| `markdownlint` | `command -v markdownlint` (or `npx markdownlint --version`) succeeds. |

## Container / CI Notes

In container environments where caches may be pre-baked (Playwright
browsers, `node_modules`, `~/.cargo`, Go module cache, Unity Editor
install), the detection heuristics above are the primary path; the
auto-install attempts should short-circuit immediately when the dry-run
shows everything is present.

If the environment is hermetic (no internet), the auto-install attempts
fail fast and the skill records `failed: tooling-missing` with the
network error preserved verbatim. The caller may either:

- Treat it as a hard block (default behavior in `gwt-build-spec` Phase 3).
- Override with an explicit `gwtd build abort --reason ...` and have the
  user run the tests on a network-reachable host.
