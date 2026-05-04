# Playwright — SPEC-2356 Operator Design System

Visual regression baseline for the Operator Design System surfaces.

## Run

```bash
# from repo root
npm run test:visual
```

## Update baseline (when intentional design change lands)

```bash
npx playwright test --update-snapshots --config crates/gwt/playwright/playwright.config.ts
```

## Test layout

| Spec | カバー範囲 |
|---|---|
| `tests/chrome.spec.ts` | Project Bar / Status Strip / Sidebar Layers / Drawer |
| `tests/command-palette.spec.ts` | ⌘P 開閉、fuzzy filter、Enter 実行 |
| `tests/living-telemetry.spec.ts` | active/idle/blocked 遷移、pulse rim、counter sync |
| `tests/theme-toggle.spec.ts` | Dark↔Light 200ms 切替、xterm 追従 |
| `tests/mission-briefing.spec.ts` | 起動 splash、reduced-motion 縮退 |
| `tests/reduced-motion.spec.ts` | Living Telemetry 縮退 |
| `tests/forced-colors.spec.ts` | forced-colors fallback |
| `tests/adoption-surfaces.spec.ts` | 各サーフェス × Dark/Light スナップショット |
