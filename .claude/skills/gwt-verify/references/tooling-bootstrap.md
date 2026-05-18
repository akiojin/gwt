# Tooling Bootstrap

`gwt-verify` checks for required tooling before running the selected
commands. Missing tools follow a bounded auto-install path. When the
auto-install path is exhausted, the skill records `failed: tooling-missing`
in the evidence bundle and the caller (typically `gwt-build-spec` Phase 3)
treats that as a hard completion blocker.

## Auto-install Contract

| Missing | Auto-install attempt | On failure |
|---|---|---|
| `pnpm` not on PATH | `corepack enable pnpm` | `failed: tooling-missing: pnpm` |
| `node_modules` absent or stale | `pnpm install --frozen-lockfile` | `failed: tooling-missing: node_modules` |
| Playwright Chromium browser absent | `pnpm exec playwright install --with-deps chromium` | `failed: tooling-missing: playwright-chromium` |
| `cargo` not on PATH | — (pre-existing project requirement) | `failed: tooling-missing: cargo` |
| `git` not on PATH or repo not initialized | — | `failed: tooling-missing: git` |
| `markdownlint` not installed (for docs-only runs) | — | `skipped: markdownlint-not-installed` (non-blocking) |

## Guardrails

- **No sudo.** The skill must not invoke `sudo` or any privilege escalation.
- **No OS package managers.** Do not call `apt`, `brew`, `pacman`,
  `dnf`, `pkg`, `pip`, `choco`, `winget`. The contract is limited to
  `corepack`, `pnpm install`, and `pnpm exec playwright install`.
- **No global mutations.** Installations target the worktree (`node_modules`,
  Playwright cache under `~/.cache/ms-playwright` / `~/Library/Caches/ms-playwright`)
  and must not modify other repositories.
- **Cache reuse.** When the Playwright cache already contains the needed
  Chromium build, the skill must reuse it instead of forcing a reinstall.
- **No retries.** Each auto-install attempt runs at most once per
  `gwt-verify` invocation. Repeated transient failures escalate to
  `failed: tooling-missing`.

## Evidence Bundle Shape on Failure

When `failed: tooling-missing` is produced, the evidence bundle MUST include:

```text
Tooling installed during run:
- pnpm install --frozen-lockfile: FAIL (exit code <n>)
- pnpm exec playwright install --with-deps chromium: not attempted (blocked by previous failure)
Overall: FAIL
failed: tooling-missing: <component>
```

So that the caller can distinguish "the project's tests are broken" from
"the local environment isn't ready to run any tests at all".

## Detection Heuristics

| Tool | Detection |
|---|---|
| `pnpm` | `command -v pnpm` succeeds and `pnpm --version` returns a string. |
| `node_modules` | `package.json` exists AND `.pnpm-lock-hash` (if present) matches the lockfile, OR a marker file under `node_modules/.modules.yaml` matches the lockfile hash. Heuristic — when in doubt, run `pnpm install --frozen-lockfile` (idempotent when already satisfied). |
| Playwright Chromium | `pnpm exec playwright --version` succeeds AND the Chromium revision listed in the Playwright version's bundled `browsers.json` is present under the OS-specific Playwright cache. Heuristic — when in doubt, run `pnpm exec playwright install --with-deps chromium --dry-run` and look for "browsers already installed". |
| `cargo` | `command -v cargo` succeeds. |
| `git` | `git rev-parse --show-toplevel` succeeds inside the worktree. |
| `markdownlint` | `command -v markdownlint` (or `npx markdownlint --version`) succeeds. |

## Container / CI Notes

In container environments where the Playwright Chromium cache may be
pre-baked, the detection heuristic for "browsers already installed" is the
primary path; the auto-install attempt should short-circuit immediately when
the dry-run shows everything present.

If the environment is hermetic (no internet), the auto-install attempts will
fail fast and the skill records `failed: tooling-missing` with the network
error preserved verbatim in the evidence bundle. The caller may either:

- Treat it as a hard block (default behavior in `gwt-build-spec` Phase 3).
- Override with an explicit `gwtd build abort --reason ...` and have the
  user run the tests on a network-reachable host.
