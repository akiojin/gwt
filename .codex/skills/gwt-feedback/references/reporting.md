# gwt-feedback

Report problems in gwt to the fixed `akiojin/gwt` destination through
`feedback.create`. PM and implementation agents use the same path; a Board
proposal or PM registration is not a prerequisite.

## Collect the report

With canonical **Autonomous ON**, gwtd discards all free text before durable
storage, fingerprinting, diagnostics or sending. The closed safe snapshot has
only a fixed schema/template, known operation/error enums, bounded numeric
package versions and OS/architecture enums. Unknown operations are withheld.
No additional feedback opt-in is required. Auto capacity is unrelated.
JSON flags, environment flags and repository settings cannot grant Auto authority.
Missing preferences or a normal false value retain Manual behavior. An actual
preference I/O/decode failure permits only safe local `withheld` storage, with
no worker, consent or send authority; its parse diagnostics are static too.

The collection and masking details below apply to Manual mode (missing/false preferences).
Collect the observed output and diagnostics yourself; do not ask the reporter
to retype them. Use `unconfirmed` for missing reproduction details. Do not dump
the environment or read unrelated private source files to enrich the report.

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
  existing labels to suggest in the structured report. Feedback does not assign
  GitHub labels, so ordinary reporters do not need repository push access.

gwtd adds the source project hash, gwt/gwtd versions, OS, and active agent
count. Do not supply a destination, project identity, or environment override.

For Manual mode (missing/false preferences), the durable local receipt retains the complete original, including sensitive
values. Do not publish that receipt. Before public submission, gwtd masks known
credentials, environment assignment values, path-shaped text (conservatively
including URLs), and Markdown inline/fenced/indented code. This applies to the
title and all report text, including requested labels and optional fields.
Assignment values are conservatively masked through the physical line end;
quoted values and escaped line continuations can span multiple lines.
Unmasked refusal bytes and original line endings are retained without truncation.

The native dialog shows the masked public snapshot; that same snapshot becomes
the GitHub Issue or comment. Only the original stays in the local receipt.
Automatic masks cannot identify every arbitrary secret or unmarked code fragment;
never promise complete detection or approve the preview for the user. The
reporter's only required interaction is the existing native consent action.
The actual gwt repository uses the same masks with its existing consent exemption.

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

The operation saves a durable local receipt. Auto always starts a detached
hidden worker and returns immediately, including in the actual gwt repository.
It never opens consent, preview, authentication or failure dialogs, browsers or
a worker console. Missing authentication or revoked/unreadable policy leaves
a static failed/withheld result; it never falls back to Manual consent. Auto
uses only existing process-provided GH_TOKEN/GITHUB_TOKEN credentials. It never
invokes gh auth token or a credential-store unlock; absence is auth_unavailable. A GitHub CLI login alone leaves Auto feedback unsent. Existing
raw receipts never become Auto reports when settings change.

With Manual mode (missing/false preferences), for other or unknown repositories,
it starts a detached approval worker and returns immediately. Continue the
project's work while feedback is queued or awaiting approval. Do not hold the
work, its completion, or its execution record open solely for feedback approval.
In the actual gwt repository, it sends synchronously and returns the Issue
number and URL.

In the Manual path, only the actual gwt repository is exempt from consent. Other and unknown
repositories require the user's explicit **Send feedback** action in an OS
native dialog before any Issue or comment is sent. Cancellation or an
unavailable desktop preserves the local report. JSON approval flags, saved
approvals and repo/environment overrides cannot grant consent or Auto authority.
Only the existing canonical Autonomous preference selects the safe Auto path;
do not approve a Manual dialog on the user's behalf.

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
URL. Auto status changes orphaned `sending` to `uncertain` under the sender lock;
a live sender keeps `sending`. Status never resends a report. For Manual, retry only a `declined` or `failed` receipt when another attempt is wanted;
a Manual external-project retry still obtains fresh native consent. Auto retry
accepts failed/withheld safe snapshots, rechecks policy and never opens consent.
`withheld` means safe information or authority is unavailable: restore canonical
policy or report a recognized operation instead of adding raw details.

```json
{"schema_version":1,"operation":"feedback.retry","params":{"id":"67e44d8a-7903-4457-9e2d-51703a44c180"}}
```

An `uncertain` result may already have reached GitHub. Preserve it and inspect
the existing receipt/Issue; never automatically resend it or submit a new
request to evade the retry refusal. Report the feedback result separately from
the project's actual blocker, and use the Board for local coordination when
needed.
