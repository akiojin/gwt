# Test Matrix (worked example for the gwt repo)

`gwt-verify` is project-agnostic. The canonical surface classification
lives in `surface-taxonomy.md` and the manifest-to-runner mapping lives
in `runner-detection.md`. This file documents the **gwt repository's
own** specialization of that contract as a worked example. Other
projects (Unity, .NET, Python, Go, Java, …) should document their own
matrix in their project AGENTS.md / README and rely on the generic
contract here as a fallback frame.

When agents run `gwt-verify` inside the gwt repository, this matrix is
the project-specific recipe.

## Project: gwt repo (Rust workspace + pnpm-driven WebView frontend)

### Detected runners

- `Cargo.toml` workspace at the repo root → `cargo test`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo fmt -- --check`.
- `package.json` at `crates/gwt/web/` (and root) → `pnpm` scripts
  including `test:frontend-bundle`, `test:frontend-unit`,
  `test:frontend-smoke`, `test:visual`, `test:release-flow`,
  `test:release-assets`, `lint:skills`.
- `crates/gwt/playwright/playwright.config.ts` → Playwright project
  for WebView visual / interaction tests.

### Surface → Command matrix (gwt repo specialization)

| Surface (changed path pattern) | Non-browser execution | Browser (Playwright) | User Check |
|---|---|---|---|
| `crates/*/src/**`, `crates/*/tests/**` | `cargo test -p <crate>`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt -- --check` | — | Recommended (business logic) |
| `crates/gwt/src/bin/gwtd*`, `crates/gwt/src/bin/gwt*` | cargo (as above) + bin smoke (`cargo run -p gwt --bin gwtd -- --version`) | — | Required (interactive CLI) |
| `crates/gwt/web/**/*.js` (logic only — no UI surface) | `pnpm test:frontend-bundle`, `pnpm test:frontend-unit`, `pnpm test:frontend-smoke` | — | Recommended (business logic) |
| `crates/gwt/web/**` (UI / CSS / HTML / theme assets) | as above | `pnpm test:visual` | **Required** (UI surface) |
| `crates/gwt/playwright/**` | — | `pnpm test:visual` | **Required** (UI surface) |
| `scripts/check-release-flow.sh`, `scripts/test_release_assets.cjs`, release scripts | `pnpm test:release-flow`, `pnpm test:release-assets` | — | **Required** (Build / Release pipeline) |
| `.claude/skills/**`, `.codex/skills/**`, `crates/gwt-skills/**` | `pnpm lint:skills`, `cargo test -p gwt-skills` | — | Recommended (skill asset) |
| `AGENTS.md`, `CLAUDE.md`, `README*.md`, `docs/**` | `markdownlint` (when available) | — | Skipped(docs-only) |

### Selection Rules (gwt repo)

1. **Multiple surfaces match** — run the union of all matched commands.
   Skip duplicates (e.g. only run `cargo clippy --all-targets` once per
   invocation).
2. **Browser column is empty → do not invoke Playwright** unless
   acceptance-aware escalation (`surface-taxonomy.md`) promotes a
   backend-only diff to a UI surface because its acceptance manifests in
   the WebView. Only WebView / browser UI surfaces — by diff or by
   acceptance — are eligible for Playwright. The same rule applies to any
   other UI runner (Cypress / Selenium / WinAppDriver) in future-arriving
   projects. This is part of the gwt-verify verification contract that
   prevents over-eager Playwright invocation, not an external SPEC
   requirement.
3. **`crates/gwt/web/**` ambiguity** — if any of `*.html`, `*.css`,
   `theme-*.js`, or visible UI assets changed, treat the surface as
   "UI / CSS / HTML" and include `pnpm test:visual`. If only non-visual
   JS logic changed, exclude `pnpm test:visual`.
4. **`docs/**` only** — markdownlint is non-blocking when markdownlint
   is absent on the system; the evidence bundle records this as
   `skipped: markdownlint-not-installed`.
5. **No changed files** — evidence bundle is `Overall: PASS` with
   `Changed surfaces: (none)` and no executed commands. User
   Verification is `skipped(no-change)`. The caller decides whether
   that means "nothing to verify" or "baseline drift".
6. **WebView-driving backend seam (acceptance-aware).** Changes under
   `crates/gwt/src/**` (the server / WebSocket protocol / app runtime that
   renders into the WebView) classify as business logic by path. But when
   the Issue / repro acceptance is a WebView behavior, acceptance-aware
   escalation (`surface-taxonomy.md`) promotes them to UI surface and
   `pnpm test:visual` is included. A pure-backend change here with no
   WebView acceptance keeps the business-logic classification (no
   Playwright).

### Worked example: acceptance-aware escalation (gwt repo)

A bug fix lands only in `crates/gwt/src/app_runtime.rs` (no `web/**` file
changed), but the Issue says "the workspace list fails to re-render after a
viewport update." The acceptance is a WebView behavior. By diff path this is
business logic (Browser column empty), but acceptance-aware escalation
promotes it to UI surface:

```text
Changed surfaces: business logic (crates/gwt/src/**)
Acceptance Surface: UI surface (escalated — repro is a WebView re-render)
Executed: cargo test -p gwt, pnpm test:visual
User Verification: required
```

The reverse holds too: a pure refactor under `crates/gwt/src/**` with no
WebView-visible acceptance records `Acceptance Surface:
non-user-facing(internal refactor, no rendered behavior change)` and skips
`pnpm test:visual`.

### Mode → Subset (gwt repo)

| Mode | Behavior |
|---|---|
| `quick` | Run the narrowest representative command per matched surface (e.g. `cargo test -p <single crate>`, `pnpm test:frontend-unit` only — skip bundle/smoke). Skip Playwright entirely unless the working tree explicitly carries a `.spec.ts` change under `crates/gwt/playwright/tests/**`. User Verification is `skipped(quick-mode)`. |
| `full` | Run the full matched list per surface, including `pnpm test:visual` headless when the Browser column is non-empty. User Verification is required for Required surfaces. |
| `pre-pr` | `full` plus release-flow tests whenever any release-system file is in the baseline diff. Visual regression is always included if any UI surface changed (no `quick` fallback). User Verification is required for Required surfaces. |
| `--headed` | When a matched command would invoke Playwright, run it with `pnpm test:visual --headed` so Chromium starts headed. Default is headless to match CI. |

### Non-Goals (gwt repo)

- `gwt-verify` does **not** mutate snapshots. Snapshot baseline updates
  remain a manual `pnpm test:visual --update-snapshots` step outside the
  skill (caller must run it explicitly with intent).
- `gwt-verify` does **not** chase external CI. The skill answers "is the
  local workspace in a verifiable state right now?" — CI verdicts are
  still the responsibility of `gwt-manage-pr`.

## Adapting this matrix for another project

The matrix above is the gwt repo's specialization. To bring `gwt-verify`
to a different project type, document a similar matrix in that project's
AGENTS.md / README that:

1. Lists the project's detected runners (per `runner-detection.md`).
2. Maps each `surface-taxonomy.md` surface to the runner commands that
   exercise it.
3. Marks the User Check tier per surface (Required / Recommended /
   Skipped).
4. Describes mode-specific subsets if the project benefits from
   `quick` vs `full` vs `pre-pr` narrowing.

The skill follows the project's matrix when present; otherwise it
relies on `surface-taxonomy.md` + `runner-detection.md` as the
fallback frame.
