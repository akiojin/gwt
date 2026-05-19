# Surface → Test Matrix (canonical)

This is the single source of truth for `gwt-verify` surface classification.
The skill must not invoke Playwright for any row whose "Browser (Playwright)"
column is empty — that contract comes from SPEC-1935 FR-124 and was reinforced
by the user during the gwt-discussion that produced Phase 17.

## Matrix

| Surface (changed path pattern) | Non-browser execution | Browser (Playwright) |
|---|---|---|
| `crates/*/src/**`, `crates/*/tests/**` | `cargo test -p <crate>`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt -- --check` | — |
| `crates/gwt/src/bin/gwtd*`, `crates/gwt/src/bin/gwt*` | cargo (as above) + bin smoke (`cargo run -p gwt --bin gwtd -- --version`) | — |
| `crates/gwt/web/**/*.js` (logic only — no UI surface) | `pnpm test:frontend-bundle`, `pnpm test:frontend-unit`, `pnpm test:frontend-smoke` | — |
| `crates/gwt/web/**` (UI / CSS / HTML / theme assets) | as above | `pnpm test:visual` |
| `crates/gwt/playwright/**` | — | `pnpm test:visual` |
| `scripts/check-release-flow.sh`, `scripts/test_release_assets.cjs`, release scripts | `pnpm test:release-flow`, `pnpm test:release-assets` | — |
| `.claude/skills/**`, `.codex/skills/**`, `crates/gwt-skills/**` | `pnpm lint:skills`, `cargo test -p gwt-skills` | — |
| `AGENTS.md`, `CLAUDE.md`, `README*.md`, `docs/**` | `markdownlint` (when available) | — |

## Selection Rules

1. **Multiple surfaces match** — run the union of all matched commands. Skip
   duplicates (e.g. only run `cargo clippy --all-targets` once per invocation).
2. **Browser column is empty → do not invoke Playwright.** This is the user's
   strong constraint: ブラウザ起動前提のシステムだけが Playwright 対象.
3. **`crates/gwt/web/**` ambiguity** — if any of `*.html`, `*.css`,
   `theme-*.js`, or visible UI assets changed, treat the surface as "UI / CSS
   / HTML" and include `pnpm test:visual`. If only non-visual JS logic
   changed, exclude `pnpm test:visual`.
4. **`docs/**` only** — markdownlint is non-blocking when markdownlint is
   absent on the system; the evidence bundle records this as `skipped:
   markdownlint-not-installed`.
5. **No changed files** — evidence bundle is `Overall: PASS` with
   `Changed surfaces: (none)` and no executed commands. The caller decides
   whether that means "nothing to verify" or "baseline drift".

## Mode → Subset

| Mode | Behavior |
|---|---|
| `quick` | Run the narrowest representative command per matched surface (e.g. `cargo test -p <single crate>`, `pnpm test:frontend-unit` only — skip bundle/smoke). Skip Playwright entirely unless the working tree explicitly carries a `.spec.ts` change under `crates/gwt/playwright/tests/**`. |
| `full` | Run the full matched list per surface, including `pnpm test:visual` headless when the Browser column is non-empty. |
| `pre-pr` | `full` plus release-flow tests whenever any release-system file is in the baseline diff. Visual regression is always included if any UI surface changed (no `quick` fallback). |
| `--headed` | When a matched command would invoke Playwright, run it with `pnpm test:visual --headed` so Chromium starts headed. Default is headless to match CI. |

## Non-Goals

- `gwt-verify` does **not** mutate snapshots. Snapshot baseline updates
  remain a manual `pnpm test:visual --update-snapshots` step outside the
  skill (caller must run it explicitly with intent).
- `gwt-verify` does **not** chase external CI. The skill answers "is the
  local workspace in a verifiable state right now?" — CI verdicts are still
  the responsibility of `gwt-manage-pr`.
