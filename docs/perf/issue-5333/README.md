# Issues terminology verification

These six screenshots were captured on 2026-11-10 from this checkout's `target/debug/gwt`, using a fresh HOME with automatic agents disabled. The isolated profile has no saved agents or GitHub token, so the empty queue and unavailable repository-label lookup are expected fixture state.

The headed Chromium check passed in dark and light themes, asserting the Issues status label, Settings navigation, notification title and issue number, and zero console/page errors.

Reproduce after building both checkout binaries:

```sh
cargo build -p gwt --bin gwt --bin gwtd
node crates/gwt/playwright/verify-issue-6769.mjs toast-host.spec.ts --grep 'Issue #5333' --trace=off
```

The existing isolated runner is reused. Optional Playwright tracing is disabled because trace finalization timed out after the assertions and left incomplete ZIP files with the installed runner; screenshots and all behavioral/error assertions remain enabled.

| Surface | Dark | Light |
| --- | --- | --- |
| Issues | [Dark](issues-naming-issues-dark.png) | [Light](issues-naming-issues-light.png) |
| Settings | [Dark](issues-naming-settings-dark.png) | [Light](issues-naming-settings-light.png) |
| Notifications | [Dark](issues-naming-notifications-dark.png) | [Light](issues-naming-notifications-light.png) |
