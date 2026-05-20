# Surface Taxonomy (project-agnostic)

`gwt-verify` classifies every changed file into one of the abstract surfaces
below. The surface determines (a) which test runners are relevant and (b)
whether the User Verification Handoff is Required, Recommended, or Skipped.

This taxonomy is intentionally project-agnostic. The patterns column shows
**typical** signals for common project types; treat them as hints, not as
exhaustive rules. If the host project's AGENTS.md / README defines a
different classification, that wins.

## Abstract Surfaces

| Surface | Typical signals (any of) | User Check |
|---|---|---|
| **UI surface** — anything the end user sees rendered: markup, theme, layout, interactive controls, motion, accessibility affordances. | `*.html`, `*.css`, `*.scss`, `*.tsx` / `*.jsx` rendered components, `theme*.{js,ts}`, Unity `Assets/**/*.{prefab,uxml,uss,asset}` for UI, `.NET` XAML / WinForms designer files, Flutter `*.dart` widget files, Qt `*.ui` files, web assets under any `web/**`, `frontend/**`, `client/**`, `ui/**`, `static/**`. | **Required** |
| **Interactive CLI / TUI surface** — a command-line or terminal UI a human invokes directly. | CLI entry points (`src/bin/*.rs`, `cmd/**/main.go`, `bin/*`, `scripts/cli.*`), ratatui / blessed / urwid / Bubble Tea TUI files, argparse / clap / cobra / commander definitions when the diff changes user-visible flags, help text, or output formatting. | **Required** |
| **Business logic (no observable surface)** — pure functions, services, parsers, models, computation. | Source under `src/**` / `lib/**` / `internal/**` / `crates/**` excluding UI and CLI entry points; backend services without UI rendering. | **Recommended** |
| **Build / Release pipeline** — code that produces or publishes the distributable. | `Dockerfile*`, `Containerfile`, GitHub Actions `release` workflows, `scripts/release-*.{sh,cjs,ps1}`, `goreleaser.yaml`, `Cargo.toml` `[package]` version bumps, `package.json` version bumps, installer specs (`*.iss`, `*.nsi`, `*.wxs`), Unity build scripts. | **Required** |
| **External binding / 3rd-party integration** — code that crosses a network or process boundary to a service we don't own. | HTTP / gRPC client modules, SDK wrappers, message-broker producers / consumers, OAuth flows, webhook senders. | **Recommended** |
| **Skill asset / agent config** — agent skills, prompts, hooks, slash-command files. | `.claude/skills/**`, `.codex/skills/**`, `.claude/commands/**`, `.gemini/**`, `.cursor/**`, `AGENTS.md` sections specifically describing agent behavior. | **Recommended** |
| **Docs / config-only** — documentation, formatting, dependency bumps with no behavior change. | `*.md` outside skill assets, `*.txt`, `LICENSE*`, `CHANGELOG.md`, `.editorconfig`, lint config (`.eslintrc*`, `clippy.toml`), formatter config, no-op dependency bumps. | **Skipped(docs-only)** |

## Selection Rules

1. **A file may match multiple surfaces.** Pick the most user-visible
   surface (UI > CLI > business logic > docs).
2. **`ui` / `web` / `frontend` / `client` directories** default to UI surface
   regardless of file extension when the diff touches files the user
   ultimately renders. Pure build-config files inside those directories
   (e.g. `webpack.config.js`) classify as Build / Release pipeline.
3. **CLI entry-point logic changes** classify as Interactive CLI surface only
   when the user-visible behavior (flags, output, error messages) changes.
   Pure internal refactors of the CLI plumbing classify as Business logic.
4. **Skill assets do not require Playwright** — they are agent-facing only.
   Their User Check is Recommended (trigger sanity check), not Required.
5. **Multiple surfaces → take the highest user-check tier across all
   matched surfaces.** Required > Recommended > Skipped.
6. **No changed files** → `Changed surfaces: (none)` and User Verification
   is `skipped(no-change)`.

## Per-project specialization hints

Concrete projects may need finer-grained mappings. The host project's
AGENTS.md may publish its own table; in that case, prefer the project table
and use this taxonomy only as a fallback for files the project table does
not mention. Example: the gwt repo's own example matrix lives in
`test-matrix.md` alongside this file.

For Unity projects, treat any change under `Assets/**` that is referenced by
a `Scene` or a `Canvas` as UI surface even when the file is a `.cs` script;
the rendered behavior is user-visible. Pure `Editor/**` scripts that only
affect the Unity Editor classify as Build / Release pipeline.

For .NET projects with WPF / WinForms, `*.xaml`, `*.Designer.cs`, and any
code-behind files for windows / pages classify as UI surface. Backend
service projects without windows classify as Business logic.

For Web / SPA projects, route definitions, layout shells, and shared
component libraries classify as UI surface. API route handlers without
rendering classify as Business logic; if they expose schema visible to a
human (OpenAPI specs, error responses), bump to External binding.

## Non-Goals

- This taxonomy does not pick the runner. That is `runner-detection.md`.
- This taxonomy does not regenerate snapshots. Visual regression is FAIL,
  not auto-update.
- This taxonomy does not enumerate every file pattern; it gives enough
  signal for the agent to classify confidently.
