# Real provider hook payloads

Each file is the raw hook stdin captured from a real provider session.
The path records its origin: `<provider>/<provider version>/<Event>.json`.
`hook_payload_fixtures_test.rs` passes every file through the hook parser
entry and checks that secret values are scrubbed.

Rules:

- Never add a hand-written payload.
- Replace session and prompt ids with `00000000-0000-0000-0000-00000000000N`.
- Rewrite absolute paths under `/redacted/`.
- Add fixtures when a provider joins the launch candidate pool.

## Capture log

| Provider | Version | Host | Events | Issue |
| -------- | ------- | ---- | ------ | ----- |
| grok-build | 1.0.92 (b06756fd6256) | macOS arm64, isolated `GROK_HOME` | UserPromptSubmit, Stop | #4779 |
