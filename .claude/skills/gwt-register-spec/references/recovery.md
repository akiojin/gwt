# Recovery from partial registration

`gwt-register-spec` is designed so the only way to produce a half-baked
SPEC Issue is for `gwtd issue spec create` to succeed and the subsequent
`gwtd issue spec <n> --edit spec -f <body>` to fail. (Validation runs
before any GitHub API call, so a `Structural` or `Format` failure never
creates an orphan Issue.)

This document covers that single failure mode and the matching recovery
flow.

## Detect

After `gwtd issue spec create` returns the new Issue number, the skill
immediately calls `gwtd issue spec <n> --edit spec -f <body>`. If `--edit`
exits non-zero, the skill stops the lifecycle with:

```
gwtd register abort --spec <n> --reason 'edit failed: <gh/gwtd error verbatim>'
```

If the skill reaches the roundtrip step and `gwtd issue spec <n>
--section spec` reports empty output or the heading does not contain the
expected H1, the skill aborts with `--reason 'roundtrip empty'`.

In both cases the GitHub Issue still exists with an empty spec section.
The caller and any human reviewer must see the orphan Issue number.

## Report

The skill returns a structured report to the caller containing at
minimum:

```json
{
  "outcome": "partial",
  "issue": <n>,
  "url": "https://github.com/akiojin/gwt/issues/<n>",
  "failed_step": "edit" | "roundtrip",
  "error": "<verbatim gh/gwtd error>",
  "next_action": "manual_edit"
}
```

The caller surfaces this back to the user. Do not silently retry — the
underlying failure (network, auth, GitHub rate limit) usually persists
and would block the retry too.

## Repair (manual)

Once the upstream failure is understood (e.g. network restored, auth
refreshed), the human or follow-up agent runs:

```
gwtd issue spec <n> --edit spec -f <body_path>
gwtd issue spec <n> --section spec | head -5      # roundtrip verify
```

If the roundtrip is OK, the SPEC is healed. There is no need to delete
and re-create the Issue.

## Repair (automated retry, deferred)

In v1 the skill does not auto-retry. Adding bounded retry with
exponential backoff is a v2 candidate; the open design questions are:

- How many retries before the orphan Issue is reported?
- Should the skill detect transient errors (HTTP 5xx, network) and
  retry only those, vs hard-fail on auth?
- Where does the retry budget live — per-skill-run, per-process, or
  per-user-session?

These will be revisited if recovery ends up being a frequent operation.
Until then, the manual flow is intentional: it forces a human to see
the orphan Issue, which is the safest outcome.

## When the Issue should be abandoned, not repaired

If the SPEC body itself turned out to be wrong (e.g. duplicate of an
existing SPEC found during a second-pass search), close the GitHub Issue
with a short comment pointing at the canonical owner. Do not edit the
spec section to point elsewhere — that pollutes the SPEC search index.

```
gwtd issue comment <n> -f <comment.md>      # explain why
# then close the Issue via the UI or `gh issue close <n>` (out of band)
```

This is the only case where the orphan Issue is intentionally left
empty.
