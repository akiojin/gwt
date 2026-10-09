---
name: gwt-feedback
description: "Use when a PM or implementation agent encounters a gwt bug, missing capability, or workflow friction in any project. Do not use for defects in the project being developed."
---

# gwt-feedback

Report problems in gwt to the fixed `akiojin/gwt` destination through
`feedback.create`. PM and implementation agents use the same path; a Board
proposal or PM registration is not a prerequisite.

## Collect the report

- `title`: a concrete symptom; `kind`: `bug`, `request`, or `friction`.
- `operation`: the affected gwtd operation or precise UI action.
- `refusal`: the complete observed rejection/output, copied verbatim. Preserve
  newlines and whitespace; never summarize, trim, or truncate it. Use an empty
  string if no refusal was observed. An error-ledger summary may be truncated:
  recover the original command output rather than presenting that summary as
  the original.
- `reproduction`: steps and relevant preconditions. Write `unconfirmed` when
  reproduction is unknown; do not invent a successful reproduction.
- `workaround`: `null` if none was used, otherwise describe what was done.
- Optional `body`: expected behavior, impact, supporting evidence, and any
  uncertainty about gwt's responsibility. Optional `error_record_id`: the
  corresponding `errors.list` record id, if known. Optional `labels`: relevant
  existing labels.

gwtd adds the source project hash, gwt/gwtd versions, OS, and active agent
count. Do not supply a destination, project identity, or environment override.

## Submit and continue working

Resolve `GWT_BIN` before running gwtd: executable `GWT_BIN_PATH`, then
`command -v gwtd`, then `$GWT_PROJECT_ROOT/target/debug/gwtd` or
`./target/debug/gwtd`. Use `"$GWT_BIN"` for each operation. If none exists,
retain the collected report and explain `gwtd not found`.

Pass the following JSON through stdin, replacing its evidence with the
actual observation. JSON escapes encode the original characters; they do not
permit editing the evidence.

```json
{"schema_version":1,"operation":"feedback.create","params":{"title":"Workspace update refuses the assigned session","kind":"friction","operation":"workspace.update","refusal":"authority recovery is blocked\nrequest was refused","reproduction":"unconfirmed; observed after relaunching the assigned session","workaround":null,"body":"The assigned session could not update its current focus."}}
```

The operation saves a durable local receipt. For other or unknown repositories,
it starts a detached approval worker and returns immediately. Continue the
project's work while feedback is queued or awaiting approval. Do not hold the
work, its completion, or its execution record open solely for feedback approval.
In the actual gwt repository, it sends synchronously and returns the Issue
number and URL.

Only the actual gwt repository is exempt from consent. Other and unknown
repositories require the user's explicit **Send feedback** action in an OS
native dialog before any Issue or comment is sent. Cancellation or an
unavailable desktop preserves the local report. Autonomous mode, JSON approval
flags, saved approvals, settings, and repo environment overrides cannot grant
consent. Do not approve the dialog on the user's behalf.

## Check the receipt

The JSON envelope's `output` contains a JSON receipt with `request_id`, `state`,
`issue_number`, `issue_url`, `receipt`, and `error`. Keep the request id and
local receipt path; pending feedback is not a successfully filed Issue. A
matching existing Issue may receive a comment instead of a new Issue.

```json
{"schema_version":1,"operation":"feedback.status","params":{"id":"67e44d8a-7903-4457-9e2d-51703a44c180"}}
```

Check status when the result is needed, without repeatedly polling or waiting
instead of doing project work. A sent receipt supplies the Issue number and
URL. Retry only a `declined` or `failed` receipt when another attempt is wanted;
an external-project retry still obtains fresh native consent.

```json
{"schema_version":1,"operation":"feedback.retry","params":{"id":"67e44d8a-7903-4457-9e2d-51703a44c180"}}
```

An `uncertain` result may already have reached GitHub. Preserve it and inspect
the existing receipt/Issue; never automatically resend it or submit a new
request to evade the retry refusal. Report the feedback result separately from
the project's actual blocker, and use the Board for local coordination when
needed.
