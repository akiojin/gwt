# Playwright headed trace fixtures

These are excerpts from real Chromium runs on Windows, generated on 2026-11-05
for Issue #6726. They are not hand-written trace events.

| Files | Playwright | Trace version | Chromium revision |
| --- | --- | --- | --- |
| `playwright-v8-{dark,light}.trace` | 1.58.2 | 8 | 1208 |
| `playwright-v10-{dark,light}.trace` | 1.64.1 | 10 | 1248 |

Each version ran two projects, with `browserName: chromium`, `headless: false`,
`trace: on`, and respectively `colorScheme: dark` and `colorScheme: light`.
The only test used `page.setContent` to insert a button containing `trace fixture`
and asserted the button text. No application, network endpoint, account,
credential, browser profile, or external project state was used. Both versions
reported two passing tests.

Generation used the matching package's CLI:

```text
node <playwright-package>/cli.js test --config <temporary>/playwright.config.cjs --output <temporary>/results --reporter=list
```

Each excerpt preserves the test-runner header, the actual browser-launch
`before` event and its successful matching `after` event, and the Chromium
library context header from the resulting trace ZIP. The v8 `params.tracesDir`
was replaced with `<fixture-output>/traces`; no other event fields were changed.
Unrelated actions, stack frames, snapshots, source files, and resources were
omitted. The full ZIPs are local generation artifacts, not repository fixtures.

The v8 launch uses `class: Test`, `method: pw:api`, and `title: Launch browser`
instead of `apiName`, while retaining `params.headless: "false"`. In v10 the
launch parameters are empty and the library header no longer has `contextId`.
The v10 header also does not necessarily include a user agent. Therefore v10
trace records alone cannot prove the actual browser's headed setting; a runtime
measurement is needed rather than inferring it from project configuration.

These fixtures contain only a synthetic blank-page test. They were inspected
for absolute user paths, application data, credentials, and network endpoints.
